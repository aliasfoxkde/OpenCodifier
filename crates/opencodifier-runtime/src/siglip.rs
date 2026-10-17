//! The `SigLIP2` dual-encoder graph runners (track V, phase F).
//!
//! Arm of record: `onnx-community/siglip2-base-patch16-256-ONNX`, which
//! ships the two towers as separate graphs (`onnx/vision_model.onnx`,
//! `onnx/text_model.onnx`) beside the 1.5 GiB monolith
//! (`onnx/model.onnx`). The split towers are the serving shape: the
//! text tower runs once per question set, the image tower once per
//! state, and no monolith-sized session is ever resident (the D9
//! latency budget is per-stage, and a resident monolith would charge
//! every request for both towers).
//!
//! Wire contract (grounded against the transformers.js and
//! optimum-onnx export configs for SigLIP2):
//!
//! * vision graph — input `pixel_values` f32 `[batch, 3, height,
//!   width]`; output `pooler_output` f32 `[batch, E]`, the
//!   head-projected image embedding (SigLIP2-base: `E = 768`).
//! * text graph — inputs `input_ids` i64 rows (the kai convention) and
//!   `attention_mask` i64 rows when the export carries it (the caller
//!   always supplies the tokenization's mask — a fabricated one would
//!   silently decide which pad positions the tower attends); output
//!   `pooler_output` f32 `[batch, E]`, likewise projected.
//!
//! Nothing above raw named tensors happens here: L2 renormalization,
//! the zero-shot head, and prompt templates are semantics and stay in
//! `opencodifier-model` (D7). Embeddings are returned unnormalized in
//! graph order, exactly as the graphs produce them.
//!
//! Weights are never committed to this workspace (D14): the caller
//! supplies a locally fetched copy, verified against its SHA-256
//! manifest through the `models verify` path.

// Everything below the doc comment is `onnx`-gated except the consts
// and the wire-name test, so these imports ride the same conditions as
// their uses.
#[cfg(any(feature = "onnx", test))]
use crate::kai::{ATTENTION_MASK, INPUT_IDS};

/// The vision-tower input: preprocessed float pixels `[B, 3, H, W]`
/// (CHW order, mean/std normalized by the caller's preprocessing).
#[cfg(any(feature = "onnx", test))]
pub const PIXEL_VALUES: &str = "pixel_values";

/// The shared output name of both towers: the head-projected pooled
/// embedding rows, `[B, E]`, unnormalized.
#[cfg(any(feature = "onnx", test))]
pub const POOLER_OUTPUT: &str = "pooler_output";

/// Prefix of every tower's `model_id` (the backend structs are
/// feature-gated, so no doc link here). Each tower appends the graph
/// file name; the ids never collide with each other or with another
/// family's ids (D6).
#[cfg(any(feature = "onnx", test))]
pub const MODEL_ID: &str = "siglip2";

/// The vision graph file `from_dir` loads, in the arm-of-record's
/// repository layout.
#[cfg(any(feature = "onnx", test))]
pub const VISION_GRAPH_FILE: &str = "onnx/vision_model.onnx";

/// The text graph file `from_dir` loads, same layout.
#[cfg(any(feature = "onnx", test))]
pub const TEXT_GRAPH_FILE: &str = "onnx/text_model.onnx";

#[cfg(any(feature = "onnx", test))]
use crate::error::RuntimeError;
#[cfg(any(feature = "onnx", test))]
use crate::tensor::DenseTensor;

/// Returns the sorted demanded input names the session carries.
///
/// The split-tower graphs differ by export: the text tower
/// conditionally carries `attention_mask`, so the graph's actual
/// signature is read once at load and never guessed at run time.
#[cfg(feature = "onnx")]
fn session_input_names(session: &ort::session::Session) -> Vec<String> {
    session.inputs().iter().map(|outlet| outlet.name().to_owned()).collect()
}

/// Checks a graph's demanded input names against the runner's contract:
/// every required name must be demanded and nothing foreign may be
/// demanded in its place. Returns a typed-refusal reason on mismatch.
#[cfg(any(feature = "onnx", test))]
fn input_name_mismatch(tower: &str, demanded: &[String], required: &[&str]) -> Option<String> {
    for name in required {
        if !demanded.iter().any(|demanded_name| demanded_name == name) {
            return Some(format!(
                "the {tower} graph does not demand the required input `{name}` (it demands: {demanded:?})"
            ));
        }
    }
    let known = required.to_vec();
    for demanded_name in demanded {
        if !known.contains(&demanded_name.as_str()) {
            return Some(format!(
                "the {tower} graph demands unknown input `{demanded_name}` (expected exactly: {known:?})"
            ));
        }
    }
    None
}

