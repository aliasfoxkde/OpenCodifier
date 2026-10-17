//! The serving embedder for the `SigLIP2` zero-shot vision rung:
//! image preprocessing, prompt encoding, and the split-tower ONNX
//! graphs behind one shared [`ImageTextEmbedder`] seam (track V,
//! phase G).
//!
//! Layering (D7, D11): decoding, resizing, and normalization happen
//! here in Rust; the runtime's graph runners are pure embedding
//! machines (`pixel_values` → `pooler_output` rows); and the decision
//! math — L2 renormalization, the zero-shot head, softmax — stays in
//! the contract module (`vision.rs`), which carries no `ort` and never
//! will: the heavyweight runtime is confined to `opencodifier-runtime`.
//!
//! Weights are never committed to this workspace (D14): the caller
//! names a locally pulled model directory, and every artifact's bytes
//! are SHA-256-verified against the manifest beside it before a
//! session is created.

use std::collections::BTreeMap;
use std::path::Path;

use opencode_core::{ImageInput, ImageMediaType};
use opencode_runtime::{
    ATTENTION_MASK, DenseTensor, INPUT_IDS, InferenceBackend, PIXEL_VALUES, POOLER_OUTPUT,
    Siglip2TextOnnxBackend, Siglip2VisionOnnxBackend, TEXT_GRAPH_FILE, VISION_GRAPH_FILE,
};

use crate::error::ModelError;
use crate::manifest::ModelManifest;
use crate::vision::ImageTextEmbedder;
use crate::vision::config_from_dir_json;

/// `ImageNet` channel means, in [0, 1] normalized pixel space — the
/// `image_mean` the arm of record's `preprocessor_config.json`
/// publishes.
pub const IMAGE_MEAN: [f32; 3] = [0.485, 0.456, 0.406];

/// `ImageNet` channel standard deviations (`image_std`), the divisor of
/// the mean-subtracted channel.
pub const IMAGE_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// The text-token budget per prompt, in tokens, before encoding
/// truncates. The arm of record's text export pipelines tokenize
/// prompts at exactly this `max_length`; longer prompts would shift
/// the pointer geometry the readout was never measured against.
pub const MAX_TEXT_TOKENS: usize = 64;

/// The tokenizer asset in a verified model directory.
pub const TOKENIZER_FILE: &str = "tokenizer.json";

/// The config asset in a verified model directory.
pub const CONFIG_FILE: &str = "config.json";

/// The real `SigLIP2` embedder: the locally fetched arm of record's
/// two split-tower graphs plus its own `tokenizer.json`.
///
/// Images are decoded, resized so the shortest edge covers the
/// configured square, center-cropped, mean/std-normalized in CHW
/// order, and sent as one `[B, 3, side, side]` tensor. Prompts are
/// encoded through the directory tokenizer (truncated at
/// [`MAX_TEXT_TOKENS`]) and collated to the longest row with the
/// attention mask — which is supplied to the graph only when the
/// graph actually demands it. Outputs are `pooler_output` rows in
/// request order, unnormalized; the zero-shot head above this module
/// renormalizes defensively.
#[derive(Debug)]
pub struct OnnxSiglip2Embedder {
    vision: Box<dyn InferenceBackend + Send + Sync>,
    text: Box<dyn InferenceBackend + Send + Sync>,
    /// Whether the loaded text graph demands `attention_mask`.
    demands_attention_mask: bool,
    tokenizer: tokenizers::Tokenizer,
    /// The pad token id collated batches pad shorter rows with.
    /// Readout-neutral: the mask zeroes every pad position.
    pad_id: u32,
    image_side: usize,
    dims: usize,
    /// Tower roster folded into one model id: either graph changing
    /// changes every cache key (D6).
    model_id: String,
}

