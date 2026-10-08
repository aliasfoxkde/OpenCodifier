//! The serving scorer for the NLI verbalization verifier: the real
//! [`NliScorer`] over the locally fetched arm of record
//! (`MoritzLaurer/deberta-v3-base-zeroshot-v2.0`, its own
//! `onnx/model.onnx`) — pair tokenization through Hugging Face
//! `tokenizers`, one graph pass per request, softmax in Rust (D7).
//!
//! Weights are never committed or fetched at build time (D14): the
//! caller names a locally pulled directory and
//! [`crate::manifest`] verifies its bytes against the SHA-256
//! manifest before anything here runs in production.

use std::path::Path;
use std::sync::Arc;

use opencodifier_runtime::nli::NliOnnxBackend;
use opencodifier_runtime::{
    ATTENTION_MASK, DenseTensor, INPUT_IDS, InferenceBackend, LOGITS_OUTPUT,
};

use crate::error::ModelError;
use crate::nli::{ENTAIL_INDEX, NliScorer, NliVerifier, Verbalizer};

/// The input budget per pair, in tokens, before encoding refuses.
///
/// The arm of record's encoder positional window is 512; the runtime
/// refuses longer pairs rather than truncating, because a silently
/// cut premise would move the entailment readout the same way a cut
/// prompt moves Kai's option pointers.
pub const MAX_PAIR_TOKENS: usize = 512;

/// The real entailment scorer: ONNX graph plus the model directory's
/// own `tokenizer.json`.
#[derive(Debug)]
pub struct OnnxNliScorer {
    backend: NliOnnxBackend,
    tokenizer: tokenizers::Tokenizer,
    /// The pad token id used when a batch is collated to its longest
    /// row. The value is provably readout-neutral — the attention mask
    /// zeroes every pad position — so the tokenizer's declared pad is
    /// used when it publishes one, `0` otherwise.
    pad_id: u32,
}

impl OnnxNliScorer {
    /// Loads the scorer from a locally fetched model directory: the
    /// graph at the repository layout path plus the directory's own
    /// `tokenizer.json`.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when the tokenizer asset is missing
    /// or unloadable, and the runtime error's message when the graph
    /// cannot be loaded.
    pub fn from_dir(directory: &Path) -> Result<Self, ModelError> {
        let tokenizer_path = directory.join("tokenizer.json");
        let json =
            std::fs::read_to_string(&tokenizer_path).map_err(|error| ModelError::Tokenizer {
                reason: format!("reading `{}` failed: {error}", tokenizer_path.display()),
            })?;
        Self::from_parts(
            NliOnnxBackend::from_dir(directory).map_err(|error| ModelError::ContractViolation {
                reason: format!("graph load failed: {error}"),
            })?,
            &json,
        )
    }

    /// Assembles the scorer from an explicit backend and tokenizer
    /// JSON string — the seam the parity harness and alternate graph
    /// layouts use.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when the JSON is not a loadable
    /// tokenizer.
    pub fn from_parts(backend: NliOnnxBackend, tokenizer_json: &str) -> Result<Self, ModelError> {
        use std::str::FromStr;
        let mut tokenizer = tokenizers::Tokenizer::from_str(tokenizer_json)
            .map_err(|error| ModelError::Tokenizer { reason: format!("load failed: {error}") })?;
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: MAX_PAIR_TOKENS,
                ..tokenizers::TruncationParams::default()
            }))
            .map_err(|error| ModelError::Tokenizer {
                reason: format!("truncation setup failed: {error}"),
            })?;
        // Batches are padded manually to the longest row; the
        // tokenizer's own padding strategy must stay off so it cannot
        // disagree with this module's collation.
        tokenizer.with_padding(None);
        let pad_id = tokenizer.get_padding().map_or(0, |padding| padding.pad_id);
        Ok(Self { backend, tokenizer, pad_id })
    }

    /// The graph identity this scorer reads — the verifier's model-id
    /// prefix, so cached decisions are keyed to the exact graph bytes'
    /// file name (D6).
    #[must_use]
    pub fn model_prefix(&self) -> &str {
        self.backend.model_id()
    }
}

