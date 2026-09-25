//! The model layer of `OpenCodifier` (PLANNING.md Phase 7).
//!
//! This crate bridges the two neighbors in the dependency ladder: the
//! engine's [`Classifier`](opencodifier_engine::Classifier) seam below and
//! the runtime's backend traits
//! above. It contains the "embedding similarity" rung of the escalation
//! ladder, the identity documents for model artifacts, and the
//! candidate-conditioned serving contract.
//!
//! ```text
//!   engine::Classifier  ◄── EmbeddingClassifier (any EmbeddingBackend)
//!                       ◄── CandidateConditionedModel (any InferenceBackend)
//!   model::ModelManifest ── pins artifact bytes (SHA-256, D14)
//!   runtime::InferenceBackend / EmbeddingBackend
//! ```
//!
//! # What is real here, honestly
//!
//! * [`EmbeddingClassifier`] — a complete, deterministic semantic scorer:
//!   embed, cosine, stable softmax. Its quality is its backend's quality.
//! * [`ModelManifest`] — digest-verified model artifacts; a mismatched
//!   artifact never loads (D14).
//! * [`CandidateConditionedModel`] — the serving contract for the future
//!   decision model: shape-checked logits, Rust-side softmax (D7). **No
//!   weights ship in V1**; without a trained artifact this is exercised
//!   by tests and stays out of every default pipeline.
//!
//! # Example
//!
//! Decide a choice question with a deterministic mock embedder — the
//! same code path a real ONNX encoder plugs into:
//!
//! ```
//! use opencodifier_core::{Candidate, ChoiceQuestion, DecisionQuestion, State};
//! use opencodifier_engine::Classifier;
//! use opencodifier_model::EmbeddingClassifier;
//! use opencodifier_runtime::MockEmbeddingBackend;
//!
//! let classifier = EmbeddingClassifier::new(Box::new(
//!     MockEmbeddingBackend::new("mock-embed-v1", 128)?,
//! ));
//! let question = ChoiceQuestion::new(
//!     "model",
//!     "Which model should run deep reasoning work?",
//!     vec![
//!         Candidate::new("local-qwen", "fast general coding")?,
//!         Candidate::new("local-glm", "deep reasoning specialist")?,
//!     ],
//! )?;
//! let distribution = classifier.decide(
//!     &State::from_text("prove a large theorem"),
//!     &DecisionQuestion::Choice(question),
//! )?;
//! assert!(distribution.probability_of("local-glm").unwrap() > 0.5);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod decision;
pub mod embedding;
pub mod error;
pub mod manifest;

pub use decision::{CANDIDATES_INPUT, CONTEXT_INPUT, CandidateConditionedModel, LOGITS_OUTPUT};
pub use embedding::EmbeddingClassifier;
pub use error::ModelError;
pub use manifest::ModelManifest;
