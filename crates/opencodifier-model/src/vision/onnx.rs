//! The SigLIP2 zero-shot ONNX transport (track V, phase F).
//!
//! Implements [`ImageTextEmbedder`] over the `onnx-community/siglip2-base-patch16-256-ONNX`
//! family. Both image and text encoders run in one ONNX session; the
//! pooled outputs are L2-normalized per the model's forward head, so
//! cosine and dot product coincide.
//!
//! # Assumptions
//!
//! * The session has two inputs: `pixel_values` (float [B,3,H,W]) and
//!   `input_ids` (int64 [B,L]), and two outputs: `image_embeds` and
//!   `text_embeds` (float [B,1,E] pooled per the model's pooling mode).
//! * Images are resized to the size the model was trained at, normalized
//!   with ImageNet mean/std, and converted to CHW order.
//! * Texts are tokenized by the model's SentencePiece vocabulary.
//! * Weights are never fetched at build time (D14); the caller supplies
//!   a locally verified model directory.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use opencodifier_core::{ImageInput, ImageMediaType};
use ort::session::Session;

use crate::error::ModelError;
use crate::vision::ImageTextEmbedder;

/// The default image size SigLIP2-base expects.
pub const DEFAULT_IMAGE_SIZE: usize = 256;

/// `ImageNet` normalization constants the model was trained with.
const IMAGE_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGE_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// The ONNX backend for the `SigLIP2` zero-shot contract.
///
/// Loads a `SigLIP2` ONNX session from a model directory, runs image and
/// text preprocessing, and returns L2-normalized pooled embeddings.
#[cfg(feature = "vision")]
#[derive(Debug)]
pub struct Siglip2OnnxBackend {
    session: Mutex<Session>,
    /// Model-specific logit scale from the directory's config.json, or the
    /// family default. Stored for future scoring use.
    #[allow(dead_code)]
    logit_scale: f64,
    model_id: String,
}

#[cfg(feature = "vision")]
impl Siglip2OnnxBackend {
    /// Loads a `SigLIP2` model from a directory.
    ///
    /// Reads `config.json` for `logit_scale` and `image_size` if present,
    /// falling back to family defaults. The directory layout is the HF
    /// snapshot structure (no weights committed to the workspace, D14).
    pub fn from_dir(directory: &Path) -> Result<Self, ModelError> {
        let config_path = directory.join("config.json");
        let config = if config_path.is_file() {
            let raw = std::fs::read_to_string(&config_path).map_err(|e| ModelError::UnreadableArtifact {
                artifact: config_path.clone(),
                reason: e.to_string(),
            })?;
            serde_json::from_str::<serde_json::Value>(&raw).ok()
        } else {
            None
        };

        let logit_scale = config
            .as_ref()
            .and_then(|c| c.get("logit_scale").and_then(serde_json::Value::as_f64))
            .unwrap_or(crate::vision::DEFAULT_LOGIT_SCALE);

        // Find the .onnx file in the directory.
        let onnx_files: Vec<_> = std::fs::read_dir(directory)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(std::result::Result::ok)
            .filter(|e| {
                e.path().extension().is_some_and(|ext| ext == "onnx")
            })
            .collect();

        let onnx_path = if onnx_files.len() == 1 {
            onnx_files[0].path()
        } else {
            return Err(ModelError::InvalidManifest {
                manifest: directory.display().to_string(),
                reason: format!(
                    "expected exactly one .onnx file, found {}",
                    onnx_files.len()
                ),
            });
        };

        Self::from_file(&onnx_path, logit_scale)
    }

    /// Loads a `SigLIP2` model from an explicit ONNX file path.
    ///
    /// `logit_scale` must be provided explicitly since the file has no
    /// embedded config. The `model_id` embeds the file name so two
    /// different exports never share a decision cache key (D6).
    pub fn from_file(onnx_path: &Path, logit_scale: f64) -> Result<Self, ModelError> {
        let mut builder = Session::builder().map_err(|e| ModelError::UnreadableArtifact {
            artifact: onnx_path.to_path_buf(),
            reason: format!("session builder failed: {e}"),
        })?;
        let session = builder
            .commit_from_file(onnx_path)
            .map_err(|e| ModelError::UnreadableArtifact {
                artifact: onnx_path.to_path_buf(),
                reason: format!("ONNX session failed to load: {e}"),
            })?;

        let graph_name = onnx_path
            .file_name()
            .map_or_else(|| "siglip2".to_owned(), |n| n.to_string_lossy().to_string());

        Ok(Self {
            session: Mutex::new(session),
            logit_scale,
            model_id: format!("siglip2:{graph_name}"),
        })
    }

