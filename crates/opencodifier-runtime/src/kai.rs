//! The Kai-0.6B-ONNX graph transport (RESEARCH.md §10.5, task #92).
//!
//! The `onnx`-gated `KaiOnnxBackend` is the
//! ONNX half of the Kai decision rung: it runs
//! the fused graph published by `onnx-community/Decision-2.0-Kai-0.6B-ONNX`
//! and nothing else. The prompt rendering, tokenization, pointer
//! computation, score bias, and softmax all live in
//! `opencodifier-model`'s `kai` contract module — this layer only moves
//! validated tensors, exactly like every other
//! [`InferenceBackend`](crate::backend::InferenceBackend) (DECISIONS.md
//! D7: no softmax below the line).
//!
//! # Graph contract
//!
//! | Tensor           | Shape      | Element type | Meaning                          |
//! |------------------|------------|--------------|----------------------------------|
//! | `input_ids`      | `[B, L]`   | i64          | Right-padded prompt token ids    |
//! | `attention_mask` | `[B, L]`   | i64          | 1 for real tokens, 0 for padding |
//! | `answer_pos`     | `[B]`      | i64          | Query token index per row        |
//! | `option_pos`     | `[B, K]`   | i64          | Option endpoint per row/candidate|
//! | `logits`         | `[B, K]`   | f32          | Pre-softmax score per candidate  |
//!
//! Positions are absolute token indices and are read in-graph by
//! `GatherND`; the graph derives position ids from `L`, so right padding
//! with mask 0 (the upstream `parity.py` convention) is the only padding
//! this transport accepts.
//!
//! # Integral `f32` transport convention
//!
//! [`DenseTensor`] carries `f32` only, while this graph takes `i64`.
//! Index tensors therefore cross the boundary as **integral-valued
//! `f32`**: every value must be a whole number, and token ids and
//! positions stay far below 2^24, where `f32` is exact. A fractional
//! value is a caller bug and is rejected, never truncated.
//!
//! The backend is feature-gated (`onnx`) behind the D2 pin
//! (`ort = 2.0.0-rc.13`, `default-features = false`): the default build
//! links no ML runtime, and `runtime-onnx-dynamic` is the D2 packaging
//! that `dlopen`s a system ONNX Runtime instead of linking one.

use crate::error::RuntimeError;
use crate::tensor::DenseTensor;

/// Stable model id of the Kai-0.6B-ONNX rung. Feeds the decision cache
/// key, so it changes only when the served artifact or its readout
/// contract changes.
pub const MODEL_ID: &str = "kai-0.6b-onnx";

/// Token-id input name.
pub const INPUT_IDS: &str = "input_ids";

/// Attention-mask input name (1 = real token, 0 = right padding).
pub const ATTENTION_MASK: &str = "attention_mask";

/// Query-position input name.
pub const ANSWER_POS: &str = "answer_pos";

/// Option-endpoint input name.
pub const OPTION_POS: &str = "option_pos";

/// Score output name: pre-softmax logits, one per candidate.
pub const LOGITS_OUTPUT: &str = "logits";

/// The graph file the published repository ships for the q8 export.
pub const DEFAULT_GRAPH_FILE: &str = "model_quantized.onnx";

/// Reads an integral-valued `f32` tensor into `i64`.
///
/// The transport convention: index tensors cross the `f32`-only
/// [`DenseTensor`] boundary as whole numbers. Anything fractional is a
/// contract violation, reported rather than rounded away.
pub fn integral_i64(tensor: &DenseTensor, name: &str) -> Result<Vec<i64>, RuntimeError> {
    let mut values = Vec::with_capacity(tensor.data().len());
    for (index, value) in tensor.data().iter().enumerate() {
        let rounded = value.round();
        if !value.is_finite() || (value - rounded).abs() > f32::EPSILON {
            return Err(RuntimeError::InvalidTensor {
                reason: format!(
                    "`{name}` carries a non-integral value at flat index {index}: {value}"
                ),
            });
        }
        // Validated integral above; token ids stay far below `i64::MAX`.
        #[allow(clippy::cast_possible_truncation)]
        values.push(rounded as i64);
    }
    Ok(values)
}