impl OnnxSiglip2Embedder {
    /// Loads the embedder from a locally fetched model directory.
    ///
    /// Every artifact — the vision graph, the text graph, the
    /// tokenizer, and the config — is verified against its SHA-256
    /// manifest before anything loads, and each graph manifest's model
    /// id must match the id the loaded backend reports. The geometry
    /// (`image_size`, `hidden_size`) comes from the verified
    /// `config.json`.
    ///
    /// # Errors
    ///
    /// [`ModelError::UnreadableArtifact`] when an artifact or manifest
    /// cannot be read, [`ModelError::InvalidManifest`]/[
    /// `ModelError::Sha256Mismatch`] when a manifest is invalid,
    /// unreadable, or disagrees with the bytes on disk, and
    /// [`ModelError::ContractViolation`] when a manifest's model id
    /// mismatches the loaded backend or a graph or config fails to
    /// load.
    pub fn from_dir(directory: &Path) -> Result<Self, ModelError> {
        let vision_path = directory.join(VISION_GRAPH_FILE);
        let text_path = directory.join(TEXT_GRAPH_FILE);
        let tokenizer_path = directory.join(TOKENIZER_FILE);
        let config_path = directory.join(CONFIG_FILE);
        // The D14 gate: digests are computed from the bytes on disk,
        // never taken on trust from anywhere.
        for path in [&vision_path, &text_path, &tokenizer_path, &config_path] {
            verify_manifest(path)?;
        }

        let config_json = std::fs::read_to_string(&config_path).map_err(|error| {
            ModelError::UnreadableArtifact {
                artifact: config_path.clone(),
                reason: error.to_string(),
            }
        })?;
        let (_head, image_side, dims) = config_from_dir_json(&config_json)?;
        let tokenizer = load_tokenizer(&tokenizer_path)?;

        let vision = Siglip2VisionOnnxBackend::from_dir(directory).map_err(|error| {
            ModelError::ContractViolation { reason: format!("vision graph load failed: {error}") }
        })?;
        let text = Siglip2TextOnnxBackend::from_dir(directory).map_err(|error| {
            ModelError::ContractViolation { reason: format!("text graph load failed: {error}") }
        })?;
        // The manifest's stated identity must be the loaded backend's
        // identity: a verified artifact that serves under a different
        // name is a cache-key trap (D6), never a rename to paper over.
        for (path, backend_id) in [(&vision_path, vision.model_id()), (&text_path, text.model_id())]
        {
            let manifest = ModelManifest::load(&manifest_for(path))?;
            if manifest.model_id != backend_id {
                return Err(ModelError::InvalidManifest {
                    manifest: manifest.model_id.clone(),
                    reason: format!("manifest id does not match the loaded backend `{backend_id}`"),
                });
            }
        }

        Ok(Self {
            demands_attention_mask: text.demands_attention_mask(),
            model_id: format!("{}+{}", vision.model_id(), text.model_id()),
            pad_id: pad_of(&tokenizer),
            dims,
            image_side,
            tokenizer,
            text: Box::new(text),
            vision: Box::new(vision),
        })
    }

    /// Loads from already-verified parts — the parity harness's and
    /// unit tests' seam, exactly as the NLI scorer's `from_parts`.
    /// Nothing is re-fetched or re-verified here; the caller owns that.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when `tokenizer_json` is not a
    /// loadable tokenizer.
    pub fn from_parts(
        vision: Box<dyn InferenceBackend + Send + Sync>,
        text: Box<dyn InferenceBackend + Send + Sync>,
        demands_attention_mask: bool,
        tokenizer_json: &str,
        image_side: usize,
        dims: usize,
    ) -> Result<Self, ModelError> {
        use std::str::FromStr;
        let tokenizer = tokenizers::Tokenizer::from_str(tokenizer_json)
            .map_err(|error| ModelError::Tokenizer { reason: format!("load failed: {error}") })?;
        Ok(Self {
            model_id: format!("{}+{}", vision.model_id(), text.model_id()),
            demands_attention_mask,
            pad_id: tokenizer.get_padding().map_or(0, |padding| padding.pad_id),
            dims,
            image_side,
            tokenizer,
            text,
            vision,
        })
    }

    /// The square the preprocessing produces before it reaches the
    /// vision graph (`config.json`'s `image_size`).
    #[must_use]
    pub fn image_side(&self) -> usize {
        self.image_side
    }
}

/// Verifies one artifact against the manifest beside it.
///
/// # Errors
///
/// Whatever [`ModelManifest::load`] or [
/// `ModelManifest::verify_artifact`] reports.
fn verify_manifest(path: &Path) -> Result<(), ModelError> {
    let manifest = ModelManifest::load(&manifest_for(path))?;
    manifest.verify_artifact(path)
}

