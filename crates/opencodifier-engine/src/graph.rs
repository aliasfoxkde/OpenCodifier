//! The declarative decision graph: a serializable DAG executed
//! topologically (PLANNING.md §9, §10).
//!
//! Graphs are *data*, not code: there is no embedded scripting language.
//! A graph declares nodes, their kinds, and their dependencies; the
//! executor derives topological waves, runs independent nodes in parallel,
//! and records one trace entry per node.
//!
//! # V1 node set
//!
//! `normalize`, `rule`, `cache`, `filter`, `lexical`, `choice`, `boolean`,
//! `score`, `threshold`, `branch`, `output`.
//!
//! PLANNING.md §9 also lists `embedding`, `classify`, `rerank`, `retrieve`,
//! `verify`, `fuse`, and `transform`. Those arrive with the ML runtime
//! crate (Phases 6+); they are deliberately absent here because this crate
//! has no ML dependency. `verify` in particular is not a node: verification
//! is handled by the engine's verifier cascade
//! ([`crate::classifier::Classifier`] as a verifier), because
//! PLANNING.md §19 requires verification to run *only* when the confidence
//! gate says so, never on every request.
//!
//! # Shape
//!
//! A graph has exactly one entry node (a node with no dependencies —
//! conventionally `normalize`), exactly one terminal `output` node, and
//! every other node is reachable from the entry. Graphs must be acyclic
//! and are executed in topological waves.
//!
//! # Validation
//!
//! [`DecisionGraph::new`] rejects: duplicate ids, unknown or repeated
//! dependencies, cycles, multiple entry nodes, unreachable nodes, missing
//! or multiple `output` nodes, a non-terminal `output`, and nodes whose
//! fields contradict their kind. Deserialization goes through the same
//! validation, so a graph read from disk is as trustworthy as one built in
//! code.

use opencodifier_core::{Limits, NodeId};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};
use crate::rules::Condition;

/// The set of node kinds the V1 engine can execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Canonicalizes the request into the working state. Conventionally
    /// the entry node.
    Normalize,
    /// Runs the deterministic rule engine (PLANNING.md §11).
    Rule,
    /// Probes the exact-decision cache (PLANNING.md §44).
    Cache,
    /// Narrows candidates deterministically (PLANNING.md §45).
    Filter,
    /// Scores surviving candidates lexically with BM25.
    Lexical,
    /// Decides choice questions.
    Choice,
    /// Decides boolean questions.
    Boolean,
    /// Decides score questions.
    Score,
    /// Applies the confidence gate and verifier cascade (§19, §20).
    Threshold,
    /// Conditional short-circuit: dependents are skipped when `when` is
    /// false.
    Branch,
    /// Terminal node; exactly one per graph.
    Output,
}

impl NodeKind {
    /// The kind's canonical name, as used in traces and wire formats.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normalize => "normalize",
            Self::Rule => "rule",
            Self::Cache => "cache",
            Self::Filter => "filter",
            Self::Lexical => "lexical",
            Self::Choice => "choice",
            Self::Boolean => "boolean",
            Self::Score => "score",
            Self::Threshold => "threshold",
            Self::Branch => "branch",
            Self::Output => "output",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One declared node: a kind, its dependencies, and its optional knobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    /// Unique node id.
    pub id: NodeId,
    /// What the node does.
    pub kind: NodeKind,
    /// Nodes that must complete before this one runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<NodeId>,
    /// Confidence threshold for `threshold` nodes; must be absent
    /// elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Branch condition for `branch` nodes; must be absent elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

impl NodeSpec {
    /// Builds a node from an already-validated id.
    #[must_use]
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        Self { id, kind, depends_on: Vec::new(), threshold: None, when: None }
    }

    /// Builds a node from a raw id string, validating it.
    pub fn build(id: impl Into<String>, kind: NodeKind) -> EngineResult<Self> {
        Ok(Self::new(NodeId::new(id)?, kind))
    }

    /// Declares dependencies, consuming and returning `self`.
    #[must_use]
    pub fn with_dependencies(mut self, dependencies: Vec<NodeId>) -> Self {
        self.depends_on = dependencies;
        self
    }

