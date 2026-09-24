//! Error type for graph validation, rule evaluation, caching, and decision
//! execution (PLANNING.md §10, §43–§45).
//!
//! Like [`opencodifier_core::CoreError`], every variant carries a stable
//! machine-readable [`EngineError::code`]. Codes are part of the public
//! contract: new codes may appear, existing strings never change meaning.
//! They are what HTTP handlers and MCP payloads map onto, so an engine
//! failure is diagnosable without pattern-matching on prose.
//!
//! [`EngineError`] wraps [`opencodifier_core::CoreError`] via `#[from]`, so
//! IR validation failures raised by core constructors propagate unchanged
//! and keep their own codes.

use opencodifier_core::CoreError;

/// Everything that can go wrong inside the decision engine.
///
/// Deliberately not `Eq`: [`EngineError::Core`] wraps `CoreError`, which
/// carries floating-point payload values, so only `PartialEq` is honest.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    /// The request ran past its deadline. Deadline checks happen at node
    /// boundaries, so the reported bound is the last observed node edge,
    /// not the exact instant of overrun.
    #[error("execution deadline exceeded after {elapsed_ms} ms (limit {limit_ms} ms)")]
    Timeout {
        /// Wall-clock milliseconds actually consumed when the deadline was
        /// observed to have expired.
        elapsed_ms: u64,
        /// The configured/requested limit in milliseconds.
        limit_ms: u64,
    },

    /// Execution was cancelled through a [`CancellationToken`](crate::clock::CancellationToken).
    #[error("execution cancelled")]
    Cancelled,

    /// A node's dependency list contains a cycle, so no topological order
    /// exists.
    #[error("graph contains a cycle through node `{node}`")]
    Cycle {
        /// A node involved in the cycle.
        node: String,
    },

    /// A node depends on an id that is not part of the graph.
    #[error("node `{node}` depends on unknown node `{dependency}`")]
    UnknownDependency {
        /// The node declaring the dependency.
        node: String,
        /// The missing dependency.
        dependency: String,
    },

    /// The same node id was declared twice.
    #[error("duplicate node id `{node}`")]
    DuplicateNode {
        /// The repeated node id.
        node: String,
    },

    /// The graph declares no `output` node.
    #[error("graph must declare exactly one output node, found 0")]
    MissingOutput,

    /// The graph declares more than one `output` node.
    #[error("graph must declare exactly one output node, found {count}")]
    MultipleOutputs {
        /// How many `output` nodes were declared.
        count: usize,
    },

    /// A node is not reachable from the graph's entry node, so it would
    /// never execute.
    #[error("node `{node}` is unreachable from the entry node")]
    UnreachableNode {
        /// The orphaned node id.
        node: String,
    },

    /// The graph declares more than one entry node (more than one node
    /// with no dependencies). A decision graph has a single entry —
    /// conventionally `normalize` — that every other node depends on,
    /// directly or transitively.
    #[error("graph must have exactly one entry node, found {count}")]
    MultipleRoots {
        /// How many dependency-free nodes were declared.
        count: usize,
    },

    /// Some node depends on the `output` node, so the output is not the end
    /// of the graph.
    #[error("node `{dependent}` depends on the output node `{node}`; output must be terminal")]
    OutputNotTerminal {
        /// The `output` node id.
        node: String,
        /// The node that depends on it.
        dependent: String,
    },

    /// The `output` node declares no dependencies, so the graph would
    /// produce a result from nothing.
    #[error("output node `{node}` must depend on at least one other node")]
    OutputWithoutDependency {
        /// The `output` node id.
        node: String,
    },

    /// The graph exceeds the node limit configured for the request.
    #[error("graph declares {nodes} nodes, limit is {limit}")]
    GraphTooLarge {
        /// Number of declared nodes.
        nodes: usize,
        /// The configured ceiling.
        limit: usize,
    },

    /// A node's fields do not match its kind (a threshold without a value,
    /// a branch condition on a non-branch node, ...).
    #[error("invalid {kind} node `{node}`: {reason}")]
    InvalidNode {
        /// The node kind the spec declared.
        kind: String,
        /// The node id.
        node: String,
        /// Why the spec is incoherent.
        reason: String,
    },

    /// A rule or rule set is structurally invalid (empty fact name, no
    /// actions, an `exclude_candidate` with neither id nor tag, ...).
    #[error("invalid rule: {reason}")]
    InvalidRule {
        /// Why the rule was rejected.
        reason: String,
    },

    /// A cache configuration is invalid (zero-capacity LRU, zero TTL, ...).
    ///
    /// The code is `cache.miss_configured`, matching the engine's published
    /// code list.
    #[error("invalid cache configuration: {reason}")]
    CacheMisconfigured {
        /// Why the configuration was rejected.
        reason: String,
    },

    /// A classifier returned a distribution the engine could not use (no
    /// key overlapping the surviving candidate set, non-finite mass, ...).
    #[error("classifier returned an unusable distribution for question `{question}`: {reason}")]
    InvalidDistribution {
        /// The question id the distribution was returned for.
        question: String,
        /// Why the distribution was rejected.
        reason: String,
    },

    /// A request carries two questions with the same id. Core does not
    /// forbid this; the engine does, because per-question narrowing and
    /// trace entries are keyed by question id.
    #[error("request contains duplicate question id `{question}`")]
    DuplicateQuestion {
        /// The repeated question id.
        question: String,
    },

    /// Engine-level configuration is incoherent (zero parallelism, a
    /// lexical prune limit of zero, ...).
    #[error("invalid engine configuration: {reason}")]
    InvalidConfig {
        /// Why the configuration was rejected.
        reason: String,
    },

    /// Canonical serialization of a request for cache-key construction
    /// failed.
    #[error("failed to serialize canonical request: {reason}")]
    Serialization {
        /// The underlying serde error message.
        reason: String,
    },

    /// A graph node failed while executing.
    #[error("node `{node}` failed: {reason}")]
    NodeFailed {
        /// The node id that failed.
        node: String,
        /// What went wrong inside the node.
        reason: String,
    },

    /// A core IR error (invalid request, invalid distribution, ...).
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl EngineError {
    /// Stable machine-readable code for this error.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "engine.timeout",
            Self::Cancelled => "engine.cancelled",
            Self::Cycle { .. } => "graph.cycle",
            Self::UnknownDependency { .. } => "graph.unknown_dependency",
            Self::DuplicateNode { .. } => "graph.duplicate_node",
            Self::MissingOutput => "graph.missing_output",
            Self::MultipleOutputs { .. } => "graph.multiple_outputs",
            Self::UnreachableNode { .. } => "graph.unreachable_node",
            Self::MultipleRoots { .. } => "graph.multiple_roots",
            Self::OutputNotTerminal { .. } => "graph.output_not_terminal",
            Self::OutputWithoutDependency { .. } => "graph.output_without_dependency",
            Self::GraphTooLarge { .. } => "graph.limit_exceeded",
            Self::InvalidNode { .. } => "graph.invalid_node",
            Self::InvalidRule { .. } => "rules.invalid",
            Self::CacheMisconfigured { .. } => "cache.miss_configured",
            Self::InvalidDistribution { .. } => "engine.invalid_distribution",
            Self::DuplicateQuestion { .. } => "engine.duplicate_question",
            Self::InvalidConfig { .. } => "engine.invalid_config",
            Self::Serialization { .. } => "engine.serialization",
            Self::NodeFailed { .. } => "engine.node_failed",
            Self::Core(_) => "ir.invalid",
        }
    }
}

