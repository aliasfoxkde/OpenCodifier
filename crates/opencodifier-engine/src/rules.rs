//! Deterministic rule engine (PLANNING.md §11).
//!
//! Rules are data — plain serde structs in JSON wire format, no embedded
//! scripting language — and they execute in declared order, so a rule set
//! is a reproducible program. Rules run *before* any scoring, which is the
//! whole point: a fact comparison is cheaper and more trustworthy than a
//! model.
//!
//! A rule has a [`Condition`] over state facts and a list of [`Action`]s.
//! Actions can set facts (feeding later stages) and exclude or pin
//! candidates (feeding candidate narrowing, §45). [`RuleEngine::evaluate`]
//! is pure: it reports effects instead of mutating, so the executor can
//! apply them and record them in the trace.
//!
//! # Wire format
//!
//! ```json
//! {
//!   "name": "long context",
//!   "when": { "op": "fact_gt", "fact": "context_tokens", "value": 100000.0 },
//!   "then": [
//!     { "op": "set_fact", "fact": "requires_long_context",
//!       "value": { "kind": "boolean", "value": true } },
//!     { "op": "exclude_candidate", "tag": "small-context" }
//!   ]
//! }
//! ```

use std::collections::BTreeMap;

use opencodifier_core::{Candidate, CandidateId, DecisionQuestion, FactValue, QuestionId, State};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};
use crate::lexical::tokenize;

/// A predicate over state facts.
///
/// Conditions are pure: they read the state and return a boolean. Missing
/// facts make comparisons false rather than erroring, so a rule set is
/// total over any input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    /// The fact exists and equals `value`.
    FactEquals {
        /// Fact key.
        fact: String,
        /// Expected value.
        value: FactValue,
    },
    /// The fact is numeric and strictly greater than `value`.
    FactGt {
        /// Fact key.
        fact: String,
        /// Threshold.
        value: f64,
    },
    /// The fact is numeric and at least `value`.
    FactGte {
        /// Fact key.
        fact: String,
        /// Threshold.
        value: f64,
    },
    /// The fact is numeric and strictly less than `value`.
    FactLt {
        /// Fact key.
        fact: String,
        /// Threshold.
        value: f64,
    },
    /// The fact is numeric and at most `value`.
    FactLte {
        /// Fact key.
        fact: String,
        /// Threshold.
        value: f64,
    },
    /// The fact is text and contains `needle` (case-sensitive).
    FactContains {
        /// Fact key.
        fact: String,
        /// Substring to look for.
        needle: String,
    },
    /// The fact is a list containing `item` (exact match).
    ListContains {
        /// Fact key.
        fact: String,
        /// List member to look for.
        item: String,
    },
    /// The fact is present at all.
    FactExists {
        /// Fact key.
        fact: String,
    },
    /// Every sub-condition holds. An empty list is rejected at validation.
    All {
        /// Sub-conditions.
        conditions: Vec<Condition>,
    },
    /// At least one sub-condition holds. An empty list is rejected at
    /// validation.
    Any {
        /// Sub-conditions.
        conditions: Vec<Condition>,
    },
    /// The sub-condition does not hold.
    Not {
        /// The negated condition.
        condition: Box<Condition>,
    },
}

impl Condition {
    /// Evaluates the condition against `state`.
    #[must_use]
    pub fn matches(&self, state: &State) -> bool {
        match self {
            Self::FactEquals { fact, value } => state.fact(fact) == Some(value),
            Self::FactGt { fact, value } => {
                Self::compare(state, fact, *value) == Some(std::cmp::Ordering::Greater)
            }
            Self::FactGte { fact, value } => {
                matches!(
                    Self::compare(state, fact, *value),
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                )
            }
            Self::FactLt { fact, value } => {
                Self::compare(state, fact, *value) == Some(std::cmp::Ordering::Less)
            }
            Self::FactLte { fact, value } => {
                matches!(
                    Self::compare(state, fact, *value),
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                )
            }
            Self::FactContains { fact, needle } => state
                .fact(fact)
                .and_then(opencodifier_core::FactValue::as_str)
                .is_some_and(|text| text.contains(needle.as_str())),
            Self::ListContains { fact, item } => {
                state.fact(fact).is_some_and(|value| value.list_contains(item))
            }
            Self::FactExists { fact } => state.fact(fact).is_some(),
            Self::All { conditions } => conditions.iter().all(|condition| condition.matches(state)),
            Self::Any { conditions } => conditions.iter().any(|condition| condition.matches(state)),
            Self::Not { condition } => !condition.matches(state),
        }
    }