/// The conventional manifest path beside an artifact.
fn manifest_for(path: &Path) -> std::path::PathBuf {
    ModelManifest::manifest_path_for(path)
}

/// Loads a `tokenizer.json`, truncating past [`MAX_TEXT_TOKENS`] and
/// keeping the tokenizer's own padding strategy off — collation is this
/// module's business, so the tokenizer cannot disagree with it (the
/// NLI scorer's precedent).
///
/// # Errors
///
/// [`ModelError::Tokenizer`] when the file is unreadable or the JSON
/// is not a loadable tokenizer.
fn load_tokenizer(path: &Path) -> Result<tokenizers::Tokenizer, ModelError> {
    use std::str::FromStr;
    let json = std::fs::read_to_string(path).map_err(|error| ModelError::Tokenizer {
        reason: format!("reading `{}` failed: {error}", path.display()),
    })?;
    let mut tokenizer = tokenizers::Tokenizer::from_str(&json)
        .map_err(|error| ModelError::Tokenizer { reason: format!("load failed: {error}") })?;
    tokenizer
        .with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_TEXT_TOKENS,
            ..tokenizers::TruncationParams::default()
        }))
        .map_err(|error| ModelError::Tokenizer {
            reason: format!("truncation setup failed: {error}"),
        })?;
    tokenizer.with_padding(None);
    Ok(tokenizer)
}

/// The declared pad id (0 when the tokenizer publishes none — pad
/// positions are masked either way).
fn pad_of(tokenizer: &tokenizers::Tokenizer) -> u32 {
    tokenizer.get_padding().map_or(0, |padding| padding.pad_id)
}

/// Decodes, resizes so the shortest edge covers `side`, center-crops
/// square, and mean/std-normalizes into CHW float order — the exact
/// tensor the vision graph consumes, pinned by the parity fixture
/// (track I).
///
/// The resize filter is `Triangle`
/// (`image::imageops::FilterType::Triangle`, the crate's
/// bilinear-style resampler); the crop is the centered widest window;
/// the channel order is R, G, B; every channel is `(pixel − mean) /
/// std`. Deterministic integer geometry throughout.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the bytes do not decode as
/// the declared media type, or the scaled geometry overflows.
fn preprocess(image: &ImageInput, side: usize) -> Result<Vec<f32>, ModelError> {
    let format = match image.media_type() {
        ImageMediaType::Png => image::ImageFormat::Png,
        ImageMediaType::Jpeg => image::ImageFormat::Jpeg,
    };
    let decoded = image::load_from_memory_with_format(image.data(), format).map_err(|error| {
        ModelError::ContractViolation {
            reason: format!("image decode failed ({}): {error}", image.media_type().as_str()),
        }
    })?;
    // Decoders may hand back RGBA/alpha channel variants; the towers
    // see RGB only, so this is the single canonicalization point
    // (alpha compositing is the caller's business, not preprocessing's).
    let decoded = decoded.to_rgb8();
    let (width, height) = decoded.dimensions();
    // Shortest edge → `side`; the longer edge follows aspect, with
    // rounding half-up in u64 so the geometry is platform-stable.
    let (width_u64, height_u64, side_u64) = (
        u64::from(width),
        u64::from(height),
        u64::try_from(side).map_err(|_| ModelError::ContractViolation {
            reason: "image side does not fit usize".to_owned(),
        })?,
    );
    let scaled_long = if width <= height {
        scale(height_u64, width_u64, side_u64)?
    } else {
        scale(width_u64, height_u64, side_u64)?
    };
    let (scaled_width_u64, scaled_height_u64) =
        if width <= height { (side_u64, scaled_long) } else { (scaled_long, side_u64) };
    // `scaled_w/h >= side` by construction: the short side scaled to
    // exactly `side`, the long edge never below it — and geometry is
    // bounded at config time to fit u32.
    let scaled_width =
        u32::try_from(scaled_width_u64).map_err(|_| ModelError::ContractViolation {
            reason: format!("scaled image width {scaled_width_u64} does not fit u32"),
        })?;
    let scaled_height =
        u32::try_from(scaled_height_u64).map_err(|_| ModelError::ContractViolation {
            reason: format!("scaled image height {scaled_height_u64} does not fit u32"),
        })?;
    let side = u32::try_from(side).map_err(|_| ModelError::ContractViolation {
        reason: "image side does not fit u32".to_owned(),
    })?;
    let resized = image::imageops::resize(
        &decoded,
        scaled_width,
        scaled_height,
        image::imageops::FilterType::Triangle,
    );
    let crop_x = scaled_width.saturating_sub(side) / 2;
    let crop_y = scaled_height.saturating_sub(side) / 2;
    let cropped = image::imageops::crop_imm(&resized, crop_x, crop_y, side, side);
    let rgb = cropped.to_image();
    // Pixels arrive HWC row-major; the planes below transpose them
    // into CHW without any u32 casts (every index is usize math over
    // a side actually present in the buffer).
    let hwc: Vec<[u8; 3]> = rgb.pixels().map(|pixel| pixel.0).collect();
    let side = side as usize;

    let mut pixels = Vec::with_capacity(3 * side * side);
    for channel in 0..3 {
        for row in 0..side {
            for column in 0..side {
                let value = f32::from(hwc[row * side + column][channel]);
                pixels.push((value / 255.0 - IMAGE_MEAN[channel]) / IMAGE_STD[channel]);
            }
        }
    }
    Ok(pixels)
}

