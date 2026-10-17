//! Input state: the unstructured text plus structured facts a decision is
//! evaluated against.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// A single typed fact about the request state.
///
/// `Float` rejects non-finite values (`NaN`, ±inf) at construction so
/// serialized state stays valid JSON and cache keys stay deterministic
/// across platforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", try_from = "RawFactValue")]
#[non_exhaustive]
pub enum FactValue {
    /// Free-text fact (e.g. modality: `"text+image"`).
    Text(String),
    /// Signed integer fact (e.g. context token count).
    Integer(i64),
    /// Finite float fact (e.g. a prior score).
    Float(f64),
    /// Boolean fact (e.g. privacy: local-only).
    Boolean(bool),
    /// Ordered list of short string tokens (e.g. available modalities).
    List(Vec<String>),
}

/// Deserialization mirror for [`FactValue`]; conversion validates.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum RawFactValue {
    /// Mirrors [`FactValue::Text`].
    Text(String),
    /// Mirrors [`FactValue::Integer`].
    Integer(i64),
    /// Mirrors [`FactValue::Float`].
    Float(f64),
    /// Mirrors [`FactValue::Boolean`].
    Boolean(bool),
    /// Mirrors [`FactValue::List`].
    List(Vec<String>),
}

impl TryFrom<RawFactValue> for FactValue {
    type Error = CoreError;

    fn try_from(raw: RawFactValue) -> CoreResult<Self> {
        match raw {
            RawFactValue::Text(text) => Ok(Self::Text(text)),
            RawFactValue::Integer(int) => Ok(Self::Integer(int)),
            RawFactValue::Float(float) => Self::float(float),
            RawFactValue::Boolean(flag) => Ok(Self::Boolean(flag)),
            RawFactValue::List(items) => Ok(Self::List(items)),
        }
    }
}

impl FactValue {
    /// Constructs a float fact, rejecting non-finite values.
    pub fn float(value: f64) -> CoreResult<Self> {
        if value.is_finite() {
            Ok(Self::Float(value))
        } else {
            Err(CoreError::InvalidProbability { value })
        }
    }

    /// Returns the value as `f64` when it is numeric.
    ///
    /// Integer facts up to 2^53 convert exactly; larger magnitudes lose
    /// precision the same way JSON numbers do, which is acceptable for
    /// the quantities facts carry (counts, sizes, priors).
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(int) => Some(*int as f64),
            Self::Float(float) => Some(*float),
            _ => None,
        }
    }

    /// Returns the text when the fact is textual.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Returns `true` when the fact is a boolean equal to `expected`.
    pub fn is_boolean(&self, expected: bool) -> bool {
        matches!(self, Self::Boolean(flag) if *flag == expected)
    }

    /// Returns `true` when the fact is a list containing `item`
    /// (case-sensitive exact match).
    pub fn list_contains(&self, item: &str) -> bool {
        match self {
            Self::List(items) => items.iter().any(|candidate| candidate == item),
            _ => false,
        }
    }
}

impl std::fmt::Display for FactValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Integer(int) => write!(f, "{int}"),
            Self::Float(float) => write!(f, "{float}"),
            Self::Boolean(flag) => write!(f, "{flag}"),
            Self::List(items) => write!(f, "[{}]", items.join(", ")),
        }
    }
}

/// The image media types the IR accepts.
///
/// The set is closed on purpose: adapters map wire media types onto these
/// variants (or refuse them), and image preprocessing is implemented per
/// variant — a new format means a new variant plus the decoder that
/// justifies it, never a pass-through string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageMediaType {
    /// PNG (RFC 2083).
    Png,
    /// JPEG (ITU-T T.81).
    Jpeg,
}

impl ImageMediaType {
    /// The canonical MIME string: `"image/png"` or `"image/jpeg"`.
    ///
    /// This is the string canonical serialization emits, so a state built
    /// through any adapter cache-keys identically.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

impl std::fmt::Display for ImageMediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ImageMediaType {
    type Error = CoreError;