    /// Compares a numeric fact with a threshold.
    fn compare(state: &State, fact: &str, value: f64) -> Option<std::cmp::Ordering> {
        let found = state.fact(fact)?.as_f64()?;
        found.partial_cmp(&value)
    }

    /// Validates structure: no empty fact names, no empty needles, finite
    /// numeric thresholds, no empty composites.
    pub fn validate(&self) -> EngineResult<()> {
        match self {
            Self::FactEquals { fact, .. } | Self::FactExists { fact } => Self::validate_fact(fact),
            Self::FactGt { fact, value }
            | Self::FactGte { fact, value }
            | Self::FactLt { fact, value }
            | Self::FactLte { fact, value } => {
                Self::validate_fact(fact)?;
                Self::validate_number(*value)
            }
            Self::FactContains { fact, needle } => {
                Self::validate_fact(fact)?;
                if needle.is_empty() {
                    return Err(Self::invalid("fact_contains needle must not be empty"));
                }
                Ok(())
            }
            Self::ListContains { fact, item } => {
                Self::validate_fact(fact)?;
                if item.is_empty() {
                    return Err(Self::invalid("list_contains item must not be empty"));
                }
                Ok(())
            }
            Self::All { conditions } | Self::Any { conditions } => {
                if conditions.is_empty() {
                    return Err(Self::invalid("composite conditions must not be empty"));
                }
                conditions.iter().try_for_each(Condition::validate)
            }
            Self::Not { condition } => condition.validate(),
        }
    }

    fn validate_fact(fact: &str) -> EngineResult<()> {
        if fact.trim().is_empty() {
            return Err(Self::invalid("fact name must not be empty"));
        }
        Ok(())
    }

    fn validate_number(value: f64) -> EngineResult<()> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(Self::invalid("numeric thresholds must be finite"))
        }
    }

    fn invalid(reason: &str) -> EngineError {
        EngineError::InvalidRule { reason: reason.to_owned() }
    }
}

/// An effect a rule applies when its condition holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Action {
    /// Writes a fact into the working state, visible to later stages.
    SetFact {
        /// Fact key to write.
        fact: String,
        /// Value to write.
        value: FactValue,
    },
    /// Removes matching candidates from narrowing.
    ///
    /// Exactly one of `id` (exact candidate id) or `tag` (a token appearing
    /// in the candidate description, or the id itself) must be present.
    ExcludeCandidate {
        /// Exact candidate id to exclude.
        id: Option<String>,
        /// Description token identifying candidates to exclude.
        tag: Option<String>,
    },
    /// Protects matching candidates from exclusion by *other* rules.
    ///
    /// Include wins over exclude: a candidate that one rule excludes and
    /// another pins survives, deterministically.
    IncludeCandidate {
        /// Exact candidate id to pin.
        id: Option<String>,
        /// Description token identifying candidates to pin.
        tag: Option<String>,
    },
}