    /// Declares a threshold, consuming and returning `self`.
    #[must_use]
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = Some(threshold);
        self
    }

    /// Declares a branch condition, consuming and returning `self`.
    #[must_use]
    pub fn with_condition(mut self, condition: Condition) -> Self {
        self.when = Some(condition);
        self
    }

    /// Validates that this node's fields agree with its kind.
    pub fn validate(&self) -> EngineResult<()> {
        let invalid = |reason: String| EngineError::InvalidNode {
            kind: self.kind.as_str().to_owned(),
            node: self.id.to_string(),
            reason,
        };
        match self.kind {
            NodeKind::Threshold => {
                let Some(threshold) = self.threshold else {
                    return Err(invalid("threshold nodes must declare a threshold".to_owned()));
                };
                if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
                    return Err(invalid(format!(
                        "threshold must be finite in [0, 1], got {threshold}"
                    )));
                }
            }
            NodeKind::Branch => {
                if self.threshold.is_some() {
                    return Err(invalid("branch nodes must not declare a threshold".to_owned()));
                }
            }
            other => {
                if self.threshold.is_some() || self.when.is_some() {
                    return Err(invalid(format!(
                        "{} nodes take neither a threshold nor a branch condition",
                        other.as_str()
                    )));
                }
            }
        }
        if let Some(condition) = &self.when {
            condition.validate()?;
        }
        Ok(())
    }
}

/// Serialization shim: the wire format is a version plus a node array, and
/// reading one always re-validates.
#[derive(Deserialize)]
struct GraphRepr {
    version: u64,
    nodes: Vec<NodeSpec>,
}

impl TryFrom<GraphRepr> for DecisionGraph {
    type Error = EngineError;

    fn try_from(repr: GraphRepr) -> EngineResult<Self> {
        Self::new(repr.version, repr.nodes)
    }
}

/// A validated, serializable decision graph.
///
/// Nodes are stored sorted by id, so iteration, wave construction, and
/// trace ordering are deterministic regardless of the order the graph was
/// declared in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "GraphRepr")]
pub struct DecisionGraph {
    version: u64,
    nodes: Vec<NodeSpec>,
}

