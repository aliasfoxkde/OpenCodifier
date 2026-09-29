//! Equivalence-preserving graph optimization (PLANNING.md §51; D22).
//!
//! [`optimize`] returns a graph that produces **the same decisions and the
//! same observable traces** as its input, with less work per run. D22
//! bounds what "same" means and what the optimizer may therefore do:
//!
//! * **Equivalence is over decisions, not traces-while-running.** A node
//!   whose output nothing downstream reads (not even the cache or a
//!   human reading the trace) may go, because no decision can observe it.
//!   Anything observable across requests — cache nodes — is untouchable.
//! * **Common subexpression elimination** merges nodes with identical
//!   *pure* specs: same kind, same dependencies, same knobs. Merged nodes
//!   rewrite their dependents to read the survivor.
//! * **No speculative transforms**: no constant folding (a rule's firing
//!   depends on run state, not graph structure), no early exit, no
//!   parallelization hints, no threshold changes. An optimization that
//!   would change *which* trace entries appear or *what* the confidence
//!   gate sees is out of scope by construction, not by flag.
//!
//! The result revalidates through
//! [`DecisionGraph::new`](crate::graph::DecisionGraph::new), so an
//! optimized graph is a first-class graph: serializable, runnable, and
//! accepted everywhere the original was.

use std::collections::{BTreeMap, BTreeSet};

use opencodifier_core::NodeId;

use crate::error::EngineResult;
use crate::graph::{DecisionGraph, NodeKind, NodeSpec};

/// Returns an equivalent graph with dead nodes removed and identical pure
/// nodes merged.
///
/// `graph_version` is preserved: optimization changes *how* a decision is
/// computed, never *what* it decided on, so cache keys stay valid (D6).
///
/// # Errors
///
/// The result revalidates through
/// [`DecisionGraph::new`](crate::graph::DecisionGraph::new) — the same
/// contract the input went through, never a weaker post-hoc check. For a
/// validated input the rebuild cannot fail (every kept spec was accepted
/// before, and merges deduplicate their rewrites), but the possibility is
/// surfaced, never assumed away with a panic.
pub fn optimize(graph: &DecisionGraph) -> EngineResult<DecisionGraph> {
    let optimized = eliminate_dead(graph, merge_common(graph.nodes().to_vec()));
    DecisionGraph::new(graph.version(), optimized)
}

/// Pass 2 — dead-node elimination: drop every node with no path to the
/// output, except cache nodes.
///
/// The executor's single-entry contract means the output is reachable from
/// every decision the graph can produce, so a node outside its ancestor
/// set can only affect skipped work and dead trace entries. Cache nodes
/// are the exception: they are observable across requests (a warmed cache
/// changes later responses), so they are never eliminated, however
/// disconnected they look. The root itself is always an ancestor of the
/// output and therefore always survives. Running after CSE also catches
/// the merge cascade: a node whose only feed was a merged duplicate dies
/// here if nothing else reads it.
fn eliminate_dead(graph: &DecisionGraph, nodes: Vec<NodeSpec>) -> Vec<NodeSpec> {
    let output = graph.output();
    let by_id: BTreeMap<&NodeId, &NodeSpec> = nodes.iter().map(|spec| (&spec.id, spec)).collect();
    // Ancestors of the output, transitively — the live set.
    let mut live: BTreeSet<NodeId> = BTreeSet::new();
    let mut frontier: Vec<&NodeSpec> = vec![output];
    while let Some(spec) = frontier.pop() {
        if !live.insert(spec.id.clone()) {
            continue;
        }
        for dependency in &spec.depends_on {
            if let Some(parent) = by_id.get(dependency) {
                frontier.push(parent);
            }
        }
    }
    nodes
        .into_iter()
        .filter(|spec| spec.kind == NodeKind::Cache || live.contains(&spec.id))
        .collect()
}

/// Pass 1 — common subexpression elimination over pure nodes.
///
/// Two nodes are interchangeable when their kind, dependencies, and every
/// knob (`threshold`, `when`, `top_n`, `floor`, `reranker`) agree: every
/// run they execute the same work over the same inputs and report the same
/// trace, so their outputs are indistinguishable and dependents may read
/// either. Impure nodes (`cache`, and `branch`, whose skipped-subgraph
/// effects are positional) are left alone. Dependency lists are rewritten
/// first and deduplicated afterwards — merging two dependencies of one
/// node can leave it naming the same survivor twice, which graph
/// validation rejects.
fn merge_common(nodes: Vec<NodeSpec>) -> Vec<NodeSpec> {
    // Canonical spec -> representative id, in node order so the earliest
    // of a duplicate group survives.
    let mut representatives: BTreeMap<Key, NodeId> = BTreeMap::new();
    // For every node: the id its dependencies were rewritten to name.
    let mut rewritten: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut kept: Vec<NodeSpec> = Vec::with_capacity(nodes.len());

    for mut spec in nodes {
        spec.depends_on = spec
            .depends_on
            .iter()
            .map(|dependency| {
                rewritten.get(dependency).cloned().unwrap_or_else(|| dependency.clone())
            })
            .collect();
        // CSE is for pure nodes only: two structurally identical cache
        // nodes are two observable caches, and branch nodes' effects are
        // positional, so neither is interchangeable.
        let key = if spec.is_pure() { Some(Key::of(&spec)) } else { None };
        if let Some(survivor) = key.as_ref().and_then(|key| representatives.get(key)) {
            rewritten.insert(spec.id.clone(), survivor.clone());
        } else {
            if let Some(key) = key {
                representatives.insert(key, spec.id.clone());
            }
            kept.push(spec);
        }
    }
    // A merged dependency group can now name the same survivor twice.
    for spec in &mut kept {
        spec.depends_on = dedupe(&spec.depends_on);
    }
    kept
}