impl Action {
    /// Validates structure: a non-empty fact name, or a selector at all.
    pub fn validate(&self) -> EngineResult<()> {
        match self {
            Self::SetFact { fact, .. } => {
                if fact.trim().is_empty() {
                    Err(EngineError::InvalidRule {
                        reason: "set_fact fact name must not be empty".to_owned(),
                    })
                } else {
                    Ok(())
                }
            }
            Self::ExcludeCandidate { id, tag } | Self::IncludeCandidate { id, tag } => {
                if id.as_deref().is_none_or(str::is_empty)
                    && tag.as_deref().is_none_or(str::is_empty)
                {
                    return Err(EngineError::InvalidRule {
                        reason: "candidate selectors need an id or a tag".to_owned(),
                    });
                }
                Ok(())
            }
        }
    }
}

/// One declared rule: a condition plus the actions to take when it holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// Optional human-readable name; unnamed rules are reported as
    /// `rule[<index>]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Predicate over state facts.
    pub when: Condition,
    /// Effects applied in order when `when` holds.
    pub then: Vec<Action>,
}

impl Rule {
    /// Validates the condition and every action.
    pub fn validate(&self) -> EngineResult<()> {
        if self.then.is_empty() {
            return Err(EngineError::InvalidRule {
                reason: "a rule must declare at least one action".to_owned(),
            });
        }
        self.when.validate()?;
        self.then.iter().try_for_each(Action::validate)
    }
}

/// A declared, ordered rule set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RuleSet {
    /// Rules in execution order.
    pub rules: Vec<Rule>,
}

/// The effects of one rule pass over one state.
///
/// Pure data: the executor decides what to do with it, which keeps the
/// rule engine free of graph knowledge and easy to test.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuleReport {
    fired: Vec<String>,
    facts_set: Vec<(String, FactValue)>,
    excluded: BTreeMap<QuestionId, Vec<CandidateId>>,
    pinned: BTreeMap<QuestionId, Vec<CandidateId>>,
}

impl RuleReport {
    /// Names (or `rule[i]` labels) of the rules that fired, in order.
    #[must_use]
    pub fn fired(&self) -> &[String] {
        &self.fired
    }

    /// Facts written by `set_fact`, in execution order.
    #[must_use]
    pub fn facts_set(&self) -> &[(String, FactValue)] {
        &self.facts_set
    }

    /// Candidates excluded per choice question id, in firing order.
    #[must_use]
    pub fn exclusions(&self) -> &BTreeMap<QuestionId, Vec<CandidateId>> {
        &self.excluded
    }

    /// Candidates pinned per choice question id, in firing order.
    #[must_use]
    pub fn pins(&self) -> &BTreeMap<QuestionId, Vec<CandidateId>> {
        &self.pinned
    }

    /// Exclusions that apply to `question`, in firing order.
    #[must_use]
    pub fn exclusions_for(&self, question: &QuestionId) -> &[CandidateId] {
        self.excluded.get(question).map_or(&[], Vec::as_slice)
    }

    /// Pins that apply to `question`, in firing order.
    #[must_use]
    pub fn pins_for(&self, question: &QuestionId) -> &[CandidateId] {
        self.pinned.get(question).map_or(&[], Vec::as_slice)
    }

    /// Writes every `set_fact` effect into `state`, in execution order.
    pub fn apply_to(&self, state: &mut State) {
        for (fact, value) in &self.facts_set {
            let previous = std::mem::take(state);
            *state = previous.with_fact(fact.clone(), value.clone());
        }
    }
}

/// Ordered rule executor.
///
/// Rules run in declared order, and each rule sees the facts written by the
/// rules before it — a rule set is a small sequential program, not a set of
/// independent assertions.
#[derive(Debug, Clone, Default)]
pub struct RuleEngine {
    rules: RuleSet,
}

impl RuleEngine {
    /// Validates and wraps a rule set.
    pub fn new(rules: RuleSet) -> EngineResult<Self> {
        for (index, rule) in rules.rules.iter().enumerate() {
            rule.validate().map_err(|error| match error {
                // Attribute the failure to the offending rule.
                EngineError::InvalidRule { reason } => {
                    EngineError::InvalidRule { reason: format!("rule[{index}]: {reason}") }
                }
                other => other,
            })?;
        }
        Ok(Self { rules })
    }