#[cfg(feature = "onnx")]
fn make_f32(data: &[f32], shape: Vec<usize>) -> Result<ort::value::Tensor<f32>, RuntimeError> {
    ort::value::Tensor::from_array((shape, data.to_vec())).map_err(|error| {
        RuntimeError::InvalidTensor { reason: format!("tensor build failed: {error}") }
    })
}

#[cfg(feature = "onnx")]
fn make_i64(data: &[i64], shape: Vec<usize>) -> Result<ort::value::Tensor<i64>, RuntimeError> {
    ort::value::Tensor::from_array((shape, data.to_vec())).map_err(|error| {
        RuntimeError::InvalidTensor { reason: format!("tensor build failed: {error}") }
    })
}

/// Validates the vision tower's named inputs and returns the shape
/// triple `(batch, height, width)` the graph demands. Pure validation —
/// no session involved, testable without weights.
#[cfg(any(feature = "onnx", test))]
fn validate_pixel_values(
    inputs: &std::collections::BTreeMap<String, DenseTensor>,
) -> Result<(usize, usize, usize), RuntimeError> {
    let missing = |name: &str| RuntimeError::MissingInput { name: name.to_owned() };
    let pixels = inputs.get(PIXEL_VALUES).ok_or_else(|| missing(PIXEL_VALUES))?;
    let shape = pixels.shape().to_vec();
    if shape.len() != 4 {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{PIXEL_VALUES}` must be rank 4 [batch, 3, height, width], got {shape:?}"
            ),
        });
    }
    let (batch, channels, height, width) = (shape[0], shape[1], shape[2], shape[3]);
    if batch == 0 || height == 0 || width == 0 {
        return Err(RuntimeError::InvalidTensor {
            reason: format!("`{PIXEL_VALUES}` carries an empty dimension: {shape:?}"),
        });
    }
    if channels != 3 {
        return Err(RuntimeError::InvalidTensor {
            reason: format!("`{PIXEL_VALUES}` channel dimension must be 3 (RGB), got {channels}"),
        });
    }
    let expected = batch * channels * height * width;
    if pixels.data().len() != expected {
        return Err(RuntimeError::InvalidTensor {
            reason: format!(
                "`{PIXEL_VALUES}` holds {} values, expected {expected} for {shape:?}",
                pixels.data().len()
            ),
        });
    }
    Ok((batch, height, width))
}

/// Validates the text tower's named inputs and returns the shape pair
/// `(batch, length)` the graph demands, plus the mask rows when the
/// caller supplied them. Pure validation — no session involved.
#[cfg(any(feature = "onnx", test))]
fn validate_id_inputs(
    inputs: &std::collections::BTreeMap<String, DenseTensor>,
) -> Result<(usize, usize, Option<Vec<i64>>), RuntimeError> {
    let missing = |name: &str| RuntimeError::MissingInput { name: name.to_owned() };
    let ids = inputs.get(INPUT_IDS).ok_or_else(|| missing(INPUT_IDS))?;
    let shape = ids.shape().to_vec();
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
    let mask = match inputs.get(ATTENTION_MASK) {
        Some(mask) => {
            if mask.shape() != ids.shape() {
                return Err(RuntimeError::InvalidTensor {
                    reason: format!(
                        "`{ATTENTION_MASK}` shape {:?} must equal `{INPUT_IDS}` shape {shape:?}",
                        mask.shape()
                    ),
                });
            }
            Some(crate::kai::integral_i64(mask, ATTENTION_MASK)?)
        }
        None => None,
    };
    // Terrain check for the ids themselves: i64 tokens validate by
    // construction above the graph, never by reinterpretation (the
    // kai convention).
    crate::kai::integral_i64(ids, INPUT_IDS)?;
    Ok((batch, length, mask))
}

#[cfg(feature = "onnx")]
fn create_session(
    model_id: &str,
    path: &std::path::Path,
) -> Result<ort::session::Session, RuntimeError> {
    let mut builder =
        ort::session::Session::builder().map_err(|error| RuntimeError::BackendFailed {
            model_id: model_id.to_owned(),
            message: format!("session builder failed: {error}"),
        })?;
    builder.commit_from_file(path).map_err(|error| RuntimeError::BackendFailed {
        model_id: model_id.to_owned(),
        message: format!("loading `{}` failed: {error}", path.display()),
    })
}

#[cfg(feature = "onnx")]
fn extract_pooled(
    tower: &str,
    model_id: &str,
    outputs: &mut ort::session::SessionOutputs<'_>,
) -> Result<DenseTensor, RuntimeError> {
    let output = outputs.get(POOLER_OUTPUT).ok_or_else(|| RuntimeError::BackendFailed {
        model_id: model_id.to_owned(),
        message: format!("the {tower} graph did not produce the `{POOLER_OUTPUT}` output"),
    })?;
    let (shape, data) =
        output.try_extract_tensor::<f32>().map_err(|error| RuntimeError::BackendFailed {
            model_id: model_id.to_owned(),
            message: format!("`{POOLER_OUTPUT}` is not an f32 tensor: {error}"),
        })?;
    // Graph shapes are non-negative i64; a negative dimension is
    // impossible for a committed model.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let shape: Vec<usize> = shape.iter().map(|dimension| *dimension as usize).collect();
    DenseTensor::new(shape, data.to_vec())
}

#[cfg(feature = "onnx")]
fn run_vision_session(
    session: &mut ort::session::Session,
    model_id: &str,
    batch: usize,
    height: usize,
    width: usize,
    pixels: &[f32],
) -> Result<DenseTensor, RuntimeError> {
    let inputs: std::collections::HashMap<String, ort::value::Tensor<f32>> =
        std::collections::HashMap::from([(
            PIXEL_VALUES.to_owned(),
            make_f32(pixels, vec![batch, 3, height, width])?,
        )]);
    let mut outputs = session.run(inputs).map_err(|error| RuntimeError::BackendFailed {
        model_id: model_id.to_owned(),
        message: format!("vision graph execution failed: {error}"),
    })?;
    extract_pooled("vision", model_id, &mut outputs)
}

#[cfg(feature = "onnx")]
fn run_text_session(
    session: &mut ort::session::Session,
    model_id: &str,
    batch: usize,
    length: usize,
    ids: &[i64],
    mask: Option<&[i64]>,
) -> Result<DenseTensor, RuntimeError> {
    let mut inputs: std::collections::HashMap<String, ort::value::Tensor<i64>> =
        std::collections::HashMap::from([(
            INPUT_IDS.to_owned(),
            make_i64(ids, vec![batch, length])?,
        )]);
    if let Some(mask) = mask {
        inputs.insert(ATTENTION_MASK.to_owned(), make_i64(mask, vec![batch, length])?);
    }
    let mut outputs = session.run(inputs).map_err(|error| RuntimeError::BackendFailed {
        model_id: model_id.to_owned(),
        message: format!("text graph execution failed: {error}"),
    })?;
    extract_pooled("text", model_id, &mut outputs)
}

/// The `SigLIP2` vision tower over ONNX Runtime.
///
/// One session, one responsibility: preprocessed `pixel_values` in,
/// head-projected image embeddings out. Everything from image decoding
/// to normalization happens above this layer.
#[cfg(feature = "onnx")]
#[derive(Debug)]
pub struct Siglip2VisionOnnxBackend {
    session: std::sync::Mutex<ort::session::Session>,
    model_id: String,
}

#[cfg(feature = "onnx")]
impl Siglip2VisionOnnxBackend {
    /// Loads `onnx/vision_model.onnx` from a model directory (the
    /// arm-of-record's published repository layout).
    ///
    /// The directory is the locally fetched copy; weights are never
    /// committed to this workspace (D14).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BackendFailed`] when the session cannot be
    /// built, the graph cannot be loaded, or the graph's signature is
    /// not the split vision tower's.
    pub fn from_dir(directory: &std::path::Path) -> Result<Self, RuntimeError> {
        Self::from_file(&directory.join(VISION_GRAPH_FILE))
    }

    /// Loads a vision tower from an explicit graph path.
    ///
    /// The `model_id` carries the file name, so two different exports
    /// never share a decision cache key (D6).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BackendFailed`] as for [`from_dir`](Self::from_dir),
    /// plus a typed refusal when the graph's signature is foreign.
    pub fn from_file(path: &std::path::Path) -> Result<Self, RuntimeError> {
        let graph = path.file_name().map_or_else(
            || VISION_GRAPH_FILE.to_owned(),
            |name| name.to_string_lossy().to_string(),
        );
        let service_id = format!("{MODEL_ID}:vision");
        let session = create_session(&service_id, path)?;
        let demanded = session_input_names(&session);
        if let Some(reason) = input_name_mismatch("vision", &demanded, &[PIXEL_VALUES]) {
            return Err(RuntimeError::BackendFailed { model_id: service_id, message: reason });
        }
        Ok(Self {
            session: std::sync::Mutex::new(session),
            model_id: format!("{service_id}:{graph}"),
        })
    }
}

