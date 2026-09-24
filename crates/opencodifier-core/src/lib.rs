//! Canonical decision IR for `OpenCodifier`.
//!
//! This crate is the internal truth every external format normalizes into
//! and every engine stage operates on (PLANNING.md §6, §41). It is
//! deliberately dependency-minimal: no HTTP, no MCP, no CLI, no Tokio, no
//! ML runtime. Vendor schemas are adapters; they are never executed
//! directly and never become the internal representation.
//!
//! # Shape of a decision
//!
//! ```text
//! State + DecisionQuestion(s) + DecisionPolicy  ->  DecisionResponse
//! ```
//!
//! Three typed questions are supported — [`ChoiceQuestion`] (dynamic
//! candidate sets), [`BooleanQuestion`], and [`ScoreQuestion`] (ordered
//! levels) — and every answer carries a complete [`Distribution`] plus
//! calibrated confidence. Abstention is a first-class outcome, not an
//! error: [`DecisionOutcome::Abstain`] means the system correctly refused
//! to decide under insufficient confidence.
//!
//! # Invariants worth knowing
//!
//! * IR constructors validate: candidate ids are unique, score levels are
//!   ordered and unique, distributions sum to 1 within `1e-6`, policy
//!   gates are ordered (`abstain_below <= verify_below <= min_confidence`).
//! * Serialization is deterministic: facts and trace details live in
//!   sorted maps, so identical requests serialize byte-identically — this
//!   is what makes exact-decision cache keys possible.
//! * Float probabilities are always finite; `NaN`/`inf` are rejected at
//!   construction.
//!
//! # Example
//!
//! ```
//! use opencodifier_core::{
//!     Candidate, ChoiceQuestion, ConfidenceReport, DecisionAnswer,
//!     DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest,
//!     DecisionResponse, DecisionTrace, DecisionMetrics, RequestMetadata, State,
//! };
//!
//! // 1. Build the canonical request.
//! let state = State::from_text("Refactor this Rust parser and add tests.")
//!     .with_fact("context_tokens", opencodifier_core::FactValue::Integer(42_000));
//! let question = DecisionQuestion::Choice(
//!     ChoiceQuestion::new(
//!         "model",
//!         "Which model should process this request?",
//!         vec![
//!             Candidate::new("local-qwen", "General coding and reasoning").unwrap(),
//!             Candidate::new("local-glm", "Complex reasoning").unwrap(),
//!         ],
//!     )
//!     .unwrap(),
//! );
//! let request = DecisionRequest::new(
//!     state,
//!     vec![question],
//!     DecisionPolicy::default(),
//!     RequestMetadata::default(),
//! )
//! .unwrap();
//!
//! // 2. (An engine would decide here; the IR only carries results.)
//! let distribution = opencodifier_core::Distribution::from_pairs([
//!     ("local-qwen", 0.91),
//!     ("local-glm", 0.09),
//! ])
//! .unwrap();
//! let answer = DecisionAnswer::Choice {
//!     question_id: request.questions()[0].id().clone(),
//!     choice: opencodifier_core::CandidateId::new("local-qwen").unwrap(),
//!     distribution: distribution.clone(),
//!     confidence: 0.91,
//! };
//! let report =
//!     ConfidenceReport::from_distribution(&distribution, 0.91, 0.02, None).unwrap();
//!
//! // 3. The policy cascade classifies the outcome deterministically.
//! let outcome = report.outcome_for(request.policy());
//! assert_eq!(outcome, DecisionOutcome::Accept);
//!
//! let response = DecisionResponse::new(
//!     vec![answer],
//!     outcome,
//!     report,
//!     DecisionTrace::new(),
//!     DecisionMetrics::default(),
//! )
//! .unwrap();
//! assert!(response.outcome().is_decisive());
//! ```

pub mod answer;
pub mod confidence;
pub mod error;
pub mod ids;
pub mod policy;
pub mod question;
pub mod request;
pub mod response;
pub mod state;
pub mod trace;

pub use answer::{DecisionAnswer, Distribution, DistributionEntry};
pub use confidence::ConfidenceReport;
pub use error::{CoreError, CoreResult};
pub use ids::{CandidateId, NodeId, QuestionId};
pub use policy::{DecisionPolicy, Limits, RequestMetadata, RiskLevel};
pub use question::{
    BooleanQuestion, Candidate, ChoiceQuestion, DecisionQuestion, ScoreLevel, ScoreQuestion,
};
pub use request::DecisionRequest;
pub use response::{DecisionOutcome, DecisionResponse};
pub use state::{FactValue, State};
pub use trace::{DecisionMetrics, DecisionTrace, TraceEntry};
