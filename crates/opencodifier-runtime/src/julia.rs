//! The Julia-1 ONNX graph transport (task #92).
//!
//! The `onnx`-gated `JuliaOnnxBackend` runs the fused graph of the
//! Julia-1 export and nothing else. Sequence building, marker math, and
//! softmax live in `opencodifier-model`'s `julia` contract module — this
//! layer only moves validated tensors, exactly like the Kai transport
//! ([`crate::kai`], DECISIONS.md D7: no softmax below the line).
//!
//! # Graph contract
//!
//! | Tensor           | Shape      | Element type | Meaning                            |
//! |------------------|------------|--------------|------------------------------------|
//! | `input_ids`      | `[B, L]`   | i64          | Prompt token ids                   |
//! | `attention_mask` | `[B, L]`   | i64          | 1 for real tokens, 0 for padding   |
//! | `marker_pos`     | `[B, K]`   | i64          | Option `<mask>` index per row/option |
//! | `marker_mask`    | `[B, K]`   | bool         | Which marker lanes are real        |
//! | `qtype`          | `[B]`      | i64          | Task type: choice 0, score 1, noul 2 |
//! | `logits`         | `[B, K]`   | f32          | Pre-softmax score per option       |
//!
//! Markers are absolute token indices read in-graph; the contract module
//! emits whole rows (`B = 1`), so the transport accepts any batch the
//! tensors agree on but the fixture-pinned path is a single row.
//!
//! # Integral `f32` transport convention
//!
//! [`DenseTensor`] carries `f32` only, while this graph takes `i64` and
//! `bool`. Index tensors cross as **integral-valued `f32`** (rejected
//! when fractional) and the mask crosses as `0.0`/`1.0` — anything else
//! is a caller bug and is refused, never rounded or coerced.
//!
//! The backend is feature-gated (`onnx`) behind the D2 pin
//! (`ort = 2.0.0-rc.13`, `default-features = false`).

use crate::error::RuntimeError;
use crate::kai::{ATTENTION_MASK, INPUT_IDS};
use crate::tensor::DenseTensor;

/// Stable model id of the Julia-1 ONNX rung. Feeds the decision cache
/// key, so it changes only when the served artifact or its readout
/// contract changes.
pub const MODEL_ID: &str = "julia-1-onnx";

/// Option-marker position input name.
pub const MARKER_POS: &str = "marker_pos";

/// Option-marker mask input name.
pub const MARKER_MASK: &str = "marker_mask";

/// Task-type input name. The graph reads the primitive from this tensor
/// rather than from the prompt alone: choice is 0, score 1, noul 2
/// (the contract module's `JuliaKind::qtype` emits the value).
pub const QTYPE_INPUT: &str = "qtype";

/// The graph file the export ships (external data beside it).
pub const DEFAULT_GRAPH_FILE: &str = "model.onnx";

/// Reads a `0.0`/`1.0` `f32` tensor into `bool`.
///
/// The mask transport convention. A value that is neither exactly zero
/// nor exactly one is a contract violation, not a truthiness input.
pub fn mask_bool(tensor: &DenseTensor, name: &str) -> Result<Vec<bool>, RuntimeError> {
    let mut values = Vec::with_capacity(tensor.data().len());
    for (index, value) in tensor.data().iter().enumerate() {
        match *value {
            0.0 => values.push(false),
            1.0 => values.push(true),
            other => {
                return Err(RuntimeError::InvalidTensor {
                    reason: format!(
                        "`{name}` must carry only 0.0/1.0 at flat index {index}, got {other}"
                    ),
                });
            }
        }
    }
    Ok(values)
}

