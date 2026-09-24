//! The deterministic decision-graph engine for `OpenCodifier`
//! (PLANNING.md §9, §10, §11, §43, §44, §45).
//!
//! This is Phase 3–5: the machinery that turns a canonical
//! [`DecisionRequest`](opencodifier_core::DecisionRequest) into a traced,
//! cached [`DecisionResponse`](opencodifier_core::DecisionResponse) — with
//! **no ML dependency, no async runtime, and no network**. The base binary
//! is useful with zero model (PLANNING.md §73).
//!
//! # Layout
//!
//! | Module | Implements |
//! |---|---|
//! | [`graph`] | §9, §10 — declarative, serializable decision DAG |
//! | [`executor`] | §10 — topological waves, parallel nodes, timeout, cancellation, traces |
//! | [`rules`] | §11 — deterministic rule engine (JSON wire format) |
//! | [`narrowing`] | §45 — candidate narrowing under safe mode |
//! | [`cache`] | §44, §64 — exact-decision cache, one key construction site |
//! | [`lexical`] | §43 — hand-written BM25 scoring |
//! | [`classifier`] | §13, §43 — `Classifier` trait, `MockClassifier`, `LexicalClassifier` |
//! | [`clock`] | §10 — `Clock`, `Deadline`, `CancellationToken` |
//! | [`engine`] | §43 — configuration and the orchestrating `DecisionEngine` |
//!
//! # The cheap-mechanism-first principle
//!
//! Every request walks the rungs in cost order: exact rule → cached
//! decision → deterministic filter → lexical match → decision model →
//! confidence gate → verifier. A more expensive rung is never invoked when
//! a cheaper one can decide with sufficient confidence, and candidates a
//! cheaper rung removed are never re-examined by a more expensive one.
//!
//! # What is deliberately not here
//!
//! `embedding`, `classify`, `rerank`, `retrieve`, and `fuse` nodes wait for
//! the ML runtime crate (Phases 6+); this crate has no ML dependency.
//! `verify` is not a node either: verification is confidence-gated
//! (PLANNING.md §19) and lives in the engine's verifier cascade.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//!
//! use opencodifier_core::{
//!     Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest,
//!     FactValue, RequestMetadata, State,
//! };
//! use opencodifier_engine::{
//!     Action, Condition, DecisionEngine, EngineConfig, LexicalClassifier, Rule, RuleEngine,
//!     RuleSet, SystemClock,
//! };
//!
//! # fn main() -> Result<(), opencodifier_engine::EngineError> {
//! // 1. Configuration: the built-in pipeline, one rule, four threads.
//! let rules = RuleEngine::new(RuleSet {
//!     rules: vec![Rule {
//!         name: Some("local only".into()),
//!         when: Condition::FactEquals {
//!             fact: "privacy".into(),
//!             value: FactValue::Text("local_only".into()),
//!         },
//!         then: vec![Action::ExcludeCandidate { id: None, tag: Some("cloud".into()) }],
//!     }],
//! })?;
//! let config = EngineConfig::with_default_pipeline()?
//!     .with_rules(Arc::new(rules))
//!     .with_parallelism(4);
//!
//! // 2. The built-in lexical classifier stands in for a model (Phase 6+).
//! let engine = DecisionEngine::new(
//!     config,
//!     Arc::new(SystemClock),
//!     Arc::new(LexicalClassifier::new()),
//!     None,
//! )?;
//!
//! // 3. A canonical request.
//! let state = State::from_text("Refactor the parser and add tests")
//!     .with_fact("privacy", FactValue::Text("local_only".into()));
//! let question = DecisionQuestion::Choice(
//!     ChoiceQuestion::new(
//!         "model",
//!         "Which model should answer?",
//!         vec![
//!             Candidate::new("local-small", "small local coding model")?,
//!             Candidate::new("cloud-large", "long context cloud model")?,
//!         ],
//!     )?,
//! );
//! let request = DecisionRequest::new(
//!     state,
//!     vec![question],
//!     DecisionPolicy::default(),
//!     RequestMetadata::default(),
//! )?;
//!
//! // 4. Decide. The cloud candidate was removed by rule, not by a model.
//! let (response, report) = engine.decide_with_report(&request)?;
//! let narrowed = &report.narrowing()[0];
//! assert_eq!(narrowed.before(), 2);
//! assert_eq!(narrowed.after(), 1);
//! assert!(!response.metrics().cache_hit);
//! assert!(!response.trace().is_empty(), "every node leaves a trace entry");
//! # Ok(())
//! # }
//! ```

pub mod cache;
pub mod classifier;
pub mod clock;
pub mod engine;
pub mod error;
pub mod executor;
pub mod graph;
pub mod lexical;
pub mod narrowing;
pub mod rules;

pub use cache::{CacheConfig, CacheKey, CacheKeyBuilder, DecisionCache, EngineIdentity};
pub use classifier::{Classifier, LexicalClassifier, MockClassifier};
pub use clock::{CancellationToken, Clock, Deadline, ManualClock, SystemClock};
pub use engine::{DecisionEngine, EngineConfig};
pub use error::{EngineError, EngineResult};
pub use executor::RunReport;
pub use graph::{DecisionGraph, NodeKind, NodeSpec};
pub use lexical::{Bm25Index, softmax, tokenize};
pub use narrowing::{LexicalScores, NarrowingOutcome};
pub use rules::{Action, Condition, Rule, RuleEngine, RuleSet};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use std::sync::Arc;

    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest,
        RequestMetadata, State,
    };

    use super::*;

    /// A request with two candidates and no rules: the smallest full run.
    fn request() -> DecisionRequest {
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model should answer?",
                vec![
                    Candidate::new("a", "small local model").unwrap(),
                    Candidate::new("b", "large cloud model").unwrap(),
                ],
            )
            .unwrap(),
        );
        DecisionRequest::new(
            State::from_text("refactor the parser"),
            vec![question],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    #[test]
    fn a_decide_round_trip_works_through_the_public_api() {
        let config = EngineConfig::with_default_pipeline().unwrap().with_parallelism(1);
        let engine = DecisionEngine::new(
            config,
            Arc::new(SystemClock),
            Arc::new(MockClassifier::new("mock/test")),
            None,
        )
        .unwrap();

        let (response, report) = engine.decide_with_report(&request()).unwrap();
        assert_eq!(response.answers().len(), 1);
        assert!(!report.cache_hit());
        assert_eq!(report.waves().len(), 7);
        assert_eq!(report.cache_key().map(|key| key.as_hex().len()), Some(64));

        // A second identical request is served from the cache.
        let (again, report) = engine.decide_with_report(&request()).unwrap();
        assert!(report.cache_hit());
        assert!(again.metrics().cache_hit);
    }
}