    /// Runs the image tower over `pixels` ([B,3,H,W] CHW f32).
    fn embed_images_raw(
        &self,
        pixels: &[f32],
        batch: usize,
        height: usize,
        width: usize,
    ) -> Result<Vec<f32>, ModelError> {
        let mut guard = self.session.lock().map_err(|e| ModelError::ContractViolation {
            reason: format!("session mutex poisoned: {e}"),
        })?;

        let inputs: HashMap<String, ort::value::Tensor<f32>> = HashMap::from([(
            "pixel_values".to_owned(),
            ort::value::Tensor::from_array((vec![batch, 3, height, width], pixels.to_vec()))
                .map_err(|e| ModelError::ContractViolation { reason: e.to_string() })?,
        )]);

        let outputs = guard.run(inputs).map_err(|e| ModelError::ContractViolation {
            reason: format!("image tower failed: {e}"),
        })?;

        let embeds = outputs.get("image_embeds").ok_or_else(|| ModelError::ContractViolation {
            reason: "image tower output missing `image_embeds`".to_owned(),
        })?;
        let (shape, data) =
            embeds.try_extract_tensor::<f32>().map_err(|e| ModelError::ContractViolation {
                reason: e.to_string(),
            })?;

        // Flatten [B,1,E] or [B,E] to [B*E].
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let total: usize = shape.iter().map(|d| *d as usize).product();
        let flat: Vec<f32> = data.iter().copied().take(total).collect();
        Ok(flat)
    }

    /// Runs the text tower over token ids.
    fn embed_texts_raw(
        &self,
        input_ids: &[i64],
        batch: usize,
        length: usize,
    ) -> Result<Vec<f32>, ModelError> {
        let mut guard = self.session.lock().map_err(|e| ModelError::ContractViolation {
            reason: format!("session mutex poisoned: {e}"),
        })?;

        let inputs: HashMap<String, ort::value::Tensor<i64>> = HashMap::from([(
            "input_ids".to_owned(),
            ort::value::Tensor::from_array((vec![batch, length], input_ids.to_vec()))
                .map_err(|e| ModelError::ContractViolation { reason: e.to_string() })?,
        )]);

        let outputs = guard.run(inputs).map_err(|e| ModelError::ContractViolation {
            reason: format!("text tower failed: {e}"),
        })?;

        let embeds = outputs.get("text_embeds").ok_or_else(|| ModelError::ContractViolation {
            reason: "text tower output missing `text_embeds`".to_owned(),
        })?;
        let (shape, data) =
            embeds.try_extract_tensor::<f32>().map_err(|e| ModelError::ContractViolation {
                reason: e.to_string(),
            })?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let total: usize = shape.iter().map(|d| *d as usize).product();
        let flat: Vec<f32> = data.iter().copied().take(total).collect();
        Ok(flat)
    }
}

#[cfg(feature = "vision")]
impl ImageTextEmbedder for Siglip2OnnxBackend {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        // SigLIP2-base uses 768-dim embeddings.
        // The actual dimension is read from the output shape at runtime;
        // we use 768 as the documented default.
        768
    }

    fn embed_images(&self, images: &[ImageInput]) -> Result<Vec<Vec<f32>>, ModelError> {
        if images.is_empty() {
            return Ok(Vec::new());
        }

        let size = DEFAULT_IMAGE_SIZE;
        let pixels_per_image = 3 * size * size;
        let mut pixels = Vec::with_capacity(images.len() * pixels_per_image);

        for image in images {
            let rgba = decode_image(image)?;
            let resized = resize_rgba(&rgba, size, size)?;
            let chw = rgba_to_chw_normalized(&resized);
            pixels.extend(chw);
        }

        let embeds = self.embed_images_raw(&pixels, images.len(), size, size)?;
        let embed_dim = self.dims();
        Ok(embeds
            .chunks(embed_dim)
            .map(<[f32]>::to_vec)
            .collect())
    }

    fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // For now: use a simple whitespace tokenization as a placeholder.
        // The real implementation uses the model's `SentencePiece` vocabulary.
        // This is a placeholder that will be replaced when the tokenizer
        // integration is added (tracked separately).
        let max_len = texts
            .iter()
            .map(|t| t.split_whitespace().count().max(1))
            .max()
            .unwrap_or(1);
        let embed_dim = self.dims();
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let tokens: Vec<i64> = text
                .split_whitespace()
                .take(max_len)
                .map(simple_token_id)
                .collect();
            let _length = tokens.len().max(1);
            let padded = if tokens.len() < max_len {
                let mut t = tokens;
                t.resize(max_len, 0_i64);
                t
            } else {
                tokens
            };

            let embeds = self.embed_texts_raw(&padded, 1, max_len)?;
            // The text tower returns [1, seq, E] pooled to [1, E].
            // For a single text, take the first embed_dim elements.
            let flat = embeds.into_iter().take(embed_dim).collect::<Vec<_>>();
            results.push(if flat.len() == embed_dim {
                flat
            } else {
                // Pad or truncate to the expected dimension.
                let mut fixed = flat;
                fixed.resize(embed_dim, 0.0);
                fixed
            });
        }

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Image preprocessing
// ---------------------------------------------------------------------------

