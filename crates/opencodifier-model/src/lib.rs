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
//!                       ◄── LlamaDecisionClassifier (llama.cpp fork, D26)
//!                       ◄── kai contract + KaiOnnxBackend (task #92)
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
//! * [`LlamaDecisionClassifier`] — the model rung of the escalation
//!   ladder (D26): the measured tree-mode contract of the
//!   `parallel-decision` llama.cpp fork as a
//!   [`Classifier`](opencodifier_engine::Classifier), full distribution
//!   required, math renormalized in Rust. The HTTP client lives behind
//!   the `llamacpp` feature (see the [`llamacpp`] module); the
//!   classifier itself is transport-agnostic and always compiled.
//! * The [`kai`] contract module — the Kai-0.6B-ONNX decision rung
//!   (task #92): exact prompt rendering, pointer computation with
//!   in-pipeline assertions, the fitted score bias, and `f64` softmax,
//!   all feature-free and verified against a frozen upstream parity
//!   fixture. The ONNX transport itself lives in
//!   `opencodifier_runtime::kai`, behind the `onnx` feature.
//! * The [`julia`] contract module — the Julia-1 ONNX decision rung
//!   (task #92): the upstream `julia.data.sequence` packing contract,
//!   marker computation with in-pipeline assertions, and `f64` softmax
//!   (no fitted bias for this model), feature-free and verified against
//!   a frozen upstream parity fixture. The ONNX transport lives in
//!   `opencodifier_runtime::julia`, behind the `onnx` feature.
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
pub mod julia;
pub mod kai;
pub mod llamacpp;
pub mod manifest;

pub use decision::{CANDIDATES_INPUT, CONTEXT_INPUT, CandidateConditionedModel, LOGITS_OUTPUT};
pub use embedding::EmbeddingClassifier;
pub use error::ModelError;
pub use julia::{
    CLS_ID, HEAD_LENGTH, JULIA_FIXTURE, JULIA_FIXTURE_TOKENIZER, JuliaEncoding, JuliaKind,
    JuliaOption, JuliaQuestion, MASK_ID, MAX_LENGTH, MAX_OPTIONS, MIN_OPTIONS, OPTION_TOKEN_LIMIT,
    PROMPT_VERSION as JULIA_PROMPT_VERSION, SEP_ID,
};
#[cfg(feature = "tokenizers")]
pub use kai::HfTokenizer;
pub use kai::{
    BpeTokenizer, FIXTURE, FIXTURE_TOKENIZER, KaiBatch, KaiEncoding, KaiKind, KaiOption,
    KaiPayload, KaiQuestion, KaiSegments, KaiTokenizer, MAX_CANDIDATES, MAX_INPUT_TOKENS,
    MAX_SCORE_LEVELS, MIN_CANDIDATES, PAD_ID, PROMPT_VERSION, SCORE_BIAS, apply_score_bias,
    canonical_json, probabilities, score_batch, score_bias, softmax,
};
#[cfg(feature = "llamacpp")]
pub use llamacpp::UreqTransport;
pub use llamacpp::{
    DEFAULT_INSTRUCTIONS, DEFAULT_TIMEOUT, LlamaConfig, LlamaDecisionClassifier, LlamaError,
    Transport,
};
pub use manifest::ModelManifest;