/// Validates the four-shape Kai graph input set.
///
/// Returns `(batch, length, width)` on success. Shapes must agree across
/// the four inputs: disagreeing shapes are the "wrong base" class of
/// failure that pointer readouts turn into confident nonsense (RESEARCH
/// §6.5), so they are refused here rather than broadcast.
pub fn validate_shapes(
    input_ids: &DenseTensor,
    attention_mask: &DenseTensor,
    answer_pos: &DenseTensor,
    option_pos: &DenseTensor,
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
    if option_pos.shape().len() != 2 {
        return Err(RuntimeError::InvalidTensor {
            reason: format!("`{OPTION_POS}` must be rank 2, got {:?}", option_pos.shape()),
        });
    }
    let batch = input_ids.shape()[0];
    let length = input_ids.shape()[1];
    let width = option_pos.shape()[1];
    if answer_pos.shape() != [batch] {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{ANSWER_POS}` shape {:?} must be [{batch}] to match the batch",
                answer_pos.shape()
            ),
        });
    }
    if option_pos.shape()[0] != batch {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{OPTION_POS}` batch {} must equal `{INPUT_IDS}` batch {batch}",
                option_pos.shape()[0]
            ),
        });
    }
    Ok((batch, length, width))
}

/// Runs the fused Kai graph through ONNX Runtime.
#[cfg(feature = "onnx")]
#[derive(Debug)]
pub struct KaiOnnxBackend {
    session: std::sync::Mutex<ort::session::Session>,
    model_id: String,
}

/// One decoded batch, ready for the graph.
#[cfg(feature = "onnx")]
struct GraphBatch {
    input_ids: Vec<i64>,
    attention_mask: Vec<i64>,
    answer_pos: Vec<i64>,
    option_pos: Vec<i64>,
    batch: usize,
    length: usize,
    width: usize,
}

#[cfg(feature = "onnx")]
impl KaiOnnxBackend {
    /// Loads `model_quantized.onnx` (q8) from a model directory.
    ///
    /// The directory is the published repository layout: the graph file
    /// sits at its root. Weights are never committed to this workspace
    /// (DECISIONS.md D14); the caller supplies a locally fetched copy.
    pub fn from_dir(directory: &std::path::Path) -> Result<Self, RuntimeError> {
        Self::from_file(&directory.join(DEFAULT_GRAPH_FILE))
    }