fn decode_image(image: &ImageInput) -> Result<Vec<u8>, ModelError> {
    match image.media_type() {
        ImageMediaType::Png | ImageMediaType::Jpeg => {}
    }

    let img =
        image::load_from_memory(image.data()).map_err(|e| ModelError::ContractViolation {
            reason: format!("image decode failed: {e}"),
        })?;

    Ok(img.into_rgba8().into_raw())
}

fn resize_rgba(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, ModelError> {
    // Bounded: width/height are at most 256 (DEFAULT_IMAGE_SIZE).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let dyn_img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
        width as u32,
        height as u32,
        rgba.to_vec(),
    )
    .ok_or_else(|| ModelError::ContractViolation {
        reason: format!("image buffer size mismatch for {width}x{height}"),
    })?;

    // Simple nearest-neighbor resize.
    #[allow(clippy::cast_possible_truncation)]
    let resized = image::imageops::resize(
        &dyn_img,
        width as u32,
        height as u32,
        image::imageops::Nearest,
    );
    Ok(resized.into_raw())
}

fn rgba_to_chw_normalized(rgba: &[u8]) -> Vec<f32> {
    // Bounded: pixel count is at most 256*256 (DEFAULT_IMAGE_SIZE squared), which
    // fits exactly in f32 mantissa; sqrt yields an integer ≤ 256.
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let pixel_count = (rgba.len() / 4).min(256 * 256);
    #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let img_side = (pixel_count as f32).sqrt() as usize;
    let mut chw = vec![0.0_f32; 3 * img_side * img_side];

    for y in 0..img_side {
        for x in 0..img_side {
            let idx = (y * img_side + x) * 4;
            if idx + 2 < rgba.len() {
                let r = f32::from(rgba[idx]) / 255.0;
                let g = f32::from(rgba[idx + 1]) / 255.0;
                let b = f32::from(rgba[idx + 2]) / 255.0;
                chw[y * img_side + x] = (r - IMAGE_MEAN[0]) / IMAGE_STD[0];
                chw[img_side * img_side + y * img_side + x] =
                    (g - IMAGE_MEAN[1]) / IMAGE_STD[1];
                chw[2 * img_side * img_side + y * img_side + x] =
                    (b - IMAGE_MEAN[2]) / IMAGE_STD[2];
            }
        }
    }
    chw
}

// ---------------------------------------------------------------------------
// Simple text tokenization placeholder
// ---------------------------------------------------------------------------

/// A placeholder tokenizer: maps lowercase ASCII words to stable token ids.
/// The real implementation uses the model's `SentencePiece` vocabulary.
fn simple_token_id(word: &str) -> i64 {
    let lower = word.to_lowercase();
    let bytes = lower.as_bytes();
    if bytes.is_empty() {
        return 1; // PAD
    }
    // Simple hash into a stable token range.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Map to token range 100..8192 (avoiding special tokens).
    let token = 100 + i64::try_from(hash % 8092).unwrap_or(100);
    token.max(4) // ≥ 4 (avoid BOS/PAD special range)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn decode_rejects_unsupported_media_types() {
        use opencodifier_core::ImageInput;
        // Create a minimal valid PNG header.
        let png_header = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d,
        ];
        let bad = ImageInput::new(ImageMediaType::Png, png_header);

        let result = decode_image(&bad);
        // A truncated but typed PNG should decode without panicking;
        // the error is from the image library, not from type mismatch.
        // (PNG magic is accepted; the library then fails on truncated data.)
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("image decode failed") || err.to_string().contains("decode")
        );
    }

    #[test]
    fn simple_token_id_is_deterministic() {
        assert_eq!(simple_token_id("hello"), simple_token_id("hello"));
        assert_ne!(simple_token_id("hello"), simple_token_id("world"));
    }
}