#[cfg(feature = "onnx")]
impl crate::backend::InferenceBackend for Siglip2VisionOnnxBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn infer(
        &self,
        inputs: &std::collections::BTreeMap<String, DenseTensor>,
    ) -> Result<std::collections::BTreeMap<String, DenseTensor>, RuntimeError> {
        let (batch, height, width) = validate_pixel_values(inputs)?;
        let pixels = &inputs[PIXEL_VALUES];
        let mut guard = self.session.lock().map_err(|_| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: "session mutex poisoned by a panicking thread".to_owned(),
        })?;
        let embeddings =
            run_vision_session(&mut guard, &self.model_id, batch, height, width, pixels.data())?;
        if embeddings.shape()[0] != batch {
            return Err(RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!(
                    "`{POOLER_OUTPUT}` batch {} must equal the input batch {batch}",
                    embeddings.shape()[0]
                ),
            });
        }
        Ok(std::collections::BTreeMap::from([(POOLER_OUTPUT.to_owned(), embeddings)]))
    }
}

/// The `SigLIP2` text tower over ONNX Runtime.
///
/// Tokenized `input_ids` in, head-projected text embeddings out. When
/// the loaded graph demands `attention_mask`, the caller supplies it —
/// the mask is the tokenization's facts, never the runner's invention.
#[cfg(feature = "onnx")]
#[derive(Debug)]
pub struct Siglip2TextOnnxBackend {
    session: std::sync::Mutex<ort::session::Session>,
    model_id: String,
    /// true when the loaded graph demands an `attention_mask` input.
    demands_attention_mask: bool,
}

