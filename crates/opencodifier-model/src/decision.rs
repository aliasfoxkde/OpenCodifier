//! The candidate-conditioned inference contract (PLANNING.md §13, §46).
//!
//! [`CandidateConditionedModel`] is the serving bridge between an
//! [`InferenceBackend`] and the decision engine: given a context tensor
//! and one embedding row per candidate, it runs the backend, validates
//! the returned logits against the contract, and applies Rust-side
//! softmax (DECISIONS.md D7). The model itself — context encoder,
//! candidate encoder, per-candidate scalar logit, no text generation —
//! lives behind the backend boundary.
//!
//! # Honest scope (V1)
//!
//! This crate ships the **contract and the serving mechanics**, not
//! trained weights. No decision-model artifact exists yet (PLANNING.md:
//! the IR and deterministic engine come first, models later); until one
//! does, this type is exercised by tests against scripted backends and
//! becomes the production path unchanged when a manifest-verified
//! artifact (see [`crate::manifest`]) is available. It is *not* wired
//! into any default pipeline: without weights there is nothing to serve.
//!
//! # Contract
//!
//! | Tensor       | Shape     | Meaning                                  |
//! |--------------|-----------|------------------------------------------|
//! | `context`    | `[1, Dc]` | Encoded request state + question         |
//! | `candidates` | `[N, Dk]` | One encoded row per surviving candidate  |
//! | `logits`     | `[1, N]`  | Backend output: one raw logit per candidate |

use opencodifier_engine::{EngineError, EngineResult, softmax};
use opencodifier_runtime::{DenseTensor, InferenceBackend};

/// The name of the context input tensor.
pub const CONTEXT_INPUT: &str = "context";
/// The name of the candidates input tensor.
pub const CANDIDATES_INPUT: &str = "candidates";
/// The name of the logits output tensor.
pub const LOGITS_OUTPUT: &str = "logits";

/// Runs a candidate-conditioned decision model over any backend.
#[derive(Debug)]
pub struct CandidateConditionedModel {
    backend: Box<dyn InferenceBackend>,
}

impl CandidateConditionedModel {
    /// Serves the model carried by `backend`.
    ///
    /// The backend's `model_id` is the caller's contract to keep in sync
    /// with the manifest that shipped the artifact
    /// ([`ModelManifest::model_id`](crate::manifest::ModelManifest::model_id));
    /// this layer cannot verify weights, only honor ids.
    pub fn new(backend: Box<dyn InferenceBackend>) -> Self {
        Self { backend }
    }

    /// The model id of the served backend (feeds decision cache keys).
    pub fn model_id(&self) -> &str {
        self.backend.model_id()
    }

    /// Scores `candidates` (one row each) against `context`.
    ///
    /// Returns one softmax-normalized probability per candidate row, in
    /// candidate order. Fails with [`EngineError::ClassifierFailed`] if
    /// the backend fails and [`EngineError::InvalidDistribution`] if the
    /// backend's logits break the contract — a backend that returns the
    /// wrong shape is a bug, never silently repaired by truncation.
    pub fn score(&self, context: &DenseTensor, candidates: &DenseTensor) -> EngineResult<Vec<f64>> {
        let expected_rows = candidates.rows();
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert(CONTEXT_INPUT.to_owned(), context.clone());
        inputs.insert(CANDIDATES_INPUT.to_owned(), candidates.clone());

        let outputs =
            self.backend.infer(&inputs).map_err(|error| EngineError::ClassifierFailed {
                model_id: self.backend.model_id().to_owned(),
                reason: error.to_string(),
            })?;

        let logits = outputs.get(LOGITS_OUTPUT).ok_or_else(|| EngineError::ClassifierFailed {
            model_id: self.backend.model_id().to_owned(),
            reason: format!("backend did not return the `{LOGITS_OUTPUT}` tensor"),
        })?;
        if logits.shape() != [1, expected_rows] {
            return Err(EngineError::ClassifierFailed {
                model_id: self.backend.model_id().to_owned(),
                reason: format!(
                    "`{LOGITS_OUTPUT}` must have shape [1, {expected_rows}], got {:?}",
                    logits.shape()
                ),
            });
        }
        let scores: Vec<f64> = logits.row(0).unwrap_or(&[]).iter().map(|l| f64::from(*l)).collect();
        Ok(softmax(&scores))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_runtime::MockInferenceBackend;
    use std::collections::BTreeMap;

    fn context() -> DenseTensor {
        DenseTensor::new(vec![1, 4], vec![0.25, 0.5, 0.75, 1.0]).unwrap()
    }

    fn candidates(n: usize) -> DenseTensor {
        // Indices stay far below f32 mantissa precision in these tests.
        #[allow(clippy::cast_precision_loss)]
        let data = (0..n * 4).map(|i| (i % 7) as f32 * 0.125).collect();
        DenseTensor::new(vec![n, 4], data).unwrap()
    }

    fn backend_with_logits(logits: Vec<f32>) -> Box<dyn InferenceBackend> {
        let outputs = BTreeMap::from([(
            LOGITS_OUTPUT.to_owned(),
            DenseTensor::new(vec![1, logits.len()], logits).unwrap(),
        )]);
        Box::new(MockInferenceBackend::new("decision-mock-v1", outputs))
    }

    #[test]
    fn scores_normalize_over_candidate_rows_in_order() {
        let model = CandidateConditionedModel::new(backend_with_logits(vec![2.0, -2.0, 0.0]));
        assert_eq!(model.model_id(), "decision-mock-v1");

        let probabilities = model.score(&context(), &candidates(3)).unwrap();
        assert_eq!(probabilities.len(), 3);
        assert!(probabilities[0] > probabilities[2]);
        assert!(probabilities[2] > probabilities[1]);
        assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_missing_logits_tensor_is_a_typed_failure() {
        let outputs =
            BTreeMap::from([("scores".to_owned(), DenseTensor::new(vec![1], vec![0.0]).unwrap())]);
        let model =
            CandidateConditionedModel::new(Box::new(MockInferenceBackend::new("m", outputs)));
        let error = model.score(&context(), &candidates(1)).unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("`logits`"), "{error}");
    }

    #[test]
    fn a_wrong_logit_shape_is_rejected_not_truncated() {
        let model = CandidateConditionedModel::new(backend_with_logits(vec![1.0, 2.0]));
        let error = model.score(&context(), &candidates(3)).unwrap_err();
        assert!(error.to_string().contains("[1, 3], got [1, 2]"), "{error}");
    }

    #[test]
    fn backend_failures_propagate_with_the_model_id() {
        let model = CandidateConditionedModel::new(Box::new(
            MockInferenceBackend::new("broken-model", BTreeMap::new())
                .with_failure("weights corrupt"),
        ));
        let error = model.score(&context(), &candidates(2)).unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(
            error.to_string().contains("broken-model")
                && error.to_string().contains("weights corrupt"),
            "{error}"
        );
    }

    #[test]
    fn non_finite_logits_degrade_to_uniform_via_softmax() {
        // The backend boundary rejects non-finite *inputs*; a backend that
        // still returns them must not crash the engine — softmax's
        // documented garbage fallback keeps decisions defined.
        let outputs = BTreeMap::from([(
            LOGITS_OUTPUT.to_owned(),
            DenseTensor::new(vec![1, 2], vec![f32::MAX, -f32::MAX]).unwrap(),
        )]);
        let model =
            CandidateConditionedModel::new(Box::new(MockInferenceBackend::new("m", outputs)));
        let probabilities = model.score(&context(), &candidates(2)).unwrap();
        assert_eq!(probabilities.len(), 2);
        assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }
}