    /// Number of declared rules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.rules.len()
    }

    /// `true` when no rules are declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.rules.is_empty()
    }

    /// The declared rules, in execution order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules.rules
    }

    /// Evaluates every rule in order against `state`.
    ///
    /// Pure: `state` is read, effects are reported. Facts written by
    /// `set_fact` are visible to subsequent rules within the pass and are
    /// reported for the caller to apply.
    ///
    /// Only choice questions are affected by candidate selectors; boolean
    /// and score questions have no candidate set to narrow.
    #[must_use]
    pub fn evaluate(&self, state: &State, questions: &[DecisionQuestion]) -> RuleReport {
        let mut working = state.clone();
        let mut report = RuleReport::default();
        for (index, rule) in self.rules.rules.iter().enumerate() {
            if !rule.when.matches(&working) {
                continue;
            }
            report.fired.push(rule.name.clone().unwrap_or_else(|| format!("rule[{index}]")));
            for action in &rule.then {
                match action {
                    Action::SetFact { fact, value } => {
                        working = working.with_fact(fact.clone(), value.clone());
                        report.facts_set.push((fact.clone(), value.clone()));
                    }
                    Action::ExcludeCandidate { id, tag } => {
                        collect_matches(
                            questions,
                            id.as_deref(),
                            tag.as_deref(),
                            &mut report.excluded,
                        );
                    }
                    Action::IncludeCandidate { id, tag } => {
                        collect_matches(
                            questions,
                            id.as_deref(),
                            tag.as_deref(),
                            &mut report.pinned,
                        );
                    }
                }
            }
        }
        report
    }
}

/// Records candidates matching a selector for every choice question.
fn collect_matches(
    questions: &[DecisionQuestion],
    id: Option<&str>,
    tag: Option<&str>,
    into: &mut BTreeMap<QuestionId, Vec<CandidateId>>,
) {
    for question in questions {
        let DecisionQuestion::Choice(choice) = question else { continue };
        for candidate in choice.candidates() {
            if matches_selector(candidate, id, tag) {
                let bucket = into.entry(choice.id().clone()).or_default();
                if !bucket.contains(candidate.id()) {
                    bucket.push(candidate.id().clone());
                }
            }
        }
    }
}

