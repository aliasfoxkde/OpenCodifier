//! The NLI zero-shot graph runner (PLAN unit 24j; RESEARCH §15.6.2).
//!
//! A cross-encoder sequence-classification graph — the arm of record is
//! `MoritzLaurer/deberta-v3-base-zeroshot-v2.0`, whose repository ships
//! its own `onnx/model.onnx` (the strongest no-self-conversion
//! provenance) — served through the same named-tensor contract as every
//! other backend: `input_ids` + `attention_mask` in (validated integral
//! `f32` rows, the [`crate::kai`] convention), `logits` out with shape
//! `[batch, label_count]`. Softmax, the entailment readout, and the
//! renormalization over candidates stay in Rust above this line (D7);
//! they live in `opencodifier-model`'s `nli` module behind its `nli`
//! feature.
//!
//! Weights are never committed to this workspace (D14): the caller
//! supplies a locally fetched copy, verified against its SHA-256
//! manifest through the `models verify` path.

// Everything below the doc comment is `onnx`-gated except the two
// consts and the wire-name test, so these imports ride the same
// conditions as their uses.
#[cfg(feature = "onnx")]
use crate::error::RuntimeError;
#[cfg(feature = "onnx")]
use crate::kai::integral_i64;
#[cfg(feature = "onnx")]
use crate::tensor::DenseTensor;

// The published graph names are shared with the Kai contract — same
// wire names, same values, one definition (`crate::kai`); the
// probabilistic semantics above them are what differ (D7).
#[cfg(any(feature = "onnx", test))]
use crate::kai::{ATTENTION_MASK, INPUT_IDS, LOGITS_OUTPUT};

/// Prefix of every [`NliOnnxBackend::model_id`].
pub const MODEL_ID: &str = "nli-zeroshot";

/// The graph file `from_dir` loads, in the arm of record's repository
/// layout (`<dir>/onnx/model.onnx`).
pub const DEFAULT_GRAPH_FILE: &str = "onnx/model.onnx";

/// Runs an NLI sequence-classification graph through ONNX Runtime.
#[cfg(feature = "onnx")]
#[derive(Debug)]
pub struct NliOnnxBackend {
    session: std::sync::Mutex<ort::session::Session>,
    model_id: String,
}

#[cfg(feature = "onnx")]
impl NliOnnxBackend {
    /// Loads `onnx/model.onnx` from a model directory (the arm of
    /// record's published repository layout).
    ///
    /// The directory is the locally fetched copy; weights are never
    /// committed to this workspace (D14).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BackendFailed`] when the session cannot be built
    /// or the graph cannot be loaded.
    pub fn from_dir(directory: &std::path::Path) -> Result<Self, RuntimeError> {
        Self::from_file(&directory.join(DEFAULT_GRAPH_FILE))
    }

    /// Loads an NLI graph from an explicit path.
    ///
    /// `model_id` is [`MODEL_ID`] plus the file name, so two different
    /// exports never share a decision cache key (D6).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BackendFailed`] when the session cannot be built
    /// or the graph cannot be loaded.
    pub fn from_file(path: &std::path::Path) -> Result<Self, RuntimeError> {
        let graph = path.file_name().map_or_else(
            || DEFAULT_GRAPH_FILE.to_owned(),
            |name| name.to_string_lossy().to_string(),
        );
        let mut builder =
            ort::session::Session::builder().map_err(|error| RuntimeError::BackendFailed {
                model_id: MODEL_ID.to_owned(),
                message: format!("session builder failed: {error}"),
            })?;
        let session =
            builder.commit_from_file(path).map_err(|error| RuntimeError::BackendFailed {
                model_id: MODEL_ID.to_owned(),
                message: format!("loading `{}` failed: {error}", path.display()),
            })?;
        Ok(Self {
            session: std::sync::Mutex::new(session),
            model_id: format!("{MODEL_ID}:{graph}"),
        })
    }