/// Validates the five-shape Julia graph input set.
///
/// Returns `(batch, length, width)` on success. Disagreeing shapes are
/// the "wrong base" class of failure that marker readouts turn into
/// confident nonsense, so they are refused here rather than broadcast.
pub fn validate_shapes(
    input_ids: &DenseTensor,
    attention_mask: &DenseTensor,
    marker_pos: &DenseTensor,
    marker_mask: &DenseTensor,
    qtype: &DenseTensor,
) -> Result<(usize, usize, usize), RuntimeError> {
    if input_ids.shape().len() != 2 {
        return Err(RuntimeError::InvalidTensor {
            reason: format!("`{INPUT_IDS}` must be rank 2, got {:?}", input_ids.shape()),
        });
    }
    if attention_mask.shape() != input_ids.shape() {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{ATTENTION_MASK}` shape {:?} must equal `{INPUT_IDS}` shape {:?}",
                attention_mask.shape(),
                input_ids.shape()
            ),
        });
    }
    if marker_pos.shape().len() != 2 {
        return Err(RuntimeError::InvalidTensor {
            reason: format!("`{MARKER_POS}` must be rank 2, got {:?}", marker_pos.shape()),
        });
    }
    if marker_mask.shape() != marker_pos.shape() {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{MARKER_MASK}` shape {:?} must equal `{MARKER_POS}` shape {:?}",
                marker_mask.shape(),
                marker_pos.shape()
            ),
        });
    }
    let batch = input_ids.shape()[0];
    let length = input_ids.shape()[1];
    let width = marker_pos.shape()[1];
    if marker_pos.shape()[0] != batch {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{MARKER_POS}` batch {} must equal `{INPUT_IDS}` batch {batch}",
                marker_pos.shape()[0]
            ),
        });
    }
    if qtype.shape() != [batch] {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{QTYPE_INPUT}` shape {:?} must be [{batch}] to match the batch",
                qtype.shape()
            ),
        });
    }
    Ok((batch, length, width))
}

/// Runs the fused Julia-1 graph through ONNX Runtime.
#[cfg(feature = "onnx")]
#[derive(Debug)]
pub struct JuliaOnnxBackend {
    session: std::sync::Mutex<ort::session::Session>,
    model_id: String,
}

/// One decoded batch, ready for the graph.
#[cfg(feature = "onnx")]
struct GraphBatch {
    input_ids: Vec<i64>,
    attention_mask: Vec<i64>,
    marker_pos: Vec<i64>,
    marker_mask: Vec<bool>,
    qtype: Vec<i64>,
    batch: usize,
    length: usize,
    width: usize,
}

#[cfg(feature = "onnx")]
impl JuliaOnnxBackend {
    /// Loads `model.onnx` (external weights beside it) from a model
    /// directory.
    ///
    /// The directory is the export repository layout. Weights are never
    /// committed to this workspace (DECISIONS.md D14); the caller
    /// supplies a locally fetched copy.
    pub fn from_dir(directory: &std::path::Path) -> Result<Self, RuntimeError> {
        Self::from_file(&directory.join(DEFAULT_GRAPH_FILE))
    }

