//! Per-node and per-kind confidence-gate policies — the escalation
//! ladder (PLANNING.md §24; the fusion study,
//! `benchmarks/decision-model/results/fusion-suite.md`).
//!
//! One graph may route different questions through different mechanisms
//! (exact rules, semantic narrowing, the decision model), and those
//! mechanisms' confidences are not commensurable: a relational proof is
//! p = 1.0 by construction, an embedding score carries no calibrated
//! probability at all, and the model rung carries a fitted temperature
//! (D15). A single request-level
//! [`DecisionPolicy`] therefore
//! cannot gate them all well. A [`LadderPolicy`] supplies optional
//! overrides — per node id, then per node kind — that the executor
//! consults at the existing confidence gate instead of the request
//! policy. No IR change, no wire change: the ladder is engine-internal
//! composition of an existing, validated type.
//!
//! The default ladder is empty: every question is gated by the request
//! policy exactly as before, traces and cache keys are unchanged, and
//! wire fixtures hold byte-for-byte. Overrides take effect only when a
//! caller configures them; the engine then decorates the model id with
//! `|ladder-v1@<id>` so cached decisions re-key (mirroring focused
//! extraction, D6).
//!
//! This is distinct from the graph's per-node `NodeSpec::threshold`
//! knob: that scalar is declared on the graph (and folded into node
//! fingerprinting) but is not consumed by the executor's gate; the
//! ladder is engine-configuration-level and carries a full policy.

use std::collections::BTreeMap;

use opencodifier_core::DecisionPolicy;

use crate::graph::NodeKind;

/// Optional per-node / per-kind confidence-gate policy overrides.
///
/// Resolution order for a question decided by node `n` of kind `k`:
/// `per_node[n]`, then `per_kind[k]`, then the request policy.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LadderPolicy {
    /// Ladder identity, folded into the cache key as `|ladder-v1@<id>`
    /// when the ladder is non-empty — changing the id re-keys every
    /// cached decision, exactly like a model swap. `"none"` is the empty
    /// default.
    pub id: String,
    /// Policy per deciding node id; wins over [`Self::per_kind`]. Keys
    /// are node ids as they appear in the graph.
    pub per_node: BTreeMap<String, DecisionPolicy>,
    /// Policy per deciding node kind, used when the deciding node has no
    /// [`Self::per_node`] entry.
    pub per_kind: BTreeMap<NodeKind, DecisionPolicy>,
}

impl LadderPolicy {
    /// An empty ladder named `id` (`"none"` is the conventional name for
    /// the empty default).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), ..Self::default() }
    }

    /// `true` when no overrides are configured — the engine then runs
    /// byte-identically to a single-policy engine.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.per_node.is_empty() && self.per_kind.is_empty()
    }

    /// The override for a question decided by node id `node_id` of kind
    /// `kind`, most specific first, paired with the source to record in
    /// the trace (`node:<id>` / `kind:<name>`). `None` when the ladder
    /// has no override for this node — the request policy governs.
    #[must_use]
    pub fn resolve(&self, node_id: &str, kind: NodeKind) -> Option<(&DecisionPolicy, String)> {
        if let Some(policy) = self.per_node.get(node_id) {
            return Some((policy, format!("node:{node_id}")));
        }
        if let Some(policy) = self.per_kind.get(&kind) {
            return Some((policy, format!("kind:{}", kind.as_str())));
        }
        None
    }

    /// Checks the ladder is internally consistent: a non-empty ladder
    /// must carry a real identity, because that identity is the only
    /// thing separating its cache keys from single-policy decisions
    /// (PLANNING.md §64 — all cache keys include the policy identity).
    ///
    /// # Errors
    ///
    /// A non-empty ladder whose `id` is empty or `"none"`.
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_empty() && (self.id.is_empty() || self.id == "none") {
            return Err(format!(
                "a non-empty ladder needs a distinct `id` (got {:?}); the id is the \
                 cache-key discriminator for ladder-gated decisions",
                self.id
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::RiskLevel;

    fn policy(min_confidence: f64) -> DecisionPolicy {
        DecisionPolicy::new(
            min_confidence,
            min_confidence - 0.1,
            min_confidence / 2.0,
            RiskLevel::Low,
        )
        .unwrap()
    }

    #[test]
    fn empty_ladder_resolves_nothing() {
        let ladder = LadderPolicy::new("none");
        assert!(ladder.is_empty());
        assert_eq!(ladder.resolve("decide", NodeKind::Choice), None);
        ladder.validate().unwrap();
    }

    #[test]
    fn per_kind_override_resolves_with_kind_source() {
        let ladder = LadderPolicy {
            id: "test-v1".into(),
            per_kind: BTreeMap::from([(NodeKind::Choice, policy(0.9))]),
            ..LadderPolicy::default()
        };
        let (resolved, source) = ladder.resolve("q_decide", NodeKind::Choice).unwrap();
        assert_eq!(resolved.min_confidence(), 0.9);
        assert_eq!(source, "kind:choice");
        // A different kind is untouched.
        assert_eq!(ladder.resolve("q_decide", NodeKind::Boolean), None);
    }

    #[test]
    fn per_node_beats_per_kind() {
        let ladder = LadderPolicy {
            id: "test-v1".into(),
            per_node: BTreeMap::from([("decide_choice".into(), policy(0.7))]),
            per_kind: BTreeMap::from([(NodeKind::Choice, policy(0.95))]),
        };
        let (resolved, source) = ladder.resolve("decide_choice", NodeKind::Choice).unwrap();
        assert_eq!(resolved.min_confidence(), 0.7);
        assert_eq!(source, "node:decide_choice");
        // Another choice node falls through to the kind entry.
        let (resolved, source) = ladder.resolve("decide_other", NodeKind::Choice).unwrap();
        assert_eq!(resolved.min_confidence(), 0.95);
        assert_eq!(source, "kind:choice");
    }

    #[test]
    fn non_empty_ladder_requires_identity() {
        let ladder = LadderPolicy {
            per_kind: BTreeMap::from([(NodeKind::Boolean, policy(0.9))]),
            ..LadderPolicy::new("none")
        };
        assert!(ladder.validate().is_err());
        let unnamed = LadderPolicy { per_kind: ladder.per_kind.clone(), ..LadderPolicy::new("") };
        assert!(unnamed.validate().is_err());
    }

    #[test]
    fn empty_ladder_accepts_the_none_identity() {
        assert!(LadderPolicy::new("none").validate().is_ok());
    }
}