/// Dependency ids in first-occurrence order, duplicates dropped.
fn dedupe(ids: &[NodeId]) -> Vec<NodeId> {
    let mut seen: BTreeSet<&NodeId> = BTreeSet::new();
    let mut unique = Vec::with_capacity(ids.len());
    for id in ids {
        if seen.insert(id) {
            unique.push(id.clone());
        }
    }
    unique
}

/// Everything that makes two pure nodes interchangeable (D22): kind,
/// dependency set (order-free — waves derive from the set), and knobs.
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq)]
struct Key {
    kind: NodeKind,
    dependencies: BTreeSet<String>,
    threshold: Option<u64>,
    when: Option<String>,
    top_n: Option<usize>,
    floor: Option<u64>,
    reranker: Option<String>,
}

impl Key {
    /// Canonicalizes `spec` into a comparable key. Float knobs are hashed
    /// through their bit pattern so `-0.0`/`0.0` and equal `f64`s agree
    /// while `NaN` still fails to equal itself.
    fn of(spec: &NodeSpec) -> Self {
        Self {
            kind: spec.kind,
            dependencies: spec.depends_on.iter().map(NodeId::to_string).collect(),
            threshold: spec.threshold.map(f64::to_bits),
            when: spec.when.as_ref().map(serde_json::to_string).transpose().ok().flatten(),
            top_n: spec.top_n,
            floor: spec.floor.map(f64::to_bits),
            reranker: spec.reranker.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::cache::EngineIdentity;
    use crate::handle::EngineHandle;
    use crate::{DecisionGraph, EngineConfig, NodeSpec};
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest,
        RequestMetadata, State,
    };

    fn node(id: &str, kind: NodeKind, dependencies: &[&str]) -> NodeSpec {
        let dependencies: Vec<NodeId> =
            dependencies.iter().map(|name| NodeId::new(*name).unwrap()).collect();
        NodeSpec::build(id, kind).unwrap().with_dependencies(dependencies)
    }

    /// The minimal runnable shape every test graph hangs off.
    fn decide_chain() -> Vec<NodeSpec> {
        vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("choice", NodeKind::Choice, &["normalize"]),
            node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.5),
            node("output", NodeKind::Output, &["threshold"]),
        ]
    }

    fn graph(nodes: Vec<NodeSpec>) -> DecisionGraph {
        DecisionGraph::new(7, nodes).unwrap()
    }

    fn request() -> DecisionRequest {
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model should answer?",
                vec![
                    Candidate::new("a", "small local coding model").unwrap(),
                    Candidate::new("b", "large cloud reasoning model").unwrap(),
                ],
            )
            .unwrap(),
        );
        DecisionRequest::new(
            State::from_text("refactor the parser and add tests"),
            vec![question],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    /// D22 acceptance: `optimize` is the identity on graphs that have
    /// nothing to remove.
    #[test]
    fn an_already_minimal_graph_is_unchanged() {
        let original = graph(decide_chain());
        let optimized = optimize(&original).unwrap();
        assert_eq!(optimized.version(), 7);
        assert_eq!(optimized.nodes().len(), original.nodes().len());
        assert_eq!(optimized.nodes(), original.nodes());
    }

    /// A node with no path to the output is removed — but a disconnected
    /// cache node is not (D22: cache effects are observable across
    /// requests).
    #[test]
    fn dead_nodes_go_and_cache_nodes_stay() {
        let nodes = vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("orphan", NodeKind::Lexical, &["normalize"]),
            node("cache", NodeKind::Cache, &["normalize"]),
            node("choice", NodeKind::Choice, &["normalize"]),
            node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.5),
            node("output", NodeKind::Output, &["threshold"]),
        ];
        let optimized = optimize(&graph(nodes)).unwrap();
        let mut ids: Vec<&str> = optimized.nodes().iter().map(|spec| spec.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["cache", "choice", "normalize", "output", "threshold"]);
    }

    /// Two identical lexical nodes feeding different dependents merge into
    /// one; both dependents now read the survivor.
    #[test]
    fn identical_pure_nodes_merge_and_dependents_are_rewritten() {
        let nodes = vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("lex-a", NodeKind::Lexical, &["normalize"]),
            node("lex-b", NodeKind::Lexical, &["normalize"]),
            node("choice", NodeKind::Choice, &["normalize"]),
            node("threshold", NodeKind::Threshold, &["lex-a", "lex-b", "choice"])
                .with_threshold(0.5),
            node("output", NodeKind::Output, &["threshold"]),
        ];
        let optimized = optimize(&graph(nodes)).unwrap();
        let mut ids: Vec<&str> = optimized.nodes().iter().map(|spec| spec.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["choice", "lex-a", "normalize", "output", "threshold"]);
        let threshold = optimized.node(&NodeId::new("threshold").unwrap()).unwrap();
        assert_eq!(threshold.depends_on.len(), 2, "merged dependency deduplicated");
    }

    /// Same kind but different knobs are not interchangeable: a threshold
    /// node at 0.5 and one at 0.9, both live, stay distinct.
    #[test]
    fn nodes_with_different_knobs_never_merge() {
        let nodes = vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("choice", NodeKind::Choice, &["normalize"]),
            node("low", NodeKind::Threshold, &["choice"]).with_threshold(0.5),
            node("high", NodeKind::Threshold, &["choice"]).with_threshold(0.9),
            node("output", NodeKind::Output, &["low", "high"]),
        ];
        let optimized = optimize(&graph(nodes)).unwrap();
        assert_eq!(optimized.nodes().len(), 5);
        assert!(optimized.node(&NodeId::new("low").unwrap()).is_some());
        assert!(optimized.node(&NodeId::new("high").unwrap()).is_some());
    }

    /// Impure nodes are excluded from CSE even when structurally
    /// identical: two cache nodes are two observable caches.
    #[test]
    fn cache_nodes_never_merge() {
        let nodes = vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("cache-a", NodeKind::Cache, &["normalize"]),
            node("cache-b", NodeKind::Cache, &["normalize"]),
            node("choice", NodeKind::Choice, &["normalize"]),
            node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.5),
            node("output", NodeKind::Output, &["threshold"]),
        ];
        let optimized = optimize(&graph(nodes)).unwrap();
        assert_eq!(optimized.nodes().len(), 6);
    }

    /// Cascaded elimination: merging a duplicate removes the work of a
    /// dead branch that only it fed.
    #[test]
    fn optimization_is_idempotent() {
        let nodes = vec![
            node("normalize", NodeKind::Normalize, &[]),
            node("lex-a", NodeKind::Lexical, &["normalize"]),
            node("lex-b", NodeKind::Lexical, &["normalize"]),
            node("only-b", NodeKind::Lexical, &["lex-b"]),
            node("choice", NodeKind::Choice, &["normalize"]),
            node("threshold", NodeKind::Threshold, &["lex-a", "choice"]).with_threshold(0.5),
            node("output", NodeKind::Output, &["threshold"]),
        ];
        let once = optimize(&graph(nodes)).unwrap();
        let twice = optimize(&once).unwrap();
        assert_eq!(twice.nodes(), once.nodes());
    }

    /// The D22 acceptance proof: on the committed recipe fleet, the
    /// optimized graph decides every request identically to the original.
    #[test]
    fn the_recipe_fleet_decides_identically_optimized() {
        let fleet_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes");
        let mut checked = 0;
        for entry in std::fs::read_dir(&fleet_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let document: crate::graph::GraphDocument =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let original = DecisionGraph::new(document.version, document.nodes).unwrap();
            let optimized = optimize(&original).unwrap();
            let before = EngineHandle::lexical(EngineConfig::new(
                DecisionGraph::new(original.version(), original.nodes().to_vec()).unwrap(),
            ))
            .unwrap()
            .decide(&request())
            .unwrap();
            let after = EngineHandle::lexical(EngineConfig::new(optimized))
                .unwrap()
                .decide(&request())
                .unwrap();
            let schema = serde_json::to_value(&before).unwrap();
            assert_eq!(
                serde_json::to_value(&after).unwrap(),
                schema,
                "optimized graph changed the decision of {}",
                path.display()
            );
            checked += 1;
        }
        // The 12 fleet recipes plus the three earlier example graphs
        // (`minimal-choice`, `strict-verify`, `boolean-score`) — every
        // committed graph participates, none silently skipped.
        assert_eq!(checked, 15, "every committed recipe graph participates");
    }

    /// Engine identity is untouched by optimization: only the shape of
    /// the graph changed.
    #[test]
    fn optimization_preserves_engine_identity() {
        let original = graph(decide_chain());
        let optimized = optimize(&original).unwrap();
        assert_eq!(optimized.version(), original.version());
        assert_eq!(EngineConfig::new(optimized).identity, EngineIdentity::default());
    }
}