/// Alias used throughout the crate for fallible engine operations.
pub type EngineResult<T> = Result<T, EngineError>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn codes_are_stable_and_distinct() {
        let errors = [
            EngineError::Timeout { elapsed_ms: 1, limit_ms: 2 },
            EngineError::Cancelled,
            EngineError::Cycle { node: "a".into() },
            EngineError::UnknownDependency { node: "a".into(), dependency: "b".into() },
            EngineError::DuplicateNode { node: "a".into() },
            EngineError::MissingOutput,
            EngineError::MultipleOutputs { count: 2 },
            EngineError::UnreachableNode { node: "a".into() },
            EngineError::MultipleRoots { count: 2 },
            EngineError::OutputNotTerminal { node: "out".into(), dependent: "x".into() },
            EngineError::OutputWithoutDependency { node: "out".into() },
            EngineError::GraphTooLarge { nodes: 9, limit: 8 },
            EngineError::InvalidNode {
                kind: "threshold".into(),
                node: "t".into(),
                reason: "r".into(),
            },
            EngineError::InvalidRule { reason: "r".into() },
            EngineError::CacheMisconfigured { reason: "r".into() },
            EngineError::InvalidDistribution { question: "q".into(), reason: "r".into() },
            EngineError::DuplicateQuestion { question: "q".into() },
            EngineError::InvalidConfig { reason: "r".into() },
            EngineError::Serialization { reason: "r".into() },
            EngineError::NodeFailed { node: "n".into(), reason: "r".into() },
            EngineError::Core(CoreError::EmptyQuestions),
        ];

        let mut seen = std::collections::BTreeSet::new();
        for error in &errors {
            assert!(seen.insert(error.code()), "duplicate code {}", error.code());
        }
        assert_eq!(errors[0].code(), "engine.timeout");
        assert_eq!(errors[1].code(), "engine.cancelled");
        assert_eq!(errors[2].code(), "graph.cycle");
        assert_eq!(errors[12].code(), "graph.invalid_node");
        assert_eq!(errors[13].code(), "rules.invalid");
        assert_eq!(errors[14].code(), "cache.miss_configured");
    }

    #[test]
    fn core_errors_convert_and_keep_their_message() {
        let error = EngineError::from(CoreError::EmptyQuestions);
        assert_eq!(error.code(), "ir.invalid");
        assert!(error.to_string().contains("at least one question"));
    }

    #[test]
    fn timeout_message_reports_both_bounds() {
        let error = EngineError::Timeout { elapsed_ms: 12, limit_ms: 10 };
        assert_eq!(error.to_string(), "execution deadline exceeded after 12 ms (limit 10 ms)");
    }
}
