//! Parsing helpers shared by the wire-format adapters.
//!
//! These are private plumbing: type-checked field access over
//! [`serde_json::Value`] that maps every failure onto a [`SchemaError`]
//! with a stable code, plus the deterministic distribution reconstruction
//! the lossy adapters (`OpenAI`, `Anthropic`, `Jev` choice answers) rely on.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::{Map, Value};

use opencode_core::{Distribution, ImageMediaType, ScoreLevel};

use crate::error::{SchemaError, SchemaResult};

/// Borrows `value` as an object, or fails with `schema.invalid_type`.
pub(crate) fn as_object<'a>(field: &str, value: &'a Value) -> SchemaResult<&'a Map<String, Value>> {
    value.as_object().ok_or_else(|| SchemaError::invalid_type(field, "object", value))
}

/// Borrows `value` as a string, or fails with `schema.invalid_type`.
pub(crate) fn as_string<'a>(field: &str, value: &'a Value) -> SchemaResult<&'a str> {
    value.as_str().ok_or_else(|| SchemaError::invalid_type(field, "string", value))
}

/// Reads a required string field from an object.
pub(crate) fn required_string(map: &Map<String, Value>, field: &str) -> SchemaResult<String> {
    let value = map.get(field).ok_or_else(|| SchemaError::missing(field))?;
    as_string(field, value).map(str::to_owned)
}

/// Reads a required object field from an object.
pub(crate) fn required_object<'a>(
    map: &'a Map<String, Value>,
    field: &str,
) -> SchemaResult<&'a Map<String, Value>> {
    let value = map.get(field).ok_or_else(|| SchemaError::missing(field))?;
    as_object(field, value)
}

/// Reads a required array field from an object.
pub(crate) fn required_array<'a>(
    map: &'a Map<String, Value>,
    field: &str,
) -> SchemaResult<&'a Vec<Value>> {
    let value = map.get(field).ok_or_else(|| SchemaError::missing(field))?;
    value.as_array().ok_or_else(|| SchemaError::invalid_type(field, "array", value))
}

/// Reads an optional string field, ignoring its absence.
pub(crate) fn optional_string(
    map: &Map<String, Value>,
    field: &str,
) -> SchemaResult<Option<String>> {
    match map.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => as_string(field, value).map(str::to_owned).map(Some),
    }
}

/// Rejects an empty string with `schema.missing_field`, since an absent and
/// a blank required label carry the same information: none.
pub(crate) fn non_empty(field: &str, value: String) -> SchemaResult<String> {
    if value.is_empty() {
        return Err(SchemaError::MissingField {
            field: field.to_owned(),
            context: " must not be empty".to_owned(),
        });
    }
    Ok(value)
}

/// Validates a finite probability in `[0, 1]`.
pub(crate) fn unit_interval(field: &str, value: f64) -> SchemaResult<f64> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(SchemaError::invalid_value(field, format!("{value} is not in [0, 1]")));
    }
    Ok(value)
}

/// Reconstructs a distribution from a single observed value.
///
/// This is the documented fidelity limitation shared by the `OpenAI`,
/// `Anthropic`, and `Jev` choice adapters: those wire formats return the
/// *chosen* value only, never a probability vector. The adapter reports the
/// choice as certain (`1.0`) and the caller must treat the resulting
/// confidence as "the vendor decided", not "the vendor was 100% sure" —
/// calibration and verification therefore happen upstream in the engine,
/// not here. This is reconstruction of a reported decision, not invented
/// evidence.
pub(crate) fn certain_distribution(key: &str) -> SchemaResult<Distribution> {
    Distribution::from_pairs([(key, 1.0)])
        .map_err(|error| SchemaError::invalid_value(key, error.to_string()))
}

/// Builds an ordered score level from a wire label.
pub(crate) fn score_level(field: &str, label: String) -> SchemaResult<ScoreLevel> {
    ScoreLevel::new(label).map_err(|error| SchemaError::invalid_value(field, error.to_string()))
}

/// Parses a `data:<media-type>;base64,<payload>` URL into its media type
/// and decoded bytes — the one image-URL grammar every image-carrying
/// adapter accepts, so the wire dialects cannot drift apart. The bytes
/// are held decoded (the IR's canonical re-encode happens at
/// serialization), so two callers sending the same image under different
/// base64 flavors cache-key identically.
pub(crate) fn parse_image_data_url(
    field: &str,
    url: &str,
) -> SchemaResult<(ImageMediaType, Vec<u8>)> {
    let rest = url.strip_prefix("data:").ok_or_else(|| {
        SchemaError::invalid_value(field, "image reference must be a `data:` URL")
    })?;
    let (media, payload) = rest.split_once(";base64,").ok_or_else(|| {
        SchemaError::invalid_value(field, "image reference must carry `;base64,` encoded data")
    })?;
    let media_type = ImageMediaType::try_from(media).map_err(SchemaError::from)?;
    let bytes = BASE64_STANDARD
        .decode(payload.as_bytes())
        .map_err(|_| SchemaError::invalid_value(field, "payload is not valid base64"))?;
    Ok((media_type, bytes))
}

