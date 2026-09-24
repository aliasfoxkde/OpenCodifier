//! Input state: the unstructured text plus structured facts a decision is
//! evaluated against.

use std::collections::BTreeMap;

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

/// The input a decision is made against: raw text plus typed facts.
///
/// Facts live in a `BTreeMap` so canonical serialization order — and
/// therefore the exact-decision cache key — is stable regardless of the
/// order facts were inserted.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct State {
    text: String,
    facts: BTreeMap<String, FactValue>,
}

impl State {
    /// Builds a state from raw unstructured text, with no facts.
    pub fn from_text(text: impl Into<String>) -> Self {
        Self { text: text.into(), facts: BTreeMap::new() }
    }

    /// Builder-style fact insertion, consuming and returning `self`.
    #[must_use]
    pub fn with_fact(mut self, key: impl Into<String>, value: FactValue) -> Self {
        self.facts.insert(key.into(), value);
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
