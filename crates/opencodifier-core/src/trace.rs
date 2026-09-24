//! Deterministic execution traces and request metrics.
//!
//! Explainability means execution *facts* — which node ran, how the
//! candidate count changed, what threshold passed — never chain-of-thought
//! (PLANNING.md §65).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::state::FactValue;

/// One deterministic fact about an executed node.
///
/// `detail` is a sorted map so trace serialization is byte-stable, which
/// makes traces diffable and cache-key safe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEntry {
    /// The graph node that produced this entry.
    pub node: String,
    /// Deterministic facts about the node's execution.
    pub detail: BTreeMap<String, FactValue>,
}

impl TraceEntry {
    /// Constructs an entry from `(key, value)` detail pairs.
    pub fn new(
        node: impl Into<String>,
        detail: impl IntoIterator<Item = (&'static str, FactValue)>,
    ) -> Self {
        Self {
            node: node.into(),
            detail: detail.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        }
    }
}

/// Ordered execution trace of a decision run.
///
/// Carries a `trace_version` so persisted traces remain interpretable as
/// the format evolves. Consumers must tolerate unknown fields: trace
/// types deliberately do **not** use `deny_unknown_fields`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionTrace {
    trace_version: u32,
    entries: Vec<TraceEntry>,
}

impl Default for DecisionTrace {
    fn default() -> Self {
        Self { trace_version: Self::VERSION, entries: Vec::new() }
    }
}

impl DecisionTrace {
    /// The current trace format version.
    pub const VERSION: u32 = 1;

    /// An empty trace at the current version.
    pub fn new() -> Self {
        Self::default()
    }

    /// The trace format version of this trace.
    pub fn trace_version(&self) -> u32 {
        self.trace_version
    }

    /// Appends an entry.
    pub fn push(&mut self, entry: TraceEntry) {
        self.entries.push(entry);
    }

    /// The recorded entries in execution order.
    pub fn entries(&self) -> &[TraceEntry] {
        &self.entries
    }

    /// `true` when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of recorded entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl IntoIterator for DecisionTrace {
    type Item = TraceEntry;
    type IntoIter = std::vec::IntoIter<TraceEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

/// System-efficiency facts about one decision run (PLANNING.md §59).
///
/// These are the numbers Amortyx consumes: how hard the narrowing pipeline
/// worked and whether caching or verification engaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DecisionMetrics {
    /// Candidates visible before deterministic filtering.
    pub candidates_in: usize,
    /// Candidates that survived to the decision stage.
    pub candidates_out: usize,
    /// Whether the exact-decision cache satisfied the request.
    pub cache_hit: bool,
    /// Whether the confidence gate routed the request to a verifier.
    pub verification_triggered: bool,
}

impl DecisionMetrics {
    /// The candidate reduction ratio (`candidates_out / candidates_in`).
    ///
    /// Returns 1.0 when no candidates were processed (`candidates_in == 0`).
    #[allow(clippy::cast_precision_loss)]
    pub fn reduction_ratio(&self) -> f64 {
        if self.candidates_in == 0 {
            1.0
        } else {
            self.candidates_out as f64 / self.candidates_in as f64
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn trace_records_in_order_and_serializes_stably() {
        let mut trace = DecisionTrace::new();
        assert_eq!(trace.trace_version(), DecisionTrace::VERSION);
        trace.push(TraceEntry::new(
            "candidate_filter",
            [("before", FactValue::Integer(18)), ("after", FactValue::Integer(7))],
        ));
        trace.push(TraceEntry::new("confidence_gate", [("passed", FactValue::Boolean(true))]));
        assert_eq!(trace.len(), 2);
        assert!(!trace.is_empty());

        let json = serde_json::to_string(&trace).expect("serialize");
        let again = serde_json::to_string(&trace).expect("serialize");
        assert_eq!(json, again, "trace serialization must be byte-stable");
        let back: DecisionTrace = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, trace);
    }

    #[test]
    fn metrics_reduction_ratio_handles_zero() {
        let metrics = DecisionMetrics {
            candidates_in: 50,
            candidates_out: 2,
            cache_hit: false,
            verification_triggered: true,
        };
        assert!((metrics.reduction_ratio() - 0.04).abs() < 1e-9);
        assert!((DecisionMetrics::default().reduction_ratio() - 1.0).abs() < 1e-9);
    }
}