/// `true` when `candidate` is selected by the `id`/`tag` selector.
///
/// A tag matches the candidate id exactly, or a token of the description.
/// This is exact string matching against declared configuration — not
/// semantic inference — which is why it counts as deterministic evidence
/// for narrowing (PLANNING.md §45).
#[must_use]
pub fn matches_selector(candidate: &Candidate, id: Option<&str>, tag: Option<&str>) -> bool {
    let id_match = id.is_some_and(|id| candidate.id().as_str() == id);
    let tag_match = tag.is_some_and(|tag| {
        candidate.id().as_str() == tag
            || tokenize(candidate.description()).contains(&tag.to_lowercase())
    });
    id_match || tag_match
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{Candidate, ChoiceQuestion};

    fn state() -> State {
        State::from_text("Refactor the parser and add tests")
            .with_fact("context_tokens", FactValue::Integer(120_000))
            .with_fact("modality", FactValue::Text("text+image".into()))
            .with_fact("privacy", FactValue::Text("local_only".into()))
            .with_fact("modalities", FactValue::List(vec!["text".into(), "image".into()]))
            .with_fact("needs_vision", FactValue::Boolean(true))
            .with_fact("prior_score", FactValue::float(0.75).expect("finite"))
    }

    fn candidates() -> Vec<Candidate> {
        vec![
            Candidate::new("cloud-large", "long context cloud model").unwrap(),
            Candidate::new("local-small", "small context local model").unwrap(),
        ]
    }

    fn choice_question() -> DecisionQuestion {
        DecisionQuestion::Choice(
            ChoiceQuestion::new("model", "Which model?", candidates()).expect("valid"),
        )
    }

    fn rule(name: &str, when: Condition, then: Vec<Action>) -> Rule {
        Rule { name: Some(name.to_owned()), when, then }
    }

    #[test]
    fn fact_conditions_evaluate_against_state() {
        let state = state();
        assert!(Condition::FactExists { fact: "privacy".into() }.matches(&state));
        assert!(!Condition::FactExists { fact: "absent".into() }.matches(&state));
        assert!(
            Condition::FactEquals {
                fact: "privacy".into(),
                value: FactValue::Text("local_only".into())
            }
            .matches(&state)
        );
        assert!(
            !Condition::FactEquals {
                fact: "privacy".into(),
                value: FactValue::Text("cloud".into())
            }
            .matches(&state)
        );
        assert!(
            Condition::FactGt { fact: "context_tokens".into(), value: 100_000.0 }.matches(&state)
        );
        assert!(
            !Condition::FactGt { fact: "context_tokens".into(), value: 200_000.0 }.matches(&state)
        );
        assert!(
            Condition::FactGte { fact: "context_tokens".into(), value: 120_000.0 }.matches(&state)
        );
        assert!(Condition::FactLt { fact: "prior_score".into(), value: 0.9 }.matches(&state));
        assert!(Condition::FactLte { fact: "prior_score".into(), value: 0.75 }.matches(&state));
        assert!(
            Condition::FactContains { fact: "modality".into(), needle: "image".into() }
                .matches(&state)
        );
        assert!(
            Condition::ListContains { fact: "modalities".into(), item: "image".into() }
                .matches(&state)
        );
        assert!(
            !Condition::ListContains { fact: "modalities".into(), item: "video".into() }
                .matches(&state)
        );
    }

    #[test]
    fn composite_conditions_short_circuit_correctly() {
        let state = state();
        let all = Condition::All {
            conditions: vec![
                Condition::FactGt { fact: "context_tokens".into(), value: 1.0 },
                Condition::FactEquals {
                    fact: "privacy".into(),
                    value: FactValue::Text("local_only".into()),
                },
            ],
        };
        assert!(all.matches(&state));
        assert!(!Condition::Not { condition: Box::new(all) }.matches(&state));
        assert!(!Condition::Any { conditions: vec![] }.matches(&state));
        let any = Condition::Any {
            conditions: vec![
                Condition::FactExists { fact: "absent".into() },
                Condition::FactExists { fact: "privacy".into() },
            ],
        };
        assert!(any.matches(&state));
    }

    #[test]
    fn missing_facts_never_match() {
        let state = State::from_text("plain text");
        assert!(!Condition::FactGt { fact: "nope".into(), value: 0.0 }.matches(&state));
        assert!(
            !Condition::FactContains { fact: "nope".into(), needle: "x".into() }.matches(&state)
        );
        // A text fact is not numeric, so comparisons are false.
        assert!(!Condition::FactGt { fact: "modality".into(), value: 0.0 }.matches(&state));
        assert!(!Condition::FactGte { fact: "modality".into(), value: 0.0 }.matches(&state));
        assert!(!Condition::FactLt { fact: "modality".into(), value: 0.0 }.matches(&state));
        assert!(!Condition::FactLte { fact: "modality".into(), value: 0.0 }.matches(&state));
    }

    #[test]
    fn inclusive_comparisons_admit_the_boundary_and_strict_ones_do_not() {
        let state = state();
        assert!(
            !Condition::FactGt { fact: "context_tokens".into(), value: 120_000.0 }.matches(&state)
        );
        assert!(
            Condition::FactGte { fact: "context_tokens".into(), value: 120_000.0 }.matches(&state)
        );
        assert!(
            !Condition::FactGte { fact: "context_tokens".into(), value: 200_000.0 }.matches(&state)
        );
        assert!(
            !Condition::FactLt { fact: "context_tokens".into(), value: 120_000.0 }.matches(&state)
        );
        assert!(
            Condition::FactLte { fact: "context_tokens".into(), value: 120_000.0 }.matches(&state)
        );
        assert!(!Condition::FactLte { fact: "prior_score".into(), value: 0.5 }.matches(&state));
    }

    #[test]
    fn unorderable_comparisons_never_match() {
        // A NaN threshold compares as unordered, which is false rather than
        // an error: a rule set is total over any input.
        let state = state();
        assert!(!Condition::FactGt { fact: "prior_score".into(), value: f64::NAN }.matches(&state));
        assert!(
            !Condition::FactGte { fact: "prior_score".into(), value: f64::NAN }.matches(&state)
        );
        assert!(!Condition::FactLt { fact: "prior_score".into(), value: f64::NAN }.matches(&state));
        assert!(
            !Condition::FactLte { fact: "prior_score".into(), value: f64::NAN }.matches(&state)
        );
    }

    #[test]
    fn validation_rejects_malformed_rules() {
        assert!(matches!(
            Condition::FactGt { fact: String::new(), value: 1.0 }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(matches!(
            Condition::FactGt { fact: "k".into(), value: f64::NAN }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(matches!(
            Condition::FactContains { fact: "k".into(), needle: String::new() }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(matches!(
            Condition::All { conditions: vec![] }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(matches!(
            Action::ExcludeCandidate { id: None, tag: None }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(matches!(
            Rule { name: None, when: Condition::FactExists { fact: "a".into() }, then: vec![] }
                .validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(
            Action::SetFact { fact: "k".into(), value: FactValue::Integer(1) }.validate().is_ok()
        );
    }

    #[test]
    fn every_condition_shape_validates() {
        let exists = || Condition::FactExists { fact: "k".into() };

        // Each numeric comparison validates its own arm.
        for condition in [
            Condition::FactGt { fact: "k".into(), value: 1.0 },
            Condition::FactGte { fact: "k".into(), value: 1.0 },
            Condition::FactLt { fact: "k".into(), value: 1.0 },
            Condition::FactLte { fact: "k".into(), value: 1.0 },
        ] {
            assert!(condition.validate().is_ok(), "{condition:?}");
        }
        assert!(
            Condition::FactGte { fact: "  ".into(), value: 1.0 }.validate().is_err(),
            "a blank fact name is rejected by every comparison"
        );
        assert!(Condition::FactLt { fact: "k".into(), value: f64::INFINITY }.validate().is_err());
        assert!(
            Condition::FactLte { fact: "k".into(), value: f64::NEG_INFINITY }.validate().is_err()
        );

        // Text, list, and existence conditions.
        assert!(
            Condition::FactEquals { fact: "k".into(), value: FactValue::Integer(1) }
                .validate()
                .is_ok()
        );
        assert!(exists().validate().is_ok());
        assert!(
            Condition::FactContains { fact: "k".into(), needle: "needle".into() }
                .validate()
                .is_ok()
        );
        assert!(
            Condition::ListContains { fact: "k".into(), item: "text".into() }.validate().is_ok()
        );
        assert!(matches!(
            Condition::ListContains { fact: "k".into(), item: String::new() }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));

        // Composites validate their sub-conditions, so a non-empty one is
        // accepted and an empty one is not.
        assert!(Condition::All { conditions: vec![exists()] }.validate().is_ok());
        assert!(Condition::Any { conditions: vec![exists(), exists()] }.validate().is_ok());
        assert!(matches!(
            Condition::Any { conditions: vec![] }.validate(),
            Err(EngineError::InvalidRule { .. })
        ));
        assert!(Condition::Not { condition: Box::new(exists()) }.validate().is_ok());
        assert!(
            Condition::Not { condition: Box::new(Condition::All { conditions: vec![] }) }
                .validate()
                .is_err(),
            "a negated condition is still validated"
        );
    }

    #[test]
    fn actions_validate_their_selectors() {
        assert!(
            Action::SetFact { fact: "   ".into(), value: FactValue::Integer(1) }
                .validate()
                .is_err()
        );
        assert!(Action::IncludeCandidate { id: None, tag: None }.validate().is_err());
        assert!(
            Action::IncludeCandidate { id: Some(String::new()), tag: None }.validate().is_err()
        );
        assert!(
            Action::ExcludeCandidate { id: Some("cloud-large".into()), tag: None }
                .validate()
                .is_ok()
        );
        assert!(
            Action::IncludeCandidate { id: None, tag: Some("large".into()) }.validate().is_ok()
        );
        assert!(
            Action::IncludeCandidate { id: Some("cloud-large".into()), tag: Some(String::new()) }
                .validate()
                .is_ok(),
            "an id satisfies the selector even with a blank tag"
        );
    }

    #[test]
    fn declared_rules_are_exposed_in_execution_order() {
        let rules = RuleSet {
            rules: vec![
                rule(
                    "first",
                    Condition::FactExists { fact: "privacy".into() },
                    vec![Action::SetFact { fact: "a".into(), value: FactValue::Integer(1) }],
                ),
                rule(
                    "second",
                    Condition::FactExists { fact: "absent".into() },
                    vec![Action::SetFact { fact: "b".into(), value: FactValue::Integer(2) }],
                ),
            ],
        };
        let engine = RuleEngine::new(rules.clone()).unwrap();
        assert_eq!(engine.rules(), rules.rules.as_slice());
        assert_eq!(engine.len(), 2);
        assert!(!engine.is_empty());
    }

    #[test]
    fn rules_execute_in_declared_order_and_see_earlier_facts() {
        let rules = RuleSet {
            rules: vec![
                rule(
                    "long context",
                    Condition::FactGt { fact: "context_tokens".into(), value: 100_000.0 },
                    vec![Action::SetFact {
                        fact: "requires_long_context".into(),
                        value: FactValue::Boolean(true),
                    }],
                ),
                rule(
                    "exclude small context",
                    // Sees the fact written by the previous rule.
                    Condition::FactEquals {
                        fact: "requires_long_context".into(),
                        value: FactValue::Boolean(true),
                    },
                    vec![Action::ExcludeCandidate { id: None, tag: Some("small".into()) }],
                ),
            ],
        };
        let engine = RuleEngine::new(rules).unwrap();
        let report = engine.evaluate(&state(), &[choice_question()]);
        assert_eq!(
            report.fired(),
            &["long context".to_owned(), "exclude small context".to_owned()]
        );
        assert_eq!(report.facts_set().len(), 1);
        assert_eq!(
            report.exclusions_for(&QuestionId::new("model").unwrap()),
            &[CandidateId::new("local-small").unwrap()]
        );
        assert!(report.pins().is_empty());

        // The same facts must be appliable to a working state.
        let mut working = state();
        report.apply_to(&mut working);
        assert_eq!(working.fact("requires_long_context"), Some(&FactValue::Boolean(true)));
    }

    #[test]
    fn unmatched_rules_produce_an_empty_report() {
        let engine = RuleEngine::new(RuleSet {
            rules: vec![rule(
                "never",
                Condition::FactExists { fact: "absent".into() },
                vec![Action::SetFact { fact: "x".into(), value: FactValue::Integer(1) }],
            )],
        })
        .unwrap();
        let report = engine.evaluate(&state(), &[choice_question()]);
        assert!(report.fired().is_empty());
        assert!(report.facts_set().is_empty());
        assert!(report.exclusions().is_empty());
    }

    #[test]
    fn include_wins_over_exclude() {
        let engine = RuleEngine::new(RuleSet {
            rules: vec![
                rule(
                    "no cloud",
                    Condition::FactExists { fact: "privacy".into() },
                    vec![Action::ExcludeCandidate { id: Some("cloud-large".into()), tag: None }],
                ),
                rule(
                    "but keep it if vision is needed",
                    Condition::FactExists { fact: "needs_vision".into() },
                    vec![Action::IncludeCandidate { id: None, tag: Some("long".into()) }],
                ),
            ],
        })
        .unwrap();
        let report = engine.evaluate(&state(), &[choice_question()]);
        assert_eq!(
            report.exclusions_for(&QuestionId::new("model").unwrap()),
            &[CandidateId::new("cloud-large").unwrap()]
        );
        assert_eq!(
            report.pins_for(&QuestionId::new("model").unwrap()),
            &[CandidateId::new("cloud-large").unwrap()]
        );
    }

    #[test]
    fn tag_selectors_match_description_tokens_and_ids() {
        let candidate = Candidate::new("local-small", "small context LOCAL model").unwrap();
        assert!(matches_selector(&candidate, Some("local-small"), None));
        assert!(matches_selector(&candidate, None, Some("local-small")));
        assert!(matches_selector(&candidate, None, Some("small")), "tag match is case-insensitive");
        assert!(!matches_selector(&candidate, None, Some("smal")));
        assert!(!matches_selector(&candidate, Some("other"), None));
    }

    #[test]
    fn rules_ignore_non_choice_questions() {
        let engine = RuleEngine::new(RuleSet {
            rules: vec![rule(
                "all",
                Condition::FactExists { fact: "privacy".into() },
                vec![Action::ExcludeCandidate { id: None, tag: Some("local".into()) }],
            )],
        })
        .unwrap();
        let questions = vec![
            DecisionQuestion::Boolean(
                opencodifier_core::BooleanQuestion::new("tools", "Needs tools?").unwrap(),
            ),
            choice_question(),
        ];
        let report = engine.evaluate(&state(), &questions);
        assert!(report.exclusions().contains_key(&QuestionId::new("model").unwrap()));
        assert!(!report.exclusions().contains_key(&QuestionId::new("tools").unwrap()));
    }

    #[test]
    fn rule_engine_attributes_validation_errors_to_the_rule_index() {
        let error = RuleEngine::new(RuleSet {
            rules: vec![
                rule(
                    "fine",
                    Condition::FactExists { fact: "a".into() },
                    vec![Action::SetFact { fact: "b".into(), value: FactValue::Integer(1) }],
                ),
                Rule { name: None, when: Condition::FactExists { fact: "a".into() }, then: vec![] },
            ],
        })
        .unwrap_err();
        assert_eq!(error.code(), "rules.invalid");
        assert!(error.to_string().contains("rule[1]"), "{error}");
    }

    #[test]
    fn unnamed_rules_are_labelled_by_index() {
        let engine = RuleEngine::new(RuleSet {
            rules: vec![Rule {
                name: None,
                when: Condition::FactExists { fact: "privacy".into() },
                then: vec![Action::SetFact { fact: "x".into(), value: FactValue::Integer(1) }],
            }],
        })
        .unwrap();
        let report = engine.evaluate(&state(), &[choice_question()]);
        assert_eq!(report.fired(), &["rule[0]".to_owned()]);
    }

    #[test]
    fn empty_rule_set_is_valid_and_inert() {
        let engine = RuleEngine::new(RuleSet::default()).unwrap();
        assert!(engine.is_empty());
        assert_eq!(engine.len(), 0);
        assert!(engine.evaluate(&state(), &[choice_question()]).fired().is_empty());
    }

    #[test]
    fn rule_sets_round_trip_through_json() {
        let rules = RuleSet {
            rules: vec![rule(
                "long context",
                Condition::FactGt { fact: "context_tokens".into(), value: 100_000.0 },
                vec![
                    Action::SetFact { fact: "long".into(), value: FactValue::Boolean(true) },
                    Action::ExcludeCandidate { id: None, tag: Some("small".into()) },
                    Action::IncludeCandidate { id: Some("cloud-large".into()), tag: None },
                ],
            )],
        };
        let json = serde_json::to_string_pretty(&rules).unwrap();
        let back: RuleSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rules);
        assert!(json.contains(r#""op": "fact_gt""#), "{json}");
    }
}