impl DecisionGraph {
    /// Validates and constructs a graph.
    ///
    /// See the module docs for the full list of rejected shapes.
    pub fn new(version: u64, nodes: Vec<NodeSpec>) -> EngineResult<Self> {
        // A graph with no nodes has no output, so the same error applies.
        if nodes.is_empty() {
            return Err(EngineError::MissingOutput);
        }
        for node in &nodes {
            node.validate()?;
        }

        let mut sorted = nodes;
        sorted.sort_by(|left, right| left.id.cmp(&right.id));
        for pair in sorted.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(EngineError::DuplicateNode { node: pair[0].id.to_string() });
            }
        }
        for node in &sorted {
            let mut seen: std::collections::BTreeSet<&NodeId> = std::collections::BTreeSet::new();
            for dependency in &node.depends_on {
                if !seen.insert(dependency) {
                    return Err(EngineError::InvalidNode {
                        kind: node.kind.as_str().to_owned(),
                        node: node.id.to_string(),
                        reason: format!("duplicate dependency `{dependency}`"),
                    });
                }
                if !sorted.iter().any(|candidate| &candidate.id == dependency) {
                    return Err(EngineError::UnknownDependency {
                        node: node.id.to_string(),
                        dependency: dependency.to_string(),
                    });
                }
            }
        }

        // Acyclicity, via Kahn's algorithm; a leftover node sits on a cycle.
        // Waves are recomputed on demand by `waves`; here they only
        // witness acyclicity.
        let (_, unplaced) = Self::waves_for(&sorted);
        if let Some(stuck) = unplaced {
            return Err(EngineError::Cycle { node: stuck });
        }

        // Exactly one entry node, and everything reachable from it.
        let roots: Vec<&NodeSpec> =
            sorted.iter().filter(|node| node.depends_on.is_empty()).collect();
        if roots.len() > 1 {
            return Err(EngineError::MultipleRoots { count: roots.len() });
        }
        let Some(root) = roots.first().copied() else {
            return Err(EngineError::Cycle { node: sorted[0].id.to_string() });
        };
        // Reachability is implied by "one entry plus acyclic", but it is
        // checked rather than assumed: a future node kind with optional
        // entry points would otherwise break the guarantee silently.
        let reached = Self::reachable_from(&sorted, &root.id);
        if reached.len() != sorted.len() {
            let orphan = sorted
                .iter()
                .find(|node| !reached.contains(&node.id))
                .map_or_else(|| root.id.to_string(), |node| node.id.to_string());
            return Err(EngineError::UnreachableNode { node: orphan });
        }

        // Exactly one terminal output, which nothing may depend on.
        let outputs: Vec<&NodeSpec> =
            sorted.iter().filter(|node| node.kind == NodeKind::Output).collect();
        match outputs.len() {
            0 => return Err(EngineError::MissingOutput),
            1 => {}
            count => return Err(EngineError::MultipleOutputs { count }),
        }
        let output = outputs[0];
        if output.depends_on.is_empty() {
            return Err(EngineError::OutputWithoutDependency { node: output.id.to_string() });
        }
        if let Some(dependent) = sorted
            .iter()
            .find(|node| node.depends_on.contains(&output.id))
            .map(|node| node.id.to_string())
        {
            return Err(EngineError::OutputNotTerminal { node: output.id.to_string(), dependent });
        }

        Ok(Self { version, nodes: sorted })
    }

    /// The graph version, folded into every cache key (PLANNING.md §64).
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// All nodes, sorted by id.
    #[must_use]
    pub fn nodes(&self) -> &[NodeSpec] {
        &self.nodes
    }

    /// Looks a node up by id.
    #[must_use]
    pub fn node(&self, id: &NodeId) -> Option<&NodeSpec> {
        self.nodes.binary_search_by(|node| node.id.cmp(id)).ok().map(|index| &self.nodes[index])
    }

    /// The single terminal output node.
    #[must_use]
    pub fn output(&self) -> &NodeSpec {
        // Invariant: `new` guarantees exactly one output node.
        self.nodes
            .iter()
            .find(|node| node.kind == NodeKind::Output)
            .unwrap_or_else(|| &self.nodes[0])
    }

    /// Nodes that depend on `id`, sorted by id.
    #[must_use]
    pub fn dependents_of(&self, id: &NodeId) -> Vec<&NodeSpec> {
        self.nodes.iter().filter(|node| node.depends_on.contains(id)).collect()
    }

    /// Topological waves: wave `i` holds every node whose dependencies all
    /// live in earlier waves. Nodes inside a wave are independent and may
    /// run in parallel (PLANNING.md §10).
    ///
    /// `None` when the graph contains a cycle, which validated graphs
    /// cannot.
    #[must_use]
    pub fn waves(&self) -> Option<Vec<Vec<NodeId>>> {
        let (waves, unplaced) = Self::waves_for(&self.nodes);
        (unplaced.is_none()).then_some(waves)
    }

    /// `true` when the graph declares more nodes than `limit`.
    #[must_use]
    pub fn exceeds(&self, limit: usize) -> bool {
        self.nodes.len() > limit
    }

    /// `true` when the graph fits within `limits.max_graph_nodes`.
    #[must_use]
    pub fn respects(&self, limits: &Limits) -> bool {
        !self.exceeds(limits.max_graph_nodes)
    }

    /// The canonical V1 pipeline.
    ///
    /// ```text
    /// normalize ─┬─ rule ─┬─ boolean ──┐
    ///            │        ├─ score ────┤
    ///            └─ cache ┴─ filter ─ lexical ─ choice ─ threshold ─ output
    /// ```
    ///
    /// `boolean` and `score` do not depend on the choice path, so they
    /// share a wave with `filter` and exercise parallel execution.
    pub fn default_pipeline() -> EngineResult<Self> {
        let spec = |id: &str, kind: NodeKind, deps: &[&str]| -> EngineResult<NodeSpec> {
            let dependencies = deps
                .iter()
                .map(|dependency| NodeId::new(*dependency))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(NodeSpec::build(id, kind)?.with_dependencies(dependencies))
        };
        Self::new(
            1,
            vec![
                spec("normalize", NodeKind::Normalize, &[])?,
                spec("rule", NodeKind::Rule, &["normalize"])?,
                spec("cache", NodeKind::Cache, &["normalize"])?,
                spec("boolean", NodeKind::Boolean, &["rule"])?,
                spec("score", NodeKind::Score, &["rule"])?,
                spec("filter", NodeKind::Filter, &["rule", "cache"])?,
                spec("lexical", NodeKind::Lexical, &["filter"])?,
                spec("choice", NodeKind::Choice, &["lexical"])?,
                spec("threshold", NodeKind::Threshold, &["choice", "boolean", "score"])?
                    .with_threshold(0.80),
                spec("output", NodeKind::Output, &["threshold"])?,
            ],
        )
    }

    /// Wave construction over a node slice.
    ///
    /// Returns the waves plus the id of one node that could not be placed,
    /// which is proof of a cycle.
    fn waves_for(nodes: &[NodeSpec]) -> (Vec<Vec<NodeId>>, Option<String>) {
        let mut remaining: std::collections::BTreeMap<&NodeId, usize> =
            nodes.iter().map(|node| (&node.id, node.depends_on.len())).collect();
        let mut dependents: std::collections::BTreeMap<&NodeId, Vec<&NodeId>> =
            std::collections::BTreeMap::new();
        for node in nodes {
            for dependency in &node.depends_on {
                dependents.entry(dependency).or_default().push(&node.id);
            }
        }
        let mut current: Vec<NodeId> = nodes
            .iter()
            .filter(|node| node.depends_on.is_empty())
            .map(|node| node.id.clone())
            .collect();
        let mut waves: Vec<Vec<NodeId>> = Vec::new();
        while !current.is_empty() {
            let mut next: Vec<NodeId> = Vec::new();
            for id in &current {
                let edges = dependents.get(id).map_or(&[] as &[&NodeId], Vec::as_slice);
                for dependent in edges {
                    if let Some(count) = remaining.get_mut(*dependent) {
                        *count -= 1;
                        if *count == 0 {
                            next.push((*dependent).clone());
                        }
                    }
                }
            }
            next.sort();
            waves.push(std::mem::take(&mut current));
            current = next;
        }
        let placed: usize = waves.iter().map(Vec::len).sum();
        let unplaced = (placed != nodes.len()).then(|| {
            nodes
                .iter()
                .find(|node| remaining.get(&node.id).is_some_and(|count| *count > 0))
                .map_or_else(String::new, |node| node.id.to_string())
        });
        (waves, unplaced)
    }

    /// Ids reachable from `root` by walking dependency edges forward.
    fn reachable_from(nodes: &[NodeSpec], root: &NodeId) -> std::collections::BTreeSet<NodeId> {
        let mut reached: std::collections::BTreeSet<NodeId> = std::collections::BTreeSet::new();
        let mut frontier: Vec<&NodeId> = vec![root];
        while let Some(current) = frontier.pop() {
            if !reached.insert(current.clone()) {
                continue;
            }
            for node in nodes {
                if node.depends_on.contains(current) {
                    frontier.push(&node.id);
                }
            }
        }
        reached
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::FactValue;

    fn node(id: &str, kind: NodeKind, deps: &[&str]) -> NodeSpec {
        let dependencies =
            deps.iter().map(|dependency| NodeId::new(*dependency).unwrap()).collect();
        NodeSpec::build(id, kind).unwrap().with_dependencies(dependencies)
    }

    #[test]
    fn default_pipeline_is_valid_and_well_shaped() {
        let graph = DecisionGraph::default_pipeline().unwrap();
        assert_eq!(graph.version(), 1);
        assert_eq!(graph.nodes().len(), 10);
        assert_eq!(graph.output().id, NodeId::new("output").unwrap());
        assert!(graph.respects(&Limits::default()));
        assert!(!graph.exceeds(Limits::default().max_graph_nodes));

        let waves = graph.waves().unwrap();
        assert_eq!(waves.len(), 7, "{waves:?}");
        assert_eq!(waves[0], vec![NodeId::new("normalize").unwrap()]);
        // Independent nodes share a wave, sorted by id.
        assert_eq!(waves[1], vec![NodeId::new("cache").unwrap(), NodeId::new("rule").unwrap()]);
        assert_eq!(
            waves[2],
            vec![
                NodeId::new("boolean").unwrap(),
                NodeId::new("filter").unwrap(),
                NodeId::new("score").unwrap()
            ]
        );
        assert_eq!(waves.last().unwrap(), &[NodeId::new("output").unwrap()]);
    }

    #[test]
    fn graph_sorts_nodes_by_id_for_deterministic_iteration() {
        let graph = DecisionGraph::new(
            1,
            vec![
                node("output", NodeKind::Output, &["choice"]),
                node("normalize", NodeKind::Normalize, &[]),
                node("choice", NodeKind::Choice, &["normalize"]),
            ],
        )
        .unwrap();
        let ids: Vec<&str> = graph.nodes().iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["choice", "normalize", "output"]);
        assert_eq!(
            graph.node(&NodeId::new("normalize").unwrap()).unwrap().kind,
            NodeKind::Normalize
        );
        assert!(graph.node(&NodeId::new("absent").unwrap()).is_none());
    }

    #[test]
    fn rejects_duplicate_node_ids() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("a", NodeKind::Rule, &[]),
                node("output", NodeKind::Output, &["a"]),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "graph.duplicate_node");
    }

    #[test]
    fn rejects_unknown_dependencies() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &["ghost"]),
                node("output", NodeKind::Output, &["a"]),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "graph.unknown_dependency");
        assert!(error.to_string().contains("ghost"));
    }

    #[test]
    fn rejects_repeated_dependencies() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("b", NodeKind::Rule, &["a", "a"]),
                node("output", NodeKind::Output, &["b"]),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "graph.invalid_node");
        assert!(error.to_string().contains("duplicate dependency `a`"));
    }

    #[test]
    fn rejects_cycles() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("b", NodeKind::Rule, &["a"]),
                node("c", NodeKind::Filter, &["b"]),
                node("d", NodeKind::Lexical, &["a"]),
                node("back", NodeKind::Choice, &["c", "d"]),
                node("output", NodeKind::Output, &["back"]),
                // c -> back -> (nothing points back to a..d) is not a
                // cycle yet; add the back edge that closes the loop.
            ],
        );
        assert!(error.is_ok(), "{error:?}");

        let cyclic = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("b", NodeKind::Rule, &["a", "c"]),
                node("c", NodeKind::Filter, &["b"]),
                node("output", NodeKind::Output, &["c"]),
            ],
        )
        .unwrap_err();
        assert_eq!(cyclic.code(), "graph.cycle");
    }

    #[test]
    fn rejects_self_dependency() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("b", NodeKind::Rule, &["b"]),
                node("output", NodeKind::Output, &["a"]),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "graph.cycle");
    }

    #[test]
    fn rejects_multiple_entry_nodes() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("orphan", NodeKind::Rule, &[]),
                node("output", NodeKind::Output, &["a"]),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "graph.multiple_roots");
    }

    #[test]
    fn a_single_entry_reaches_every_node() {
        // One entry node plus acyclicity implies reachability; the
        // invariant is asserted rather than assumed, because a future node
        // kind with optional entry points would break it silently.
        let graph = DecisionGraph::default_pipeline().unwrap();
        let root = graph
            .nodes()
            .iter()
            .find(|node| node.depends_on.is_empty())
            .map(|node| node.id.clone())
            .unwrap();
        let reached = DecisionGraph::reachable_from(graph.nodes(), &root);
        assert_eq!(reached.len(), graph.nodes().len());
    }

    #[test]
    fn rejects_missing_and_multiple_outputs() {
        let missing = DecisionGraph::new(1, vec![node("a", NodeKind::Normalize, &[])]);
        assert_eq!(missing.unwrap_err().code(), "graph.missing_output");
        assert_eq!(DecisionGraph::new(1, Vec::new()).unwrap_err().code(), "graph.missing_output");

        let duplicated = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("out1", NodeKind::Output, &["a"]),
                node("out2", NodeKind::Output, &["a"]),
            ],
        )
        .unwrap_err();
        assert_eq!(duplicated.code(), "graph.multiple_outputs");
    }

    #[test]
    fn rejects_non_terminal_output() {
        let error = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("out", NodeKind::Output, &["a"]),
                node("after", NodeKind::Rule, &["out"]),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "graph.output_not_terminal");
        assert!(error.to_string().contains("`after`"));
    }

    #[test]
    fn rejects_output_without_dependencies() {
        let error = DecisionGraph::new(1, vec![node("out", NodeKind::Output, &[])]).unwrap_err();
        assert_eq!(error.code(), "graph.output_without_dependency");
    }

    #[test]
    fn rejects_nodes_whose_fields_disagree_with_their_kind() {
        let missing_threshold = DecisionGraph::new(
            1,
            vec![
                node("a", NodeKind::Normalize, &[]),
                node("gate", NodeKind::Threshold, &["a"]),
                node("out", NodeKind::Output, &["gate"]),
            ],
        )
        .unwrap_err();
        assert_eq!(missing_threshold.code(), "graph.invalid_node");

        let out_of_range = node("gate", NodeKind::Threshold, &["a"]).with_threshold(1.5);
        assert_eq!(out_of_range.validate().unwrap_err().code(), "graph.invalid_node");

        let stray = node("a", NodeKind::Normalize, &[]).with_threshold(0.5);
        assert_eq!(stray.validate().unwrap_err().code(), "graph.invalid_node");

        // A branch selects by condition, so a numeric gate on the same node
        // is contradictory rather than merely redundant.
        let gated_branch = node("branch", NodeKind::Branch, &[]).with_threshold(0.5);
        let branch_error = gated_branch.validate().unwrap_err();
        assert_eq!(branch_error.code(), "graph.invalid_node");
        assert!(
            branch_error.to_string().contains("must not declare a threshold"),
            "{branch_error}"
        );

        let condition = crate::rules::Condition::FactExists { fact: "k".into() };
        let stray_condition = node("a", NodeKind::Normalize, &[]).with_condition(condition);
        assert_eq!(stray_condition.validate().unwrap_err().code(), "graph.invalid_node");

        let invalid_condition = node("branch", NodeKind::Branch, &[])
            .with_condition(crate::rules::Condition::FactExists { fact: String::new() });
        assert_eq!(invalid_condition.validate().unwrap_err().code(), "rules.invalid");
    }

    #[test]
    fn branch_nodes_accept_conditions() {
        let branch = node("branch", NodeKind::Branch, &["a"]).with_condition(
            crate::rules::Condition::FactEquals {
                fact: "privacy".into(),
                value: FactValue::Text("local_only".into()),
            },
        );
        assert!(branch.validate().is_ok());
    }

    #[test]
    fn dependents_are_listed_in_id_order() {
        let graph = DecisionGraph::default_pipeline().unwrap();
        let normalize = NodeId::new("normalize").unwrap();
        let dependents: Vec<&str> =
            graph.dependents_of(&normalize).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(dependents, vec!["cache", "rule"]);
    }

    #[test]
    fn graphs_round_trip_through_json_and_revalidate() {
        let graph = DecisionGraph::default_pipeline().unwrap();
        let json = serde_json::to_string_pretty(&graph).unwrap();
        assert!(json.contains(r#""kind": "normalize""#), "{json}");
        let back: DecisionGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(back, graph);
        assert_eq!(back.waves(), graph.waves());
    }

    #[test]
    fn tampered_graphs_are_rejected_on_deserialize() {
        let graph = DecisionGraph::default_pipeline().unwrap();
        let json = serde_json::to_string(&graph).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Give the first node (sorted: `boolean`) a duplicate dependency.
        value["nodes"][0]["depends_on"] = serde_json::json!(["rule", "rule"]);
        let tampered = value.to_string();
        assert!(serde_json::from_str::<DecisionGraph>(&tampered).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Point the output node at itself: a one-node cycle.
        value["nodes"] = serde_json::json!([
            {"id": "output", "kind": "output", "depends_on": ["output"]}
        ]);
        assert!(serde_json::from_str::<DecisionGraph>(&value.to_string()).is_err());
    }

    #[test]
    fn node_kind_names_are_stable() {
        for (kind, name) in [
            (NodeKind::Normalize, "normalize"),
            (NodeKind::Rule, "rule"),
            (NodeKind::Cache, "cache"),
            (NodeKind::Filter, "filter"),
            (NodeKind::Lexical, "lexical"),
            (NodeKind::Choice, "choice"),
            (NodeKind::Boolean, "boolean"),
            (NodeKind::Score, "score"),
            (NodeKind::Threshold, "threshold"),
            (NodeKind::Branch, "branch"),
            (NodeKind::Output, "output"),
        ] {
            assert_eq!(kind.as_str(), name);
            assert_eq!(kind.to_string(), name);
        }
    }
}
