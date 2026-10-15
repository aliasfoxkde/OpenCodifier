//! Rendering sampled facts into state text the engine's extraction
//! grammar accepts exactly.
//!
//! The grammar (opencodifier-engine `facts`) is the serving contract:
//! whatever the rung proves at inference it must extract from text in
//! this exact shape, so the corpus states are drawn from that same
//! shape — varying only what the grammar leaves free (sentence order,
//! status synonyms, which facts a graph mixes). Rendering never
//! capitalizes sentence-initial entity names: names are kept verbatim
//! so a rendered fact re-extracts to the identical structure.

use opencodifier_engine::facts::{Health, RelationalFact};

use crate::rng::Rng;
use crate::sample::{Sampled, question_text};

/// The word for a [`Health`] value as stated in text. `Down` has three
/// synonyms in the grammar and picks among them for surface diversity;
/// the other statuses have one word each.
#[must_use]
pub fn status_word(status: Health, rng: &mut Rng) -> &'static str {
    match status {
        Health::Healthy => "healthy",
        Health::Degraded => "degraded",
        Health::Down => crate::vocab::DOWN_WORDS[rng.below(crate::vocab::DOWN_WORDS.len())],
    }
}

/// One fact as a grammar-exact sentence (no trailing period).
#[must_use]
pub fn fact_sentence(fact: &RelationalFact, rng: &mut Rng) -> String {
    match fact {
        RelationalFact::DependsOn { dependent, dependency } => {
            format!("{dependent} depends on {dependency}")
        }
        RelationalFact::Health { entity, status } => {
            format!("{entity} is {}", status_word(*status, rng))
        }
        RelationalFact::HealthList { entity, statuses } => {
            let words: Vec<&str> =
                statuses.iter().map(|status| status_word(*status, rng)).collect();
            format!("{entity}: {}", words.join(", "))
        }
        RelationalFact::RestoresAfter { later, earlier } => {
            format!("{later} comes back online only after {earlier}")
        }
    }
}

/// The fixed-form sentence for a fact — the round-trip canonical form
/// used by [`crate::verify`], independent of draw-time synonyms.
#[must_use]
pub fn canonical_sentence(fact: &RelationalFact) -> String {
    let word = |status: Health| match status {
        Health::Healthy => "healthy",
        Health::Degraded => "degraded",
        Health::Down => "failing",
    };
    match fact {
        RelationalFact::DependsOn { dependent, dependency } => {
            format!("{dependent} depends on {dependency}")
        }
        RelationalFact::Health { entity, status } => {
            format!("{entity} is {}", word(*status))
        }
        RelationalFact::HealthList { entity, statuses } => {
            let words: Vec<&str> = statuses.iter().map(|status| word(*status)).collect();
            format!("{entity}: {}", words.join(", "))
        }
        RelationalFact::RestoresAfter { later, earlier } => {
            format!("{later} comes back online only after {earlier}")
        }
    }
}

/// Renders the state text: all facts stated once, in a per-item random
/// sentence order, joined with periods.
#[must_use]
pub fn state_text(facts: &[RelationalFact], rng: &mut Rng) -> String {
    let mut sentences: Vec<String> = facts.iter().map(|f| fact_sentence(f, rng)).collect();
    rng.shuffle(&mut sentences);
    format!("{}.", sentences.join(". "))
}

/// The question key a family's record is filed under.
#[must_use]
pub fn question_key(family: crate::sample::Family) -> &'static str {
    match family {
        crate::sample::Family::RootCauseChain | crate::sample::Family::RootCauseAgreeing => {
            "root_cause"
        }
        crate::sample::Family::HealthiestBoard => "healthiest",
        crate::sample::Family::FirstRestoredChain => "first_restored",
        crate::sample::Family::ScoreBoard => "health_score",
    }
}