    /// Runs one validated batch through the graph.
    ///
    /// The mutex guard and the logits copy live inside this method's
    /// scope: the backend hands back an owned [`DenseTensor`], never a
    /// borrow of its session.
    fn run(
        &self,
        batch: usize,
        length: usize,
        input_ids: &[i64],
        attention_mask: &[i64],
    ) -> Result<DenseTensor, RuntimeError> {
        let mut guard = self.session.lock().map_err(|_| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: "session mutex poisoned by a panicking thread".to_owned(),
        })?;
        let session = &mut *guard;
        let inputs: std::collections::HashMap<String, ort::value::Tensor<i64>> =
            std::collections::HashMap::from([
                (INPUT_IDS.to_owned(), make_i64(input_ids, vec![batch, length])?),
                (ATTENTION_MASK.to_owned(), make_i64(attention_mask, vec![batch, length])?),
            ]);
        let outputs = session.run(inputs).map_err(|error| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: format!("graph execution failed: {error}"),
        })?;
        let output = outputs.get(LOGITS_OUTPUT).ok_or_else(|| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: format!("the graph did not produce the `{LOGITS_OUTPUT}` output"),
        })?;
        let (shape, data) =
            output.try_extract_tensor::<f32>().map_err(|error| RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!("`{LOGITS_OUTPUT}` is not an f32 tensor: {error}"),
            })?;
        // Graph shapes are non-negative i64; a negative dimension is
        // impossible for a committed model.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let shape: Vec<usize> = shape.iter().map(|dimension| *dimension as usize).collect();
        DenseTensor::new(shape, data.to_vec())
    }
}

/// Wraps `i64` data as an ONNX Runtime tensor of the given shape.
#[cfg(feature = "onnx")]
fn make_i64(data: &[i64], shape: Vec<usize>) -> Result<ort::value::Tensor<i64>, RuntimeError> {
    ort::value::Tensor::from_array((shape, data.to_vec())).map_err(|error| {
        RuntimeError::InvalidTensor { reason: format!("tensor build failed: {error}") }
    })
}

#[cfg(feature = "onnx")]
impl crate::backend::InferenceBackend for NliOnnxBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn infer(
        &self,
        inputs: &std::collections::BTreeMap<String, DenseTensor>,
    ) -> Result<std::collections::BTreeMap<String, DenseTensor>, RuntimeError> {
        let missing = |name: &str| RuntimeError::MissingInput { name: name.to_owned() };
        let input_ids = inputs.get(INPUT_IDS).ok_or_else(|| missing(INPUT_IDS))?;
        let attention_mask = inputs.get(ATTENTION_MASK).ok_or_else(|| missing(ATTENTION_MASK))?;
        if attention_mask.shape() != input_ids.shape() {
            return Err(RuntimeError::InvalidTensor {
                reason: format!(
                    "`{ATTENTION_MASK}` shape {:?} must equal `{INPUT_IDS}` shape {:?}",
                    attention_mask.shape(),
                    input_ids.shape()
                ),
            });
        }
        let shape = input_ids.shape();
        if shape.len() != 2 {
            return Err(RuntimeError::InvalidTensor {
                reason: format!("`{INPUT_IDS}` must be rank 2 [batch, length], got {shape:?}"),
            });
        }
        let (batch, length) = (shape[0], shape[1]);
        if batch == 0 || length == 0 {
            return Err(RuntimeError::InvalidTensor {
                reason: format!("`{INPUT_IDS}` carries an empty dimension: {shape:?}"),
            });
        }
        let logits = self.run(
            batch,
            length,
            &integral_i64(input_ids, INPUT_IDS)?,
            &integral_i64(attention_mask, ATTENTION_MASK)?,
        )?;
        if logits.shape()[0] != batch {
            return Err(RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!(
                    "`{LOGITS_OUTPUT}` batch {} must equal the input batch {batch}",
                    logits.shape()[0]
                ),
            });
        }
        Ok(std::collections::BTreeMap::from([(LOGITS_OUTPUT.to_owned(), logits)]))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn wire_names_are_the_published_graph_names() {
        assert_eq!(INPUT_IDS, "input_ids");
        assert_eq!(ATTENTION_MASK, "attention_mask");
        assert_eq!(LOGITS_OUTPUT, "logits");
        assert_eq!(DEFAULT_GRAPH_FILE, "onnx/model.onnx");
        // Distinct from every other backend family sharing a cache.
        assert_eq!(crate::kai::MODEL_ID, "kai-0.6b-onnx");
        assert_eq!(MODEL_ID, "nli-zeroshot");
    }
}