    /// Accepts the canonical MIME strings plus the common file-extension
    /// aliases (`png`, `jpg`, `jpeg`); everything else is a typed refusal.
    fn try_from(value: &str) -> CoreResult<Self> {
        match value {
            "image/png" | "png" => Ok(Self::Png),
            "image/jpeg" | "jpeg" | "jpg" => Ok(Self::Jpeg),
            other => Err(CoreError::UnsupportedMediaType { media_type: other.to_owned() }),
        }
    }
}

/// One image attached to a [`State`].
///
/// Bytes are held decoded; the wire carries base64 in both directions.
/// The IR decodes at its boundary and re-encodes canonically (STANDARD
/// alphabet, padding) on the way out, so the exact-decision cache key is a
/// function of the image bytes, never of the caller's base64 flavor.
///
/// Size enforcement is the caller's [`Limits`](crate::policy::Limits) at
/// [`DecisionRequest::new`](crate::request::DecisionRequest::new), with
/// one hostile-input exception: the wire ceiling
/// [`MAX_ENCODED_IMAGE_BYTES`] applies at decode time because a payload
/// that size would otherwise be allocated before any limit could see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawImageInput", into = "RawImageInput")]
pub struct ImageInput {
    media_type: ImageMediaType,
    data: Vec<u8>,
}

/// Wire mirror for [`ImageInput`]: media type as its canonical MIME
/// string, payload as canonical base64.
#[derive(Debug, Deserialize, Serialize)]
struct RawImageInput {
    media_type: String,
    data: String,
}

/// Wire ceiling on one encoded image payload, enforced where the bytes
/// would otherwise be allocated (hostile-input bound; 16 MiB covers the
/// default limits four times over plus base64 overhead).
pub const MAX_ENCODED_IMAGE_BYTES: usize = 16 * 1024 * 1024;

impl TryFrom<RawImageInput> for ImageInput {
    type Error = CoreError;

    fn try_from(raw: RawImageInput) -> CoreResult<Self> {
        let media_type = ImageMediaType::try_from(raw.media_type.as_str())?;
        if raw.data.len() > MAX_ENCODED_IMAGE_BYTES {
            return Err(CoreError::ImageTooLarge {
                bytes: raw.data.len(),
                limit: MAX_ENCODED_IMAGE_BYTES,
            });
        }
        let data = BASE64_STANDARD
            .decode(raw.data.as_bytes())
            .map_err(|error| CoreError::InvalidImage { reason: error.to_string() })?;
        Ok(Self { media_type, data })
    }
}

impl From<ImageInput> for RawImageInput {
    fn from(image: ImageInput) -> Self {
        Self {
            media_type: image.media_type.as_str().to_owned(),
            data: BASE64_STANDARD.encode(image.data),
        }
    }
}

impl ImageInput {
    /// Constructs an image from a media type and decoded bytes.
    pub fn new(media_type: ImageMediaType, data: Vec<u8>) -> Self {
        Self { media_type, data }
    }

    /// The image's media type.
    pub fn media_type(&self) -> ImageMediaType {
        self.media_type
    }

    /// The image's decoded bytes.
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/// The input a decision is made against: raw text plus typed facts.
///
/// Facts live in a `BTreeMap` so canonical serialization order — and
/// therefore the exact-decision cache key — is stable regardless of the
/// order facts were inserted.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct State {
    text: String,
    /// Optional on the wire: omitted means no typed facts, the shape
    /// every text-only quickstart means.
    #[serde(default)]
    facts: BTreeMap<String, FactValue>,
    /// Optional on the wire: omitted means no images, the shape every
    /// text-only caller means. Images live outside `facts` so their bytes
    /// can never masquerade as matchable metadata, and they stay exempt
    /// from `max_input_bytes` (text budget) in favor of their own
    /// `max_image_bytes` / `max_images` ceilings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<ImageInput>,
}

impl State {
    /// Builds a state from raw unstructured text, with no facts.
    pub fn from_text(text: impl Into<String>) -> Self {
        Self { text: text.into(), facts: BTreeMap::new(), images: Vec::new() }
    }