/// Scales the longer edge by `target / shorter` with rounding half-up
/// in integer arithmetic.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the u64 arithmetic
/// overflows (an image whose scaled edge exceeds u64 is not an input
/// any tower sessions).
fn scale(longer: u64, shorter: u64, target: u64) -> Result<u64, ModelError> {
    longer
        .checked_mul(target)
        .and_then(|scaled| scaled.checked_add(shorter / 2))
        .and_then(|biased| biased.checked_div(shorter))
        .ok_or_else(|| ModelError::ContractViolation {
            reason: format!("image edge {longer} does not scale to target {target}"),
        })
}

impl OnnxSiglip2Embedder {
    /// The `pooler_output` rows of a two-tower call, in request order,
    /// with the width contract checked against this embedder's dims.
    fn pooled_rows(
        outputs: &BTreeMap<String, DenseTensor>,
        tower: &str,
        batch: usize,
        dims: usize,
    ) -> Result<Vec<Vec<f32>>, ModelError> {
        let embeddings =
            outputs.get(POOLER_OUTPUT).ok_or_else(|| ModelError::ContractViolation {
                reason: format!("the {tower} graph did not return `{POOLER_OUTPUT}`"),
            })?;
        if embeddings.shape().first() != Some(&batch) {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "`{POOLER_OUTPUT}` must return {batch} rows, got first shape {:?}",
                    embeddings.shape()
                ),
            });
        }
        let width = embeddings.shape().get(1).copied().unwrap_or_default();
        if width != dims {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "`{POOLER_OUTPUT}` row width {width} does not match the \
                 config's hidden_size {dims}"
                ),
            });
        }
        (0..batch)
            .map(|row| {
                embeddings.row(row).map(<[f32]>::to_vec).ok_or_else(|| {
                    ModelError::ContractViolation {
                        reason: format!("`{POOLER_OUTPUT}` lost row {row} of {batch}"),
                    }
                })
            })
            .collect()
    }
}