    /// Loads a Kai graph from an explicit path.
    ///
    /// `KaiOnnxBackend::model_id` is [`MODEL_ID`] plus the file name, so
    /// two different exports never share a decision cache key (D6).
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
    /// `SessionOutputs` borrows the session, so the mutex guard and the
    /// logits copy live inside this method's scope: the backend hands back
    /// an owned [`DenseTensor`] and never a borrow of its session.
    fn run(&self, graph: &GraphBatch) -> Result<DenseTensor, RuntimeError> {
        let mut guard = self.session.lock().map_err(|_| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: "session mutex poisoned by a panicking thread".to_owned(),
        })?;
        let session = &mut *guard;
        let batch = graph.batch;
        let length = graph.length;
        let width = graph.width;
        let inputs: std::collections::HashMap<String, ort::value::Tensor<i64>> =
            std::collections::HashMap::from([
                (INPUT_IDS.to_owned(), make_i64(&graph.input_ids, vec![batch, length])?),
                (ATTENTION_MASK.to_owned(), make_i64(&graph.attention_mask, vec![batch, length])?),
                (ANSWER_POS.to_owned(), make_i64(&graph.answer_pos, vec![batch])?),
                (OPTION_POS.to_owned(), make_i64(&graph.option_pos, vec![batch, width])?),
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
impl crate::backend::InferenceBackend for KaiOnnxBackend {
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
        let answer_pos = inputs.get(ANSWER_POS).ok_or_else(|| missing(ANSWER_POS))?;
        let option_pos = inputs.get(OPTION_POS).ok_or_else(|| missing(OPTION_POS))?;
        let (batch, length, width) =
            validate_shapes(input_ids, attention_mask, answer_pos, option_pos)?;

        let graph_batch = GraphBatch {
            input_ids: integral_i64(input_ids, INPUT_IDS)?,
            attention_mask: integral_i64(attention_mask, ATTENTION_MASK)?,
            answer_pos: integral_i64(answer_pos, ANSWER_POS)?,
            option_pos: integral_i64(option_pos, OPTION_POS)?,
            batch,
            length,
            width,
        };
        let logits = self.run(&graph_batch)?;
        if logits.shape() != [batch, width] {
            return Err(RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!(
                    "`{LOGITS_OUTPUT}` must have shape [{batch}, {width}], got {:?}",
                    logits.shape()
                ),
            });
        }
        Ok(std::collections::BTreeMap::from([(LOGITS_OUTPUT.to_owned(), logits)]))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn wire_names_are_the_published_graph_names() {
        assert_eq!(INPUT_IDS, "input_ids");
        assert_eq!(ATTENTION_MASK, "attention_mask");
        assert_eq!(ANSWER_POS, "answer_pos");
        assert_eq!(OPTION_POS, "option_pos");
        assert_eq!(LOGITS_OUTPUT, "logits");
        assert_eq!(MODEL_ID, "kai-0.6b-onnx");
        assert_eq!(DEFAULT_GRAPH_FILE, "model_quantized.onnx");
    }

    #[test]
    fn integral_tensors_round_trip_and_fractional_ones_are_refused() {
        let clean = DenseTensor::new(vec![1, 3], vec![0.0, 1972.0, 44.0]).unwrap();
        assert_eq!(integral_i64(&clean, INPUT_IDS).unwrap(), vec![0, 1972, 44]);

        let fractional = DenseTensor::new(vec![1, 2], vec![1.0, 2.5]).unwrap();
        let error = integral_i64(&fractional, INPUT_IDS).unwrap_err();
        assert_eq!(error.code(), "runtime.invalid_tensor");
        assert!(error.to_string().contains("non-integral"), "{error}");
    }

    #[test]
    fn shapes_must_agree_across_the_four_inputs() {
        let ids = DenseTensor::new(vec![2, 5], vec![0.0; 10]).unwrap();
        let mask = DenseTensor::new(vec![2, 5], vec![0.0; 10]).unwrap();
        let answer = DenseTensor::new(vec![2], vec![4.0, 4.0]).unwrap();
        let options = DenseTensor::new(vec![2, 3], vec![0.0; 6]).unwrap();
        assert_eq!(validate_shapes(&ids, &mask, &answer, &options).unwrap(), (2, 5, 3));

        let wrong_mask = DenseTensor::new(vec![2, 4], vec![0.0; 8]).unwrap();
        let error = validate_shapes(&ids, &wrong_mask, &answer, &options).unwrap_err();
        assert!(error.to_string().contains("attention_mask"), "{error}");

        let wrong_answer = DenseTensor::new(vec![3], vec![0.0, 0.0, 0.0]).unwrap();
        let error = validate_shapes(&ids, &mask, &wrong_answer, &options).unwrap_err();
        assert!(error.to_string().contains("answer_pos"), "{error}");

        let wrong_options = DenseTensor::new(vec![1, 3], vec![0.0; 3]).unwrap();
        let error = validate_shapes(&ids, &mask, &answer, &wrong_options).unwrap_err();
        assert!(error.to_string().contains("option_pos"), "{error}");

        let rank1 = DenseTensor::new(vec![4], vec![0.0; 4]).unwrap();
        let error = validate_shapes(&rank1, &mask, &answer, &options).unwrap_err();
        assert!(error.to_string().contains("input_ids"), "{error}");
    }

    /// The backend struct is only constructed when a graph file exists,
    /// but the type must stay `Send + Sync` for the engine's threads.
    #[cfg(feature = "onnx")]
    #[test]
    fn backend_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<KaiOnnxBackend>();
    }
}
