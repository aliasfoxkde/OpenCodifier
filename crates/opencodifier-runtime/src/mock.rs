//! Deterministic mock backends (PLANNING.md Phase 6).
//!
//! These are test doubles, not simulated models: they make **no claim**
//! about semantic quality. Their value is that they are exact — identical
//! inputs always produce identical outputs on every platform, so tests
//! and benches can assert on precise outputs rather than tolerances.
//! The engine's full pipeline runs against them with zero ML.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backend::{EmbeddingBackend, InferenceBackend};
use crate::error::RuntimeError;
use crate::tensor::DenseTensor;

/// A scripted [`InferenceBackend`].
///
/// Returns clones of configured output tensors on every call, optionally
/// requiring named inputs to be present, optionally failing. Call counts
/// are exposed so tests can assert how often the (expensive) layer ran.
#[derive(Debug)]
pub struct MockInferenceBackend {
    model_id: String,
    outputs: BTreeMap<String, DenseTensor>,
    required: Vec<String>,
    failure: Option<String>,
    calls: AtomicU64,
}

impl MockInferenceBackend {
    /// Creates a backend that answers every call with `outputs`.
    pub fn new(model_id: impl Into<String>, outputs: BTreeMap<String, DenseTensor>) -> Self {
        Self {
            model_id: model_id.into(),
            outputs,
            required: Vec::new(),
            failure: None,
            calls: AtomicU64::new(0),
        }
    }

    /// Requires these input names to be present on every call, reporting
    /// [`RuntimeError::MissingInput`] for the first absent one (checked
    /// in the given order).
    #[must_use]
    pub fn with_required_inputs(mut self, required: &[&str]) -> Self {
        self.required = required.iter().map(|name| (*name).to_owned()).collect();
        self
    }

    /// Makes every call fail with [`RuntimeError::BackendFailed`] carrying
    /// `message` — for exercising failure paths above this layer.
    #[must_use]
    pub fn with_failure(mut self, message: impl Into<String>) -> Self {
        self.failure = Some(message.into());
        self
    }

    /// How many times [`InferenceBackend::infer`] was called.
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

impl InferenceBackend for MockInferenceBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn infer(
        &self,
        inputs: &BTreeMap<String, DenseTensor>,
    ) -> Result<BTreeMap<String, DenseTensor>, RuntimeError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(message) = &self.failure {
            return Err(RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: message.clone(),
            });
        }
        for name in &self.required {
            if !inputs.contains_key(name) {
                return Err(RuntimeError::MissingInput { name: name.clone() });
            }
        }
        Ok(self.outputs.clone())
    }
}

/// A deterministic [`EmbeddingBackend`] with no model file.
///
/// Embeds text as an L2-normalized bag of words hashed into `dims`
/// buckets with FNV-1a (a fixed hash, unlike `std`'s per-process seeded
/// hasher — determinism here is a contract, not an implementation
/// detail). An empty text embeds to the all-zero vector; any non-empty
/// text embeds to a unit vector.
#[derive(Debug, Clone)]
pub struct MockEmbeddingBackend {
    model_id: String,
    dims: usize,
}

impl MockEmbeddingBackend {
    /// Creates a mock embedder with `dims` buckets (at least 1).
    ///
    /// `dims` is capped at 4096: the mock exists for tests of narrow
    /// semantic layers, not to imitate a large embedding space.
    pub fn new(model_id: impl Into<String>, dims: usize) -> Result<Self, RuntimeError> {
        if dims == 0 || dims > 4096 {
            return Err(RuntimeError::InvalidTensor {
                reason: format!("mock embedding dims must be in 1..=4096, got {dims}"),
            });
        }
        Ok(Self { model_id: model_id.into(), dims })
    }
}

