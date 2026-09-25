//! The backend traits (PLANNING.md Phase 6, DECISIONS.md D2, D5, D7).
//!
//! Two traits, deliberately narrow:
//!
//! * [`InferenceBackend`] — run named tensors through a model, get named
//!   tensors back. The decision model (Phase 7) defines *which* names and
//!   shapes mean what; this layer only transports validated `f32` data.
//! * [`EmbeddingBackend`] — turn text into fixed-width `f32` vectors for
//!   the semantic layers (lexical fallback, similarity scoring).
//!
//! Both are **synchronous** (D5): the engine executes decision graphs on
//! plain threads, and a backend call is bounded work on a local model.
//! Blocking here is the contract — there is no async runtime in the core
//! path to block.
//!
//! Both are object-safe, `Send + Sync`, and return logits/vectors only
//! (D7): a backend never applies softmax, never calibrates, and never
//! generates text. Probability semantics live in Rust, above this line.

use std::collections::BTreeMap;

use crate::error::RuntimeError;
use crate::tensor::DenseTensor;

/// A named-tensor model runner: logits in spirit, `f32` tensors in fact.
///
/// Implementations must be deterministic for identical inputs given the
/// same [`model_id`](InferenceBackend::model_id) — cache correctness
/// depends on it, because cached decisions are keyed by request plus
/// model identity.
pub trait InferenceBackend: std::fmt::Debug + Send + Sync {
    /// Stable identifier of the model this backend serves. Must change
    /// when the model changes: it feeds the decision cache key.
    fn model_id(&self) -> &str;

    /// Runs the model over the named inputs.
    ///
    /// Inputs the model does not need may be ignored; inputs it needs and
    /// does not find are reported as [`RuntimeError::MissingInput`]. All
    /// failure modes are typed — a backend must never panic on any input
    /// it is handed.
    fn infer(
        &self,
        inputs: &BTreeMap<String, DenseTensor>,
    ) -> Result<BTreeMap<String, DenseTensor>, RuntimeError>;
}

/// A text-embedding source for the semantic layers.
///
/// Vectors are `f32`, finite, exactly [`EmbeddingBackend::dims`] wide.
/// Backends are encouraged to return L2-normalized vectors (the mock and
/// the reference ONNX encoder both do) so cosine and dot products
/// coincide; callers must not rely on it unless the backend documents it.
pub trait EmbeddingBackend: std::fmt::Debug + Send + Sync {
    /// Stable identifier of the embedding model. Feeds cache keys exactly
    /// like an inference model id.
    fn model_id(&self) -> &str;

    /// The fixed width of every vector this backend returns.
    fn dims(&self) -> usize;

    /// Embeds each text, preserving order.
    ///
    /// An empty slice yields an empty vector. Empty *texts* are valid
    /// input; what they embed to is the backend's documented choice.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    // The traits must stay object-safe: interface layers hold
    // `Box<dyn InferenceBackend>` / `Box<dyn EmbeddingBackend>` in the
    // engine facade.
    #[test]
    fn traits_are_object_safe_and_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn InferenceBackend>>();
        assert_send_sync::<Box<dyn EmbeddingBackend>>();
    }
}