    /// Loads a Julia-1 graph from an explicit path.
    ///
    /// `JuliaOnnxBackend::model_id` is [`MODEL_ID`] plus the file name,
    /// so two different exports never share a decision cache key (D6).
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
    /// scope: the backend hands back an owned [`DenseTensor`] and never a
    /// borrow of its session.
    fn run(&self, graph: &GraphBatch) -> Result<DenseTensor, RuntimeError> {
        let mut guard = self.session.lock().map_err(|_| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: "session mutex poisoned by a panicking thread".to_owned(),
        })?;
        let session = &mut *guard;
        let batch = graph.batch;
        let length = graph.length;
        let width = graph.width;
        // Mixed element types (i64 ids + a bool mask) cross as dyn-typed
        // values; the names are checked against the graph at run time.
        let inputs: std::collections::HashMap<String, ort::value::Value> =
            std::collections::HashMap::from([
                (
                    crate::kai::INPUT_IDS.to_owned(),
                    make_i64(&graph.input_ids, vec![batch, length])?.into_dyn(),
                ),
                (
                    crate::kai::ATTENTION_MASK.to_owned(),
                    make_i64(&graph.attention_mask, vec![batch, length])?.into_dyn(),
                ),
                (
                    MARKER_POS.to_owned(),
                    make_i64(&graph.marker_pos, vec![batch, width])?.into_dyn(),
                ),
                (
                    MARKER_MASK.to_owned(),
                    make_bool(&graph.marker_mask, vec![batch, width])?.into_dyn(),
                ),
                (QTYPE_INPUT.to_owned(), make_i64(&graph.qtype, vec![batch])?.into_dyn()),
            ]);
        let outputs = session.run(inputs).map_err(|error| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: format!("graph execution failed: {error}"),
        })?;
        let output =
            outputs.get(crate::kai::LOGITS_OUTPUT).ok_or_else(|| RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!(
                    "the graph did not produce the `{}` output",
                    crate::kai::LOGITS_OUTPUT
                ),
            })?;
        let (shape, data) =
            output.try_extract_tensor::<f32>().map_err(|error| RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!("`{}` is not an f32 tensor: {error}", crate::kai::LOGITS_OUTPUT),
            })?;
        // Graph shapes are non-negative i64; a negative dimension is
        // impossible for a committed model.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let shape: Vec<usize> = shape.iter().map(|dimension| *dimension as usize).collect();
        DenseTensor::new(shape, data.to_vec())
    }
}

#[cfg(feature = "onnx")]
fn make_i64(data: &[i64], shape: Vec<usize>) -> Result<ort::value::Tensor<i64>, RuntimeError> {
    ort::value::Tensor::from_array((shape, data.to_vec())).map_err(|error| {
        RuntimeError::InvalidTensor { reason: format!("tensor build failed: {error}") }
    })
}

#[cfg(feature = "onnx")]
fn make_bool(data: &[bool], shape: Vec<usize>) -> Result<ort::value::Tensor<bool>, RuntimeError> {
    ort::value::Tensor::from_array((shape, data.to_vec())).map_err(|error| {
        RuntimeError::InvalidTensor { reason: format!("tensor build failed: {error}") }
    })
}