/// FNV-1a over bytes; stable across platforms and processes.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl EmbeddingBackend for MockEmbeddingBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0_f32; self.dims];
                for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
                    if token.is_empty() {
                        continue;
                    }
                    // `dims` is capped at 4096, so this narrowing cannot
                    // truncate on any supported target.
                    #[allow(clippy::cast_possible_truncation)]
                    let slot =
                        (fnv1a(token.to_ascii_lowercase().as_bytes()) % self.dims as u64) as usize;
                    vector[slot] += 1.0;
                }
                let norm: f32 = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for value in &mut vector {
                        *value /= norm;
                    }
                }
                vector
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    fn logits(rows: usize, width: usize, base: f32) -> DenseTensor {
        // Indices stay far below f32 mantissa precision in these tests.
        #[allow(clippy::cast_precision_loss)]
        let data = (0..rows * width).map(|i| base + i as f32 * 0.5).collect();
        DenseTensor::new(vec![rows, width], data).unwrap()
    }

    #[test]
    fn scripted_backend_returns_outputs_and_counts_calls() {
        let backend = MockInferenceBackend::new(
            "mock-decision-v1",
            BTreeMap::from([("logits".to_owned(), logits(1, 3, 0.0))]),
        );
        assert_eq!(backend.model_id(), "mock-decision-v1");
        assert_eq!(backend.calls(), 0);

        let inputs = BTreeMap::new();
        let out = backend.infer(&inputs).unwrap();
        assert_eq!(out["logits"], logits(1, 3, 0.0));
        assert_eq!(out["logits"].row(0).unwrap(), &[0.0, 0.5, 1.0]);
        backend.infer(&inputs).unwrap();
        assert_eq!(backend.calls(), 2);
    }

    #[test]
    fn required_inputs_are_enforced_by_name() {
        let backend =
            MockInferenceBackend::new("m", BTreeMap::from([("o".to_owned(), logits(1, 1, 0.0))]))
                .with_required_inputs(&["context", "candidates"]);
        let missing_both = backend.infer(&BTreeMap::new()).unwrap_err();
        assert_eq!(missing_both.code(), "runtime.missing_input");
        assert!(missing_both.to_string().contains("`context`"), "{missing_both}");

        let mut only_context = BTreeMap::new();
        only_context.insert("context".to_owned(), logits(1, 1, 0.0));
        let missing_second = backend.infer(&only_context).unwrap_err();
        assert!(missing_second.to_string().contains("`candidates`"), "{missing_second}");

        only_context.insert("candidates".to_owned(), logits(2, 1, 1.0));
        assert!(backend.infer(&only_context).is_ok());
    }

    #[test]
    fn injected_failures_surface_as_backend_failed() {
        let backend =
            MockInferenceBackend::new("broken", BTreeMap::new()).with_failure("session closed");
        let error = backend.infer(&BTreeMap::new()).unwrap_err();
        assert_eq!(error.code(), "runtime.backend_failed");
        assert_eq!(error.to_string(), "backend `broken` failed: session closed");
    }

    #[test]
    fn mock_embedder_is_deterministic_normalized_and_ordered() {
        let backend = MockEmbeddingBackend::new("mock-embed-v1", 64).unwrap();
        assert_eq!(backend.model_id(), "mock-embed-v1");
        assert_eq!(backend.dims(), 64);

        let texts = ["route this request", "route this request", "different words entirely", ""];
        let vectors = backend.embed(&texts).unwrap();
        assert_eq!(vectors.len(), texts.len());
        assert_eq!(vectors[0], vectors[1], "identical texts embed identically");
        assert_ne!(vectors[0], vectors[2], "different texts differ");

        let norm: f32 = vectors[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "non-empty text must be unit length, got {norm}");
        assert!(vectors[3].iter().all(|v| *v == 0.0), "empty text embeds to the zero vector");

        // Repeatability across separate calls and separate instances.
        let again = MockEmbeddingBackend::new("mock-embed-v1", 64).unwrap();
        assert_eq!(again.embed(&texts[..1]).unwrap()[0], vectors[0]);
    }

    #[test]
    fn mock_embedder_rejects_out_of_range_dims() {
        for dims in [0, 4097] {
            let error = MockEmbeddingBackend::new("m", dims).unwrap_err();
            assert_eq!(error.code(), "runtime.invalid_tensor", "dims {dims}");
            assert!(error.to_string().contains("1..=4096"), "{error}");
        }
        assert!(MockEmbeddingBackend::new("m", 1).is_ok());
        assert!(MockEmbeddingBackend::new("m", 4096).is_ok());
    }
}