#[cfg(feature = "onnx")]
impl Siglip2TextOnnxBackend {
    /// Loads `onnx/text_model.onnx` from a model directory (the
    /// arm-of-record's published repository layout).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BackendFailed`] when the session cannot be
    /// built, the graph cannot be loaded, or the graph's signature is
    /// not the split text tower's.
    pub fn from_dir(directory: &std::path::Path) -> Result<Self, RuntimeError> {
        Self::from_file(&directory.join(TEXT_GRAPH_FILE))
    }

    /// Loads a text tower from an explicit graph path.
    ///
    /// The `model_id` carries the file name, so two different exports
    /// never share a decision cache key (D6).
    ///
    /// # Errors
    ///
    /// [`RuntimeError::BackendFailed`] as for [`from_dir`](Self::from_dir),
    /// plus a typed refusal when the graph's signature is foreign.
    pub fn from_file(path: &std::path::Path) -> Result<Self, RuntimeError> {
        let graph = path
            .file_name()
            .map_or_else(|| TEXT_GRAPH_FILE.to_owned(), |name| name.to_string_lossy().to_string());
        let service_id = format!("{MODEL_ID}:text");
        let session = create_session(&service_id, path)?;
        let demanded = session_input_names(&session);
        if let Some(reason) = input_name_mismatch("text", &demanded, &[INPUT_IDS]) {
            return Err(RuntimeError::BackendFailed { model_id: service_id, message: reason });
        }
        let demands_attention_mask = demanded.iter().any(|name| name == ATTENTION_MASK);
        Ok(Self {
            session: std::sync::Mutex::new(session),
            model_id: format!("{service_id}:{graph}"),
            demands_attention_mask,
        })
    }
}