/// A short family code for record ids.
#[must_use]
pub fn family_code(family: crate::sample::Family) -> &'static str {
    match family {
        crate::sample::Family::RootCauseChain => "rcc",
        crate::sample::Family::RootCauseAgreeing => "rca",
        crate::sample::Family::HealthiestBoard => "hb",
        crate::sample::Family::FirstRestoredChain => "frc",
        crate::sample::Family::ScoreBoard => "sb",
    }
}

/// Assembles the corpus record for a sampled item — the same
/// record/request/target shape `decision_sft_prep` consumes from every
/// other corpus source, so generated items flow through the unchanged
/// prep and its rendering path. Choice criteria serialize in sorted-key
/// order (the suite's own ballot convention); the score family carries
/// the fixed 4-level scale as a LIST (index = level, the prep's score
/// shape) and the gold as the level-index string.
#[must_use]
pub fn corpus_record(sampled: &Sampled, state: &str, record_id: &str) -> serde_json::Value {
    let question = question_text(sampled);
    let key = question_key(sampled.family);
    if sampled.family == crate::sample::Family::ScoreBoard {
        // The record gold is the rubric level over the constructed
        // facts — never a parse of a string the sampler wrote, so the
        // record cannot disagree with its own fact set.
        let (healthy, degraded, down) = crate::sample::count_statuses(&sampled.facts);
        let target_label = crate::sample::health_level(healthy, degraded, down).to_string();
        return serde_json::json!({
            "record_id": record_id,
            "source": sampled.family.source_tag(),
            "request": {
                "state": state,
                "questions": {
                    key: {
                        "type": "score",
                        "instructions": question,
                        "criteria": crate::vocab::SCORE_LEVELS,
                    }
                }
            },
            "target": {
                key: {
                    "type": "score",
                    "label": target_label,
                }
            }
        });
    }
    let noun = sampled.domain.noun;
    let mut criteria = serde_json::Map::new();
    let mut ballot: Vec<&str> = vec![&sampled.gold];
    for d in &sampled.distractors {
        ballot.push(&d.name);
    }
    for name in ballot {
        criteria.insert(name.to_owned(), serde_json::Value::String(format!("{noun} {name}")));
    }
    serde_json::json!({
        "record_id": record_id,
        "source": sampled.family.source_tag(),
        "request": {
            "state": state,
            "questions": {
                key: {
                    "type": "choice",
                    "instructions": question,
                    "criteria": criteria,
                }
            }
        },
        "target": {
            key: {
                "type": "choice",
                "label": sampled.gold,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::rng::Rng;
    use crate::sample::{Family, sample};

    #[test]
    fn rendered_sentences_are_lowercase_and_period_joined() {
        let mut rng = Rng::new(13);
        let sampled = sample(Family::RootCauseChain, &mut rng);
        let text = state_text(&sampled.facts, &mut rng);
        assert!(text.ends_with('.'));
        for sentence in text.trim_end_matches('.').split(". ") {
            let first = sentence.chars().next().unwrap();
            assert!(
                first.is_ascii_lowercase() || first.is_ascii_digit(),
                "sentence starts capitalized: {sentence}"
            );
        }
    }

    #[test]
    fn corpus_record_shape_matches_the_prep_contract() {
        let mut rng = Rng::new(14);
        let sampled = sample(Family::RootCauseAgreeing, &mut rng);
        let text = state_text(&sampled.facts, &mut rng);
        let record = corpus_record(&sampled, &text, "ig-test-000001");
        assert_eq!(record["source"], "itemgen/root-cause-agreeing");
        assert_eq!(record["target"]["root_cause"]["label"], sampled.gold);
        assert_eq!(record["request"]["questions"]["root_cause"]["type"], "choice");
        let criteria =
            record["request"]["questions"]["root_cause"]["criteria"].as_object().unwrap();
        assert_eq!(criteria.len(), 1 + sampled.distractors.len());
        assert!(criteria.values().all(|v| v.as_str().unwrap().starts_with(sampled.domain.noun)));
    }
}
