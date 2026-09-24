//! The inference boundary of `OpenCodifier` (PLANNING.md Phase 6).
//!
//! This crate is the *only* place a numerical model is reached through.
//! It defines two narrow, synchronous, object-safe traits —
//! [`InferenceBackend`] and [`EmbeddingBackend`] — plus the validated
//! [`DenseTensor`] they exchange and deterministic [`mock` backends] that
//! let the whole product run, test, and benchmark with zero ML
//! dependencies (DECISIONS.md D2, D4, D5).
//!
//! ```text
//!   engine (decision graphs, rules, caches)        pure Rust
//!        │  Box<dyn InferenceBackend>                    ▲
//!        ▼                                               │ logits only (D7)
//!   opencodifier-runtime  ── MockInferenceBackend ───────┘
//!        │
//!        └──(feature-gated, feasibility-gated)── ONNX Runtime
//! ```
//!
//! # Contract
//!
//! * **Sync (D5).** Backend calls are bounded, blocking, local work. No
//!   async runtime exists in the core path.
//! * **Logits only (D7).** Backends return raw `f32` tensors. Softmax,
//!   calibration, and confidence are computed in Rust above this layer —
//!   raw softmax probability is never treated as calibrated confidence.
//! * **Determinism.** Identical inputs against the same
//!   `model_id` must give identical outputs; model ids feed the decision
//!   cache key, so changing a model changes the id or breaks the cache.
//! * **Typed failure.** Every failure is a [`RuntimeError`] with a stable
//!   `code()`. Backends never panic on hostile input; tensors are
//!   validated before a backend ever sees them.
//! * **No ML by default.** The default build links no inference library.
//!   ONNX support (ort rc.13, load-dynamic) is a feature that ships only
//!   after the feasibility gate recorded in `docs/PLAN.md` — and the
//!   deterministic path remains the default regardless.
//!
//! # Example
//!
//! Script a mock decision backend, run it, and embed candidate
//! descriptions — the same shapes the ONNX backends expose:
//!
//! ```
//! use std::collections::BTreeMap;
//!
//! use opencodifier_runtime::{
//!     DenseTensor, EmbeddingBackend, InferenceBackend, MockEmbeddingBackend,
//!     MockInferenceBackend,
//! };
//!
//! let logits = DenseTensor::new(vec![1, 2], vec![1.5, -0.5])?;
//! let backend: Box<dyn InferenceBackend> = Box::new(
//!     MockInferenceBackend::new("mock-decision-v1", BTreeMap::from([(
//!         "logits".to_owned(),
//!         logits,
//!     )]))
//!     .with_required_inputs(&["context", "candidates"]),
//! );
//!
//! let mut inputs = BTreeMap::new();
//! inputs.insert("context".to_owned(), DenseTensor::new(vec![1, 4], vec![0.0; 4])?);
//! inputs.insert("candidates".to_owned(), DenseTensor::new(vec![2, 4], vec![0.0; 8])?);
//!
//! let outputs = backend.infer(&inputs)?;
//! // Softmax over `outputs["logits"]` happens above this layer (D7).
//! assert_eq!(outputs["logits"].shape(), &[1, 2]);
//!
//! let embedder: Box<dyn EmbeddingBackend> =
//!     Box::new(MockEmbeddingBackend::new("mock-embed-v1", 32)?);
//! let vectors = embedder.embed(&["general coding", "deep reasoning"])?;
//! assert_eq!(vectors.len(), 2);
//! assert_eq!(vectors[0].len(), embedder.dims());
//! # Ok::<(), opencodifier_runtime::RuntimeError>(())
//! ```

pub mod backend;
pub mod error;
pub mod mock;
pub mod tensor;

pub use backend::{EmbeddingBackend, InferenceBackend};
pub use error::RuntimeError;
pub use mock::{MockEmbeddingBackend, MockInferenceBackend};
pub use tensor::{DenseTensor, MAX_TENSOR_ELEMENTS};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use crate::backend::{EmbeddingBackend, InferenceBackend};
    use crate::error::RuntimeError;
    use crate::mock::{MockEmbeddingBackend, MockInferenceBackend};
    use crate::tensor::DenseTensor;

    /// The two mock backends must be usable through the public trait
    /// objects from multiple threads at once — the engine's parallel
    /// waves depend on `Send + Sync` backends.
    #[test]
    fn backends_are_shared_across_threads_through_trait_objects() {
        let logits = DenseTensor::new(vec![1, 2], vec![1.0, 2.0]).unwrap();
        let inference: Box<dyn InferenceBackend> = Box::new(MockInferenceBackend::new(
            "mock-decision-v1",
            std::collections::BTreeMap::from([("logits".to_owned(), logits)]),
        ));
        let embedding: Box<dyn EmbeddingBackend> =
            Box::new(MockEmbeddingBackend::new("mock-embed-v1", 16).unwrap());

        std::thread::scope(|scope| {
            for _ in 0..4 {
                let inference = &inference;
                let embedding = &embedding;
                scope.spawn(move || {
                    let outputs = inference.infer(&std::collections::BTreeMap::new()).unwrap();
                    assert_eq!(outputs["logits"].row(0).unwrap(), &[1.0, 2.0]);
                    let vectors = embedding.embed(&["hello world"]).unwrap();
                    assert_eq!(vectors[0].len(), 16);
                });
            }
        });
    }

    /// Errors must convert transparently: a tensor validation failure
    /// raised while building a scripted output keeps its own code.
    #[test]
    fn tensor_errors_keep_their_codes_through_the_api() {
        let error = DenseTensor::new(vec![0], Vec::new()).unwrap_err();
        assert_eq!(error, RuntimeError::InvalidTensor { reason: "dimension 0 is zero".into() });
        assert_eq!(error.code(), "runtime.invalid_tensor");
    }
}
