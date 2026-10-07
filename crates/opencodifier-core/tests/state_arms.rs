//! State fact operations: typed lookup, canonical iteration, numeric
//! coercion, and the wire tags every fact kind round trips through.
//!
//! The state is the thing a decision is made against, so its facts are
//! hostile input too: whatever arrives on the wire must come back as the
//! same typed value construction would have produced, and a fact the IR
//! refuses (a non-finite float) must be refused on the wire as well.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_core::{FactValue, State};

/// The one-fact state a lookup test reads back.
fn state_with_fact(key: &str, value: FactValue) -> State {
    State::from_text("deploy is failing").with_fact(key, value)
}

#[test]
fn fact_lookup_returns_the_typed_value_or_none() {
    let state = state_with_fact("context_tokens", FactValue::Integer(42_000));
    assert_eq!(state.fact("context_tokens"), Some(&FactValue::Integer(42_000)));
    // An absent key is a `None`, never a panic and never a default fact:
    // the caller decides what an unanswered fact means.
    assert_eq!(state.fact("never_inserted"), None);
}

#[test]
fn facts_iterate_in_canonical_key_order() {
    let state = State::from_text("route this request")
        .with_fact("zebra", FactValue::Integer(1))
        .with_fact("alpha", FactValue::Text("first".into()))
        .with_fact("middle", FactValue::Boolean(true));

    let keys: Vec<&str> = state.facts().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, vec!["alpha", "middle", "zebra"], "iteration must be sorted, not insertion");

    // A text-only state has no facts to iterate, and the iterator says so
    // rather than yielding anything for the text itself.
    let text_only = State::from_text("no facts here");
    assert_eq!(text_only.facts().count(), 0);
    assert_eq!(text_only.fact("text"), None, "the raw text is not a fact");
}

#[test]
fn numeric_facts_convert_to_f64_and_the_rest_do_not() {
    assert_eq!(FactValue::Integer(7).as_f64(), Some(7.0));
    assert_eq!(FactValue::Float(0.75).as_f64(), Some(0.75));
    // Non-numeric kinds have no numeric reading: the caller must not be
    // handed a silent `0.0` for a fact that was never a number.
    assert_eq!(FactValue::Text("0.75".into()).as_f64(), None);
    assert_eq!(FactValue::Boolean(true).as_f64(), None);
    assert_eq!(FactValue::List(vec!["1".into()]).as_f64(), None);
}

#[test]
fn every_fact_kind_round_trips_through_its_wire_tag() {
    let facts = [
        ("text", FactValue::Text("text+image".into())),
        ("integer", FactValue::Integer(-12)),
        ("float", FactValue::float(0.25).expect("finite")),
        ("boolean", FactValue::Boolean(false)),
        ("list", FactValue::List(vec!["text".into(), "image".into()])),
    ];
    for (kind, fact) in facts {
        let json = serde_json::to_value(&fact).expect("serialize fact");
        assert_eq!(json["kind"], kind, "wire tag of {fact}");
        let back: FactValue = serde_json::from_value(json).expect("deserialize fact");
        assert_eq!(back, fact, "round trip of kind `{kind}`");
    }
}

#[test]
fn a_wire_float_fact_that_is_not_finite_is_refused() {
    // `1e999` overflows every f64; whether the JSON layer or the float
    // constructor refuses it, the IR must never hold a non-finite fact —
    // serialized state has to stay valid JSON and cache-key stable.
    let overflowing = serde_json::from_str::<FactValue>(r#"{"kind":"float","value":1e999}"#);
    assert!(overflowing.is_err(), "a non-finite float fact must be refused");
}