impl ImageTextEmbedder for OnnxSiglip2Embedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn embed_images(&self, images: &[ImageInput]) -> Result<Vec<Vec<f32>>, ModelError> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        let side = self.image_side;
        let batch = images.len();
        let mut pixels = Vec::with_capacity(batch * 3 * side * side);
        for (index, image) in images.iter().enumerate() {
            let plane = preprocess(image, side).map_err(|error| match error {
                ModelError::ContractViolation { reason } => {
                    ModelError::ContractViolation { reason: format!("image {index}: {reason}") }
                }
                other => other,
            })?;
            pixels.extend(plane);
        }
        let tensors = BTreeMap::from([(
            PIXEL_VALUES.to_owned(),
            DenseTensor::new(vec![batch, 3, side, side], pixels).map_err(|error| {
                ModelError::ContractViolation { reason: format!("`pixel_values`: {error}") }
            })?,
        )]);
        let outputs = self.vision.infer(&tensors).map_err(|error| {
            ModelError::ContractViolation { reason: format!("vision graph failed: {error}") }
        })?;
        Self::pooled_rows(&outputs, "image", batch, self.dims)
    }

    fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encodings = texts
            .iter()
            .map(|text| {
                self.tokenizer.encode(*text, true).map_err(|error| ModelError::Tokenizer {
                    reason: format!("prompt encode failed: {error}"),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let width = encodings.iter().map(|encoding| encoding.get_ids().len()).max().unwrap_or(1);
        if width == 0 {
            return Err(ModelError::ContractViolation {
                reason: "a prompt encoded to zero tokens; refusing an empty row".to_owned(),
            });
        }
        let batch = encodings.len();
        let mut input_ids = Vec::with_capacity(batch * width);
        let mut attention_mask = Vec::with_capacity(batch * width);
        for encoding in &encodings {
            let ids = encoding.get_ids();
            // Token ids and mask values ride the Kai f32-tensor
            // convention: far below 2^24, where f32 is exact.
            #[allow(clippy::cast_precision_loss)]
            let to_f32 = |value: &u32| *value as f32;
            input_ids.extend(ids.iter().map(to_f32));
            #[allow(clippy::cast_precision_loss)]
            let pad = self.pad_id as f32;
            input_ids.extend(std::iter::repeat_n(pad, width - ids.len()));
            let mask = encoding.get_attention_mask();
            attention_mask.extend(mask.iter().map(to_f32));
            attention_mask.extend(std::iter::repeat_n(0.0_f32, width - mask.len()));
        }
        let mut tensors = BTreeMap::from([(
            INPUT_IDS.to_owned(),
            DenseTensor::new(vec![batch, width], input_ids).map_err(|error| {
                ModelError::ContractViolation { reason: format!("`input_ids`: {error}") }
            })?,
        )]);
        if self.demands_attention_mask {
            tensors.insert(
                ATTENTION_MASK.to_owned(),
                DenseTensor::new(vec![batch, width], attention_mask).map_err(|error| {
                    ModelError::ContractViolation { reason: format!("`attention_mask`: {error}") }
                })?,
            );
        }
        let outputs = self.text.infer(&tensors).map_err(|error| ModelError::ContractViolation {
            reason: format!("text graph failed: {error}"),
        })?;
        Self::pooled_rows(&outputs, "text", batch, self.dims)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::kai::FIXTURE_TOKENIZER;
    use opencode_runtime::MockInferenceBackend;
    use sha2::{Digest, Sha256};

    /// Encodes a patterned image into PNG bytes — an analytical
    /// fixture built at test time, the same discipline as the
    /// byte-level BPE oracle: nothing ships, everything is computed.
    fn png_bytes(
        width: usize,
        height: usize,
        color: impl Fn(usize, usize) -> [u8; 3],
    ) -> image::ImageResult<Vec<u8>> {
        let mut buffer = image::RgbImage::new(
            u32::try_from(width).expect("test images stay tiny"),
            u32::try_from(height).expect("test images stay tiny"),
        );
        for (x, y, slot) in buffer.enumerate_pixels_mut() {
            *slot = image::Rgb(color(x as usize, y as usize));
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(buffer)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)?;
        Ok(bytes)
    }

    fn png_input(bytes: Vec<u8>) -> ImageInput {
        ImageInput::new(ImageMediaType::Png, bytes)
    }

    #[test]
    fn solid_white_normalizes_to_the_image_net_constants() {
        let bytes = png_bytes(4, 4, |_, _| [255_u8, 255, 255]).unwrap();
        let plane = preprocess(&png_input(bytes), 4).unwrap();
        let per_channel: [f32; 3] =
            std::array::from_fn(|channel| (1.0 - IMAGE_MEAN[channel]) / IMAGE_STD[channel]);
        assert_eq!(plane.len(), 3 * 4 * 4);
        // CHW: channel c's plane is every value with index
        // c*side*side + row*side + col, and all are the channel's
        // normalized constant.
        for (channel, expected) in per_channel.iter().enumerate() {
            let offset = channel * 4 * 4;
            for slot in &plane[offset..offset + 4 * 4] {
                assert_eq!(slot, expected);
            }
        }
    }

    #[test]
    fn wide_images_center_crop() {
        // 8x4 with a red block spanning x 4..8 (the rest white): the
        // shortest edge is height 4 with side 4, so no resize, and the
        // centered 4x4 crop sits at x 2..6 — half white, half red. The
        // green channel separates them (white = 1, red = 0).
        let bytes =
            png_bytes(8, 4, |x, _| if x >= 4 { [255, 0, 0] } else { [255, 255, 255] }).unwrap();
        let plane = preprocess(&png_input(bytes), 4).unwrap();
        let green = |column: usize| plane[4 * 4 + column];
        let crop_x = (8 - 4) / 2;
        assert_eq!(green(0), (1.0 - IMAGE_MEAN[1]) / IMAGE_STD[1]);
        assert_eq!(green(crop_x - 1), (1.0 - IMAGE_MEAN[1]) / IMAGE_STD[1]);
        assert_eq!(green(crop_x), (0.0 - IMAGE_MEAN[1]) / IMAGE_STD[1]);
        assert_eq!(green(3), (0.0 - IMAGE_MEAN[1]) / IMAGE_STD[1]);
    }

    #[test]
    fn tall_images_scale_the_longer_edge_then_crop() {
        // 4x8 with a blue block spanning y 4..8 (the rest white): the
        // shortest edge is width 4 with side 4, so no resize, and the
        // centered 4x4 crop takes rows 2..6 — half white, half blue.
        // The red channel separates them (white = 1, blue = 0).
        let bytes =
            png_bytes(4, 8, |_, y| if y >= 4 { [0, 0, 255] } else { [255, 255, 255] }).unwrap();
        let plane = preprocess(&png_input(bytes), 4).unwrap();
        let red = |row: usize| plane[row * 4];
        // Crop row 1 is source row 3 (white), crop row 2 is source
        // row 4 (blue).
        assert_eq!(red(1), (1.0 - IMAGE_MEAN[0]) / IMAGE_STD[0]);
        assert_eq!(red(2), (0.0 - IMAGE_MEAN[0]) / IMAGE_STD[0]);
        assert_eq!(plane.len(), 3 * 4 * 4);
    }

    #[test]
    fn decode_failure_is_typed_and_carries_the_media_type() {
        let bytes: Vec<u8> = Vec::from(b"not a png");
        let error = preprocess(&png_input(bytes), 4).unwrap_err();
        assert!(error.to_string().contains("image/png"));
        assert!(error.to_string().contains("decode failed"));
    }

    #[test]
    fn rows_wider_or_narrower_than_the_config_dims_are_refused() {
        let outputs = BTreeMap::from([(
            POOLER_OUTPUT.to_owned(),
            DenseTensor::new(vec![2, 4], vec![0.0; 8]).unwrap(),
        )]);
        let error = OnnxSiglip2Embedder::pooled_rows(&outputs, "text", 2, 768).unwrap_err();
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn missing_pooler_output_is_typed() {
        let outputs = BTreeMap::from([(
            "wrong_output".to_owned(),
            DenseTensor::new(vec![1, 768], vec![0.0; 768]).unwrap(),
        )]);
        let error = OnnxSiglip2Embedder::pooled_rows(&outputs, "image", 1, 768).unwrap_err();
        assert!(error.to_string().contains("pooler_output"));
    }

    #[test]
    fn empty_slices_return_empty_without_reaching_a_backend() {
        let vision = MockInferenceBackend::new(
            "mock:vision",
            // The embedder must return before the backend is touched,
            // so the output map is never consulted (and a zero-batch
            // tensor is not representable in the runtime).
            BTreeMap::new(),
        )
        .with_required_inputs(&[PIXEL_VALUES]);
        let text = MockInferenceBackend::new("mock:text", BTreeMap::new())
            .with_required_inputs(&[INPUT_IDS]);
        let embedder = OnnxSiglip2Embedder::from_parts(
            Box::new(vision),
            Box::new(text),
            true,
            FIXTURE_TOKENIZER,
            4,
            4,
        )
        .unwrap();
        assert_eq!(embedder.embed_images(&[] as &[ImageInput]).unwrap(), Vec::<Vec<f32>>::new());
        assert_eq!(embedder.embed_texts(&[] as &[&str]).unwrap(), Vec::<Vec<f32>>::new());
    }

    #[test]
    fn embed_texts_collates_with_mask_on_a_demanding_graph() {
        // The mock answers every call with the configured row; what
        // this pins is the call contract: `input_ids` AND
        // `attention_mask` on the wire when the graph demands them
        // (absence is a typed MissingInput), and two rows out.
        let outputs = BTreeMap::from([(
            POOLER_OUTPUT.to_owned(),
            DenseTensor::new(vec![2, 4], vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap(),
        )]);
        let text = MockInferenceBackend::new("mock:text", outputs)
            .with_required_inputs(&[INPUT_IDS, ATTENTION_MASK]);
        let embedder = OnnxSiglip2Embedder::from_parts(
            Box::new(MockInferenceBackend::new("mock:vision", BTreeMap::new())),
            Box::new(text),
            true,
            FIXTURE_TOKENIZER,
            4,
            4,
        )
        .unwrap();
        let vectors = embedder.embed_texts(&["bbb", "&"]).unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0][0], 1.0);
        assert_eq!(vectors[1][0], 0.0);
        assert_eq!(embedder.model_id(), "mock:vision+mock:text");
    }

    #[test]
    fn mask_is_supplied_only_when_the_graph_demands_it() {
        // demands_attention_mask = false: feeding a mask to a graph
        // that never asked for one is an unknown-feed error at the
        // session, so this mock requires exactly `input_ids` on the
        // wire and nothing more.
        let outputs = BTreeMap::from([(
            POOLER_OUTPUT.to_owned(),
            DenseTensor::new(vec![1, 4], vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
        )]);
        let text =
            MockInferenceBackend::new("mock:text", outputs).with_required_inputs(&[INPUT_IDS]);
        let embedder = OnnxSiglip2Embedder::from_parts(
            Box::new(MockInferenceBackend::new("mock:vision", BTreeMap::new())),
            Box::new(text),
            false,
            FIXTURE_TOKENIZER,
            4,
            4,
        )
        .unwrap();
        assert_eq!(embedder.embed_texts(&["aa"]).unwrap().len(), 1);
        // The fixture tokenizer must carry a nonempty pad-free config
        // here — this pins that the fixture loads at all through the
        // production seam.
        assert_eq!(embedder.image_side(), 4);
        assert_eq!(embedder.dims(), 4);
    }

    #[test]
    fn manifests_refuse_a_digest_mismatch() {
        let dir = std::env::temp_dir().join("oc-vision-manifest-test");
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("graph.onnx");
        std::fs::write(&artifact, b"actual bytes").unwrap();
        let digest = format!("{:x}", Sha256::new().chain_update(b"other bytes").finalize());
        let manifest = ModelManifest {
            model_id: "siglip2:vision:graph.onnx".to_owned(),
            sha256: digest,
            format: "onnx".to_owned(),
            revision: "r2026.10.1".to_owned(),
        };
        std::fs::write(
            ModelManifest::manifest_path_for(&artifact),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_manifest(&artifact).is_err());
    }

    #[test]
    fn manifests_accept_their_exact_bytes() {
        let dir = std::env::temp_dir().join("oc-vision-manifest-test-ok");
        std::fs::create_dir_all(&dir).unwrap();
        let artifact = dir.join("tokenizer.json");
        std::fs::write(&artifact, FIXTURE_TOKENIZER).unwrap();
        let digest = format!("{:x}", Sha256::new().chain_update(FIXTURE_TOKENIZER).finalize());
        let manifest = ModelManifest {
            model_id: "siglip2:text:tokenizer.json".to_owned(),
            sha256: digest,
            format: "json".to_owned(),
            revision: "r2026.10.1".to_owned(),
        };
        std::fs::write(
            ModelManifest::manifest_path_for(&artifact),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_manifest(&artifact).is_ok());
    }
}