    /// Builder-style fact insertion, consuming and returning `self`.
    #[must_use]
    pub fn with_fact(mut self, key: impl Into<String>, value: FactValue) -> Self {
        self.facts.insert(key.into(), value);
        self
    }

    /// Builder-style image attachment, consuming and returning `self`.
    ///
    /// Images are appended in call order; canonical serialization
    /// preserves that order, so attach deterministically.
    #[must_use]
    pub fn with_image(mut self, image: ImageInput) -> Self {
        self.images.push(image);
        self
    }

    /// The raw unstructured text of this state.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Looks a fact up by key.
    pub fn fact(&self, key: &str) -> Option<&FactValue> {
        self.facts.get(key)
    }

    /// Iterates facts in canonical (sorted) key order.
    pub fn facts(&self) -> impl Iterator<Item = (&String, &FactValue)> {
        self.facts.iter()
    }

    /// The images attached to this state, in attachment order.
    pub fn images(&self) -> &[ImageInput] {
        &self.images
    }

    /// How many images are attached to this state.
    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    /// `true` when the state carries at least one image.
    pub fn has_images(&self) -> bool {
        !self.images.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn serializes_in_canonical_key_order() {
        let state = State::from_text("hello")
            .with_fact("zebra", FactValue::Integer(1))
            .with_fact("alpha", FactValue::Text("x".into()));
        let json = serde_json::to_string(&state).expect("serialize");
        assert_eq!(
            json,
            r#"{"text":"hello","facts":{"alpha":{"kind":"text","value":"x"},"zebra":{"kind":"integer","value":1}}}"#
        );
    }

    #[test]
    fn imageless_state_serializes_byte_identically_to_the_text_only_shape() {
        // The `images` field must stay invisible on the wire when empty —
        // every committed fixture and cache key depends on it. The shape
        // pinned here is the exact pre-images serialization (`facts` always
        // serializes, even empty).
        let state = State::from_text("hello");
        let json = serde_json::to_string(&state).expect("serialize");
        assert_eq!(json, r#"{"text":"hello","facts":{}}"#);
    }

    #[test]
    fn image_media_type_accepts_canonical_and_alias_forms() {
        assert_eq!(
            ImageMediaType::try_from("image/png").expect("canonical png"),
            ImageMediaType::Png
        );
        assert_eq!(ImageMediaType::try_from("jpg").expect("alias jpg"), ImageMediaType::Jpeg);
        assert_eq!(ImageMediaType::Png.as_str(), "image/png");
        assert_eq!(ImageMediaType::Jpeg.to_string(), "image/jpeg");
    }

    #[test]
    fn image_media_type_refuses_unknown_types() {
        let error = ImageMediaType::try_from("image/webp").expect_err("refused");
        assert_eq!(error.code(), "ir.unsupported_media_type");
        assert!(
            matches!(error, CoreError::UnsupportedMediaType { media_type } if media_type == "image/webp")
        );
    }

    #[test]
    fn image_payload_decodes_base64_and_canonicalizes_on_the_wire() {
        let image = ImageInput::new(ImageMediaType::Png, vec![0xFA, 0xCE, 0x01]);
        let json = serde_json::to_string(&image).expect("serialize");
        assert_eq!(json, r#"{"media_type":"image/png","data":"+s4B"}"#);
        let back: ImageInput = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, image);
    }

    #[test]
    fn image_payload_canonical_input_reencodes_identically() {
        // Canonical padded input decodes and re-encodes byte-identically:
        // cache keys depend on image bytes, never on the wire flavor (the
        // non-canonical flavors are refused outright, see below).
        let padded = r#"{"media_type":"image/jpeg","data":"/9j/4AAQ"}"#;
        let image: ImageInput = serde_json::from_str(padded).expect("decode");
        let re = serde_json::to_string(&image).expect("serialize");
        assert_eq!(re, padded);
        assert_eq!(image.data(), [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
    }

    #[test]
    fn image_payload_refuses_non_canonical_alphabets_and_padding() {
        // URL-safe alphabet character (`-`) is not STANDARD base64.
        let error = ImageInput::try_from(RawImageInput {
            media_type: "image/png".to_owned(),
            data: "a-bc".to_owned(),
        })
        .expect_err("refused");
        assert_eq!(error.code(), "ir.invalid_image");
        // Missing padding is likewise refused by the canonical engine.
        let error = ImageInput::try_from(RawImageInput {
            media_type: "image/png".to_owned(),
            data: "QQ".to_owned(),
        })
        .expect_err("refused");
        assert_eq!(error.code(), "ir.invalid_image");
        // And the typed refusal survives the serde boundary as a message.
        let error =
            serde_json::from_str::<ImageInput>(r#"{"media_type":"image/png","data":"a-bc"}"#)
                .expect_err("refused");
        assert!(error.to_string().contains("invalid image payload"));
    }

    #[test]
    fn image_payload_refuses_oversized_wire_input() {
        let error = ImageInput::try_from(RawImageInput {
            media_type: "image/png".to_owned(),
            data: "A".repeat(MAX_ENCODED_IMAGE_BYTES + 1),
        })
        .expect_err("refused");
        assert_eq!(error.code(), "ir.image_too_large");
    }

    #[test]
    fn state_image_accessors_and_builder() {
        let image = ImageInput::new(ImageMediaType::Jpeg, vec![1, 2, 3]);
        let state = State::from_text("what is pictured").with_image(image.clone());
        assert!(state.has_images());
        assert_eq!(state.image_count(), 1);
        assert_eq!(state.images(), [image].as_slice());
        assert!(!State::from_text("plain").has_images());
    }

    #[test]
    fn state_with_images_round_trips_through_json() {
        let state = State::from_text("what is pictured")
            .with_image(ImageInput::new(ImageMediaType::Png, vec![0xDE, 0xAD, 0xBE, 0xEF]));
        let json = serde_json::to_string(&state).expect("serialize");
        assert!(json.contains(r#""images":"#));
        let back: State = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, state);
    }

    #[test]
    fn float_fact_rejects_non_finite() {
        assert!(FactValue::float(f64::NAN).is_err());
        assert!(FactValue::float(f64::INFINITY).is_err());
        assert!(FactValue::float(0.5).is_ok());
    }

    #[test]
    fn fact_accessors() {
        let list = FactValue::List(vec!["image".into(), "text".into()]);
        assert!(list.list_contains("image"));
        assert!(!list.list_contains("video"));
        assert!(FactValue::Boolean(true).is_boolean(true));
        assert!(!FactValue::Boolean(false).is_boolean(true));
        assert_eq!(FactValue::Integer(7).as_f64(), Some(7.0));
        assert_eq!(FactValue::Text("t".into()).as_str(), Some("t"));
        assert_eq!(FactValue::Integer(1).as_str(), None);
    }

    #[test]
    fn round_trips_through_json() {
        let state = State::from_text("refactor the parser")
            .with_fact("context_tokens", FactValue::Integer(42_000))
            .with_fact("prior_difficulty", FactValue::float(0.75).expect("finite"))
            .with_fact("privacy", FactValue::Boolean(true))
            .with_fact("modalities", FactValue::List(vec!["text".into()]));
        let json = serde_json::to_string(&state).expect("serialize");
        let back: State = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, state);
    }

    #[test]
    fn display_is_readable() {
        assert_eq!(FactValue::Text("hi".into()).to_string(), "hi");
        assert_eq!(FactValue::Integer(-2).to_string(), "-2");
        assert_eq!(FactValue::List(vec!["a".into(), "b".into()]).to_string(), "[a, b]");
    }
}