/// Decodes a bare base64 image payload (no data-URL wrapper — the media
/// type arrives in its own wire field), mapping failure onto
/// `schema.invalid_value` with the named field.
pub(crate) fn decode_base64_image(field: &str, payload: &str) -> SchemaResult<Vec<u8>> {
    BASE64_STANDARD
        .decode(payload.as_bytes())
        .map_err(|_| SchemaError::invalid_value(field, "payload is not valid base64"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use serde_json::json;

    #[test]
    fn as_object_and_as_string_report_invalid_types() {
        assert!(as_object("state", &json!({})).is_ok());
        let error = as_object("state", &json!("hi")).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");
        assert_eq!(
            error.to_string(),
            "field `state` has an invalid type: expected object, got string"
        );

        assert_eq!(as_string("model", &json!("jev")).unwrap(), "jev");
        let error = as_string("model", &json!(7)).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");
    }

    #[test]
    fn required_accessors_distinguish_missing_from_mistyped() {
        let map = json!({"text": "x", "items": [1]}).as_object().unwrap().clone();

        assert_eq!(required_string(&map, "text").unwrap(), "x");
        assert_eq!(required_string(&map, "nope").unwrap_err().code(), "schema.missing_field");
        assert_eq!(required_string(&map, "items").unwrap_err().code(), "schema.invalid_type");

        assert_eq!(required_array(&map, "items").unwrap().len(), 1);
        assert_eq!(required_array(&map, "text").unwrap_err().code(), "schema.invalid_type");
        assert_eq!(required_array(&map, "nope").unwrap_err().code(), "schema.missing_field");

        assert_eq!(required_object(&map, "text").unwrap_err().code(), "schema.invalid_type");
        assert_eq!(required_object(&map, "nope").unwrap_err().code(), "schema.missing_field");
    }

    #[test]
    fn optional_string_treats_null_as_absent() {
        let map = json!({"a": "x", "b": null, "c": 3}).as_object().unwrap().clone();
        assert_eq!(optional_string(&map, "a").unwrap(), Some("x".to_owned()));
        assert_eq!(optional_string(&map, "b").unwrap(), None);
        assert_eq!(optional_string(&map, "missing").unwrap(), None);
        assert_eq!(optional_string(&map, "c").unwrap_err().code(), "schema.invalid_type");
    }

    #[test]
    fn non_empty_reports_missing_field() {
        assert_eq!(non_empty("label", "easy".into()).unwrap(), "easy");
        let error = non_empty("label", String::new()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert_eq!(error.to_string(), "missing required field `label` must not be empty");
    }

    #[test]
    fn unit_interval_rejects_out_of_range_and_non_finite() {
        assert_eq!(unit_interval("probability", 0.25).unwrap(), 0.25);
        assert_eq!(unit_interval("probability", 0.0).unwrap(), 0.0);
        assert_eq!(unit_interval("probability", 1.0).unwrap(), 1.0);
        assert_eq!(unit_interval("p", 1.5).unwrap_err().code(), "schema.invalid_value");
        assert_eq!(unit_interval("p", f64::NAN).unwrap_err().code(), "schema.invalid_value");
    }

    #[test]
    fn certain_distribution_is_normalized_and_tops_at_key() {
        let distribution = certain_distribution("qwen").unwrap();
        assert_eq!(distribution.top().key, "qwen");
        assert!((distribution.top().probability - 1.0).abs() < 1e-9);
        assert_eq!(
            certain_distribution("qwen").unwrap(),
            certain_distribution("qwen").unwrap(),
            "the reconstruction is deterministic"
        );
    }

    #[test]
    fn score_level_wraps_core_validation_errors() {
        assert_eq!(score_level("levels", "easy".into()).unwrap().label(), "easy");
        assert_eq!(
            score_level("levels", String::new()).unwrap_err().code(),
            "schema.invalid_value"
        );
    }

    #[test]
    fn data_urls_parse_into_media_type_and_bytes() {
        let (media, bytes) =
            parse_image_data_url("input[].image_url", "data:image/png;base64,iVBORw0KGgo=")
                .unwrap();
        assert_eq!(media, ImageMediaType::Png);
        assert_eq!(bytes, vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // Bare `jpeg` (no `image/` prefix) is the alias the IR accepts.
        let (media, _) = parse_image_data_url("url", "data:jpeg;base64,/9j/4A==").unwrap();
        assert_eq!(media, ImageMediaType::Jpeg);
    }

    #[test]
    fn malformed_data_urls_are_typed_invalid_values() {
        // Not a `data:` URL at all — a remote reference cannot be decided
        // locally.
        let error = parse_image_data_url("url", "https://example.test/x.png").unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert_eq!(
            error.to_string(),
            "invalid value for `url`: image reference must be a `data:` URL"
        );

        // No `;base64,` separator.
        let error = parse_image_data_url("url", "data:image/png,image.png").unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains(";base64,"), "{error}");

        // Unknown media type surfaces the IR's own typed error (its core
        // code remaps to `schema.invalid_value` at this boundary).
        let error = parse_image_data_url("url", "data:image/webp;base64,aGk=").unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("image/webp"), "{error}");

        // Payload that is not base64.
        let error = parse_image_data_url("url", "data:image/png;base64,not base64!!").unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("base64"), "{error}");
    }

    #[test]
    fn bare_base64_payloads_decode_with_named_fields() {
        assert_eq!(decode_base64_image("source.data", "aGk=").unwrap(), b"hi".to_vec());
        let error = decode_base64_image("source.data", "!!!").unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("source.data"), "{error}");
    }
}