#[cfg(feature = "onnx")]
impl crate::backend::InferenceBackend for JuliaOnnxBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn infer(
        &self,
        inputs: &std::collections::BTreeMap<String, DenseTensor>,
    ) -> Result<std::collections::BTreeMap<String, DenseTensor>, RuntimeError> {
        let missing = |name: &str| RuntimeError::MissingInput { name: name.to_owned() };
        let input_ids =
            inputs.get(crate::kai::INPUT_IDS).ok_or_else(|| missing(crate::kai::INPUT_IDS))?;
        let attention_mask = inputs
            .get(crate::kai::ATTENTION_MASK)
            .ok_or_else(|| missing(crate::kai::ATTENTION_MASK))?;
        let marker_pos = inputs.get(MARKER_POS).ok_or_else(|| missing(MARKER_POS))?;
        let marker_mask = inputs.get(MARKER_MASK).ok_or_else(|| missing(MARKER_MASK))?;
        let qtype = inputs.get(QTYPE_INPUT).ok_or_else(|| missing(QTYPE_INPUT))?;
        let (batch, length, width) =
            validate_shapes(input_ids, attention_mask, marker_pos, marker_mask, qtype)?;

        let graph_batch = GraphBatch {
            input_ids: crate::kai::integral_i64(input_ids, INPUT_IDS)?,
            attention_mask: crate::kai::integral_i64(attention_mask, ATTENTION_MASK)?,
            marker_pos: crate::kai::integral_i64(marker_pos, MARKER_POS)?,
            marker_mask: mask_bool(marker_mask, MARKER_MASK)?,
            qtype: crate::kai::integral_i64(qtype, QTYPE_INPUT)?,
            batch,
            length,
            width,
        };
        let logits = self.run(&graph_batch)?;
        if logits.shape() != [batch, width] {
            return Err(RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!(
                    "`{}` must have shape [{batch}, {width}], got {:?}",
                    crate::kai::LOGITS_OUTPUT,
                    logits.shape()
                ),
            });
        }
        Ok(std::collections::BTreeMap::from([(crate::kai::LOGITS_OUTPUT.to_owned(), logits)]))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn wire_names_are_the_published_graph_names() {
        assert_eq!(MARKER_POS, "marker_pos");
        assert_eq!(MARKER_MASK, "marker_mask");
        assert_eq!(QTYPE_INPUT, "qtype");
        assert_eq!(MODEL_ID, "julia-1-onnx");
        assert_eq!(DEFAULT_GRAPH_FILE, "model.onnx");
        assert_eq!(crate::kai::INPUT_IDS, "input_ids");
        assert_eq!(crate::kai::LOGITS_OUTPUT, "logits");
    }

    #[test]
    fn mask_tensors_round_trip_and_stray_values_are_refused() {
        let clean = DenseTensor::new(vec![1, 3], vec![1.0, 0.0, 1.0]).unwrap();
        assert_eq!(mask_bool(&clean, MARKER_MASK).unwrap(), vec![true, false, true]);

        let stray = DenseTensor::new(vec![1, 2], vec![1.0, 0.5]).unwrap();
        let error = mask_bool(&stray, MARKER_MASK).unwrap_err();
        assert_eq!(error.code(), "runtime.invalid_tensor");
        assert!(error.to_string().contains("0.0/1.0"), "{error}");
    }

    #[test]
    fn shapes_must_agree_across_the_five_inputs() {
        let ids = DenseTensor::new(vec![1, 5], vec![0.0; 5]).unwrap();
        let mask = DenseTensor::new(vec![1, 5], vec![0.0; 5]).unwrap();
        let pos = DenseTensor::new(vec![1, 3], vec![0.0; 3]).unwrap();
        let flag = DenseTensor::new(vec![1, 3], vec![0.0; 3]).unwrap();
        let kind = DenseTensor::new(vec![1], vec![0.0]).unwrap();
        assert_eq!(validate_shapes(&ids, &mask, &pos, &flag, &kind).unwrap(), (1, 5, 3));

        let wrong_mask = DenseTensor::new(vec![1, 4], vec![0.0; 4]).unwrap();
        let error = validate_shapes(&ids, &wrong_mask, &pos, &flag, &kind).unwrap_err();
        assert!(error.to_string().contains("attention_mask"), "{error}");

        let wrong_flags = DenseTensor::new(vec![1, 2], vec![0.0; 2]).unwrap();
        let error = validate_shapes(&ids, &mask, &pos, &wrong_flags, &kind).unwrap_err();
        assert!(error.to_string().contains("marker_mask"), "{error}");

        let wrong_batch = DenseTensor::new(vec![2, 3], vec![0.0; 6]).unwrap();
        let error = validate_shapes(&ids, &mask, &wrong_batch, &flag, &kind).unwrap_err();
        assert!(error.to_string().contains("marker_pos"), "{error}");

        let wrong_kind = DenseTensor::new(vec![2], vec![0.0, 0.0]).unwrap();
        let error = validate_shapes(&ids, &mask, &pos, &flag, &wrong_kind).unwrap_err();
        assert!(error.to_string().contains("qtype"), "{error}");

        let rank1 = DenseTensor::new(vec![4], vec![0.0; 4]).unwrap();
        let error = validate_shapes(&rank1, &mask, &pos, &flag, &kind).unwrap_err();
        assert!(error.to_string().contains("input_ids"), "{error}");
    }

    /// The backend struct is only constructed when a graph file exists,
    /// but the type must stay `Send + Sync` for the engine's threads.
    #[cfg(feature = "onnx")]
    #[test]
    fn backend_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<JuliaOnnxBackend>();
    }
}