impl NliScorer for OnnxNliScorer {
    fn entailment_probabilities(
        &self,
        premise: &str,
        hypotheses: &[String],
    ) -> Result<Vec<f64>, ModelError> {
        if hypotheses.is_empty() {
            return Err(ModelError::ContractViolation {
                reason: "no hypotheses to score: the question verbalized to zero rows".to_owned(),
            });
        }
        let encodings = hypotheses
            .iter()
            .map(|hypothesis| {
                self.tokenizer.encode((premise.to_owned(), hypothesis.clone()), true).map_err(
                    |error| ModelError::Tokenizer {
                        reason: format!("pair encode failed: {error}"),
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let width = encodings.iter().map(|encoding| encoding.get_ids().len()).max().unwrap_or(1);
        let batch = encodings.len();
        let mut input_ids = Vec::with_capacity(batch * width);
        let mut attention_mask = Vec::with_capacity(batch * width);
        for encoding in &encodings {
            let ids = encoding.get_ids();
            // Token ids and mask values ride the Kai f32-tensor
            // convention: they stay far below 2^24, where f32 is
            // exact (the runtime validates the round-trip back to
            // `i64`).
            #[allow(clippy::cast_precision_loss)]
            let to_f32 = |value: &u32| *value as f32;
            input_ids.extend(ids.iter().map(to_f32));
            attention_mask.extend(encoding.get_attention_mask().iter().map(to_f32));
            // Rows shorter than the batch maximum pad to the right;
            // the mask positions above keep the readout identical to
            // a single-row pass.
            let padding = width - ids.len();
            #[allow(clippy::cast_precision_loss)]
            let pad = self.pad_id as f32;
            input_ids.extend(std::iter::repeat_n(pad, padding));
            attention_mask.extend(std::iter::repeat_n(0.0_f32, padding));
        }
        let tensors = std::collections::BTreeMap::from([
            (
                INPUT_IDS.to_owned(),
                DenseTensor::new(vec![batch, width], input_ids)
                    .map_err(|error| ModelError::ContractViolation { reason: error.to_string() })?,
            ),
            (
                ATTENTION_MASK.to_owned(),
                DenseTensor::new(vec![batch, width], attention_mask)
                    .map_err(|error| ModelError::ContractViolation { reason: error.to_string() })?,
            ),
        ]);
        let outputs = self.backend.infer(&tensors).map_err(|error| {
            ModelError::ContractViolation { reason: format!("graph execution failed: {error}") }
        })?;
        let logits = outputs.get(LOGITS_OUTPUT).ok_or_else(|| ModelError::ContractViolation {
            reason: format!("the graph did not return `{LOGITS_OUTPUT}`"),
        })?;
        let classes = logits.shape().get(1).copied().unwrap_or_default();
        let min_width = ENTAIL_INDEX + 1;
        if classes < min_width {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "`{LOGITS_OUTPUT}` row width {classes} has no entailment column \
                     {ENTAIL_INDEX}"
                ),
            });
        }
        let masses: Vec<f64> = (0..batch)
            .map(|row| {
                let values = logits.row(row).ok_or_else(|| ModelError::ContractViolation {
                    reason: format!("`{LOGITS_OUTPUT}` lost row {row} of {batch}"),
                })?;
                Ok(softmax_at(values, ENTAIL_INDEX))
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        Ok(masses)
    }
}

/// The softmax mass of `index` over one logits row, computed in `f64`
/// (D7): the max is subtracted for numerical stability, then the
/// requested column is divided by the full partition.
fn softmax_at(values: &[f32], index: usize) -> f64 {
    let logits: Vec<f64> = values.iter().map(|value| f64::from(*value)).collect();
    let max = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let shifted: Vec<f64> = logits.iter().map(|logit| (logit - max).exp()).collect();
    let partition: f64 = shifted.iter().sum();
    shifted[index] / partition
}

/// Builds the serving [`NliVerifier`]: the real scorer over the
/// locally fetched model directory, with `template` as the
/// verbalizer. The verifier's model id is the graph identity plus the
/// template tag (`nli-zeroshot:model.onnx:bare`), so swapping either
/// the graph or the template invalidates cached decisions (D6).
///
/// # Errors
///
/// Whatever [`OnnxNliScorer::from_dir`] reports.
pub fn verifier_from_dir(
    directory: &Path,
    template: Verbalizer,
) -> Result<NliVerifier, ModelError> {
    let scorer = OnnxNliScorer::from_dir(directory)?;
    let prefix = scorer.model_prefix().to_owned();
    Ok(NliVerifier::new(Arc::new(scorer), &prefix, template))
}