#[cfg(feature = "onnx")]
impl crate::backend::InferenceBackend for Siglip2TextOnnxBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn infer(
        &self,
        inputs: &std::collections::BTreeMap<String, DenseTensor>,
    ) -> Result<std::collections::BTreeMap<String, DenseTensor>, RuntimeError> {
        let (batch, length, mask) = validate_id_inputs(inputs)?;
        if self.demands_attention_mask && mask.is_none() {
            return Err(RuntimeError::MissingInput { name: ATTENTION_MASK.to_owned() });
        }
        let ids = &inputs[INPUT_IDS];
        let tokens = crate::kai::integral_i64(ids, INPUT_IDS)?;
        let mut guard = self.session.lock().map_err(|_| RuntimeError::BackendFailed {
            model_id: self.model_id.clone(),
            message: "session mutex poisoned by a panicking thread".to_owned(),
        })?;
        let embeddings =
            run_text_session(&mut guard, &self.model_id, batch, length, &tokens, mask.as_deref())?;
        if embeddings.shape()[0] != batch {
            return Err(RuntimeError::BackendFailed {
                model_id: self.model_id.clone(),
                message: format!(
                    "`{POOLER_OUTPUT}` batch {} must equal the input batch {batch}",
                    embeddings.shape()[0]
                ),
            });
        }
        Ok(std::collections::BTreeMap::from([(POOLER_OUTPUT.to_owned(), embeddings)]))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn wire_names_are_the_published_graph_names() {
        assert_eq!(PIXEL_VALUES, "pixel_values");
        assert_eq!(POOLER_OUTPUT, "pooler_output");
        assert_eq!(INPUT_IDS, "input_ids");
        assert_eq!(ATTENTION_MASK, "attention_mask");
        assert_eq!(VISION_GRAPH_FILE, "onnx/vision_model.onnx");
        assert_eq!(TEXT_GRAPH_FILE, "onnx/text_model.onnx");
        // Distinct from every other backend family sharing a cache (D6).
        assert_eq!(crate::kai::MODEL_ID, "kai-0.6b-onnx");
        assert_eq!(crate::nli::MODEL_ID, "nli-zeroshot");
        assert_eq!(MODEL_ID, "siglip2");
    }

    #[test]
    fn vision_input_mismatch_reasons_name_the_offender() {
        let match_ok = input_name_mismatch("vision", &[PIXEL_VALUES.to_owned()], &[PIXEL_VALUES]);
        assert_eq!(match_ok, None);
        let missing = input_name_mismatch("vision", &[], &[PIXEL_VALUES]);
        assert!(missing.unwrap().contains("pixel_values"));
        let foreign = input_name_mismatch(
            "vision",
            &["logits".to_owned(), PIXEL_VALUES.to_owned()],
            &[PIXEL_VALUES],
        );
        assert!(foreign.unwrap().contains("logits"));
    }

    #[test]
    fn pixel_validation_refuses_every_malformed_named_input() {
        let rank3 = DenseTensor::new(vec![1, 4, 4], vec![0.0; 16]).unwrap();
        let bad_rank = std::collections::BTreeMap::from([(PIXEL_VALUES.to_owned(), rank3)]);
        let error = validate_pixel_values(&bad_rank).unwrap_err();
        assert_eq!(error.code(), "runtime.invalid_tensor", "{error}");
        assert!(error.to_string().contains("rank 4"), "{error}");

        let four_channel = DenseTensor::new(vec![1, 4, 4, 4], vec![0.0; 64]).unwrap();
        let bad_channels =
            std::collections::BTreeMap::from([(PIXEL_VALUES.to_owned(), four_channel)]);
        let error = validate_pixel_values(&bad_channels).unwrap_err();
        assert!(error.to_string().contains("3 (RGB)"), "{error}");

        let empty = std::collections::BTreeMap::new();
        let error = validate_pixel_values(&empty).unwrap_err();
        assert_eq!(error.code(), "runtime.missing_input", "{error}");
    }

    #[test]
    fn id_validation_refuses_malformed_rows_and_masks() {
        let rank3 = DenseTensor::new(vec![1, 4, 4], vec![0.0; 16]).unwrap();
        let bad_rank = std::collections::BTreeMap::from([(INPUT_IDS.to_owned(), rank3)]);
        let error = validate_id_inputs(&bad_rank).unwrap_err();
        assert!(error.to_string().contains("rank 2"), "{error}");

        let valid_ids = DenseTensor::new(vec![1, 4], vec![1.0; 4]).unwrap();
        let bad_mask = DenseTensor::new(vec![1, 5], vec![1.0; 5]).unwrap();
        let bad_shape = std::collections::BTreeMap::from([
            (INPUT_IDS.to_owned(), valid_ids),
            (ATTENTION_MASK.to_owned(), bad_mask),
        ]);
        let error = validate_id_inputs(&bad_shape).unwrap_err();
        assert!(error.to_string().contains(ATTENTION_MASK), "{error}");

        let good = std::collections::BTreeMap::from([(
            INPUT_IDS.to_owned(),
            DenseTensor::new(vec![2, 4], vec![1.0; 8]).unwrap(),
        )]);
        let (batch, length, mask) = validate_id_inputs(&good).unwrap();
        assert_eq!((batch, length), (2, 4));
        assert_eq!(mask, None);
    }

    #[test]
    fn nonexistent_graphs_fail_with_typed_backend_errors() {
        // Only meaningful when the feature is compiled: the loader must
        // refuse a missing file as a `BackendFailed`, never a panic.
        #[cfg(feature = "onnx")]
        {
            let error = Siglip2VisionOnnxBackend::from_file(std::path::Path::new(
                "nonexistent/vision_model.onnx",
            ))
            .unwrap_err();
            assert_eq!(error.code(), "runtime.backend_failed", "{error}");
            let error = Siglip2TextOnnxBackend::from_file(std::path::Path::new(
                "nonexistent/text_model.onnx",
            ))
            .unwrap_err();
            assert_eq!(error.code(), "runtime.backend_failed", "{error}");
        }
    }
}
