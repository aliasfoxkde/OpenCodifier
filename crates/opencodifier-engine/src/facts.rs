//! Deterministic relational fact extraction from unstructured state text
//! (PLANNING.md §43: the cheapest reliable mechanism first).
//!
//! Ops state text says relational things in a small number of ways —
//! "billing depends on catalog", "north: healthy, degraded, down",
//! "dispatch comes back online only after gateway". This module turns
//! those sentences into typed facts with an **exact** grammar, so the
//! relational solver ([`crate::RelationalSolver`]) and rules can compute
//! over structure instead of guessing over prose.
//!
//! # The grammar
//!
//! Text is split into sentences on `.`; each sentence is whitespace-
//! tokenized and must match one pattern exactly — there is no fuzzy
//! matching, no partial credit, and no learning:
//!
//! | sentence | fact |
//! |---|---|
//! | `X depends on Y` | [`RelationalFact::DependsOn`] |
//! | `X is healthy\|degraded\|down\|failing` | [`RelationalFact::Health`] |
//! | `X: S, S, …` (statuses) | [`RelationalFact::HealthList`] |
//! | `X comes back online only after Y` | [`RelationalFact::RestoresAfter`] |
//!
//! Everything that does not match a pattern in full is ignored — the
//! extractor is total (never fails, never panics on any input) and
//! conservative (a fact is emitted only when the sentence says exactly
//! that). Structural words and status words match
//! case-insensitively; entity names are kept verbatim and must be
//! non-empty runs of `[A-Za-z0-9_-]` of at most 64 characters, so
//! hostile text can never smuggle structure into a name. (Names cannot
//! contain `.` for the same reason sentences are split on it.)

/// Health of an entity, as stated. `failing` parses to [`Health::Down`]:
/// both mean "not serving".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Health {
    /// Fully serving.
    Healthy,
    /// Serving with reduced capacity.
    Degraded,
    /// Not serving (`down`, `failing`).
    Down,
}

impl Health {
    /// Parses a status word, case-insensitively.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "healthy" | "ok" => Some(Self::Healthy),
            "degraded" => Some(Self::Degraded),
            "down" | "failing" | "offline" => Some(Self::Down),
            _ => None,
        }
    }
}

/// One relational fact extracted from state text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationalFact {
    /// "`dependent` depends on `dependency`" — failure and repair flow
    /// from `dependency` to `dependent`.
    DependsOn {
        /// The component that needs the other.
        dependent: String,
        /// The component that is needed.
        dependency: String,
    },
    /// "`entity` is `<status>`" — a single stated health value.
    Health {
        /// The entity the status describes.
        entity: String,
        /// The stated status.
        status: Health,
    },
    /// "`entity`: s1, s2, …" — a status report over a group (a region's
    /// node list, a fleet's instances).
    HealthList {
        /// The group the report describes.
        entity: String,
        /// The statuses in stated order.
        statuses: Vec<Health>,
    },
    /// "`later` comes back online only after `earlier`" — a bring-up
    /// ordering constraint.
    RestoresAfter {
        /// The component that must wait.
        later: String,
        /// The component that must come up first.
        earlier: String,
    },
}

/// The longest entity name the extractor will accept. Anything longer is
/// treated as prose, not a name.
const MAX_NAME_LEN: usize = 64;

/// Whether `word` can be an entity name: non-empty `[A-Za-z0-9_-]`,
/// bounded length. Everything else is prose.
fn is_name(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= MAX_NAME_LEN
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The sentence's tokens with structural words lowercased and names kept
/// verbatim. `name_slots` positions hold entity names.
fn normalize(sentence: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    for word in sentence.split_whitespace() {
        let trimmed = word.trim_matches(',');
        if trimmed.is_empty() {
            continue;
        }
        tokens.push(trimmed.to_owned());
    }
    (!tokens.is_empty()).then_some(tokens)
}

/// Extracts every relational fact the text states, in text order.
///
/// Total and deterministic: the same text always yields the same facts,
/// and text with no relational sentences yields none.
#[must_use]
pub fn extract(text: &str) -> Vec<RelationalFact> {
    let mut facts = Vec::new();
    for sentence in text.split('.') {
        let Some(tokens) = normalize(sentence) else { continue };
        if let Some(fact) = parse_sentence(&tokens) {
            facts.push(fact);
        }
    }
    facts
}

/// Parses one tokenized sentence into a fact, or `None` when it matches
/// no pattern.
fn parse_sentence(tokens: &[String]) -> Option<RelationalFact> {
    let lowered: Vec<String> = tokens.iter().map(|token| token.to_ascii_lowercase()).collect();
    match tokens.len() {
        // "X depends on Y"
        4 if lowered[1] == "depends" && lowered[2] == "on" => {
            let (dependent, dependency) = (tokens[0].as_str(), tokens[3].as_str());
            (is_name(dependent) && is_name(dependency)).then(|| RelationalFact::DependsOn {
                dependent: dependent.to_owned(),
                dependency: dependency.to_owned(),
            })
        }
        // "X is <status>"
        3 if lowered[1] == "is" => {
            let status = Health::parse(&lowered[2])?;
            let entity = tokens[0].as_str();
            is_name(entity).then(|| RelationalFact::Health { entity: entity.to_owned(), status })
        }
        // "X comes back online only after Y"
        7 if lowered[1..6] == ["comes", "back", "online", "only", "after"] => {
            let (later, earlier) = (tokens[0].as_str(), tokens[6].as_str());
            (is_name(later) && is_name(earlier)).then(|| RelationalFact::RestoresAfter {
                later: later.to_owned(),
                earlier: earlier.to_owned(),
            })
        }
        // "X: s1, s2, …" — the group report form.
        _ if tokens.len() >= 2 && tokens[0].ends_with(':') => {
            let entity = tokens[0].trim_end_matches(':');
            let mut statuses = Vec::new();
            for token in &lowered[1..] {
                statuses.push(Health::parse(token)?);
            }
            is_name(entity)
                .then(|| RelationalFact::HealthList { entity: entity.to_owned(), statuses })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn extracts_dependency_facts() {
        let facts = extract("billing depends on catalog. dispatch depends on billing.");
        assert_eq!(
            facts,
            vec![
                RelationalFact::DependsOn {
                    dependent: "billing".into(),
                    dependency: "catalog".into()
                },
                RelationalFact::DependsOn {
                    dependent: "dispatch".into(),
                    dependency: "billing".into()
                },
            ]
        );
    }

    #[test]
    fn extracts_health_and_failing_as_down() {
        let facts = extract("auth is healthy. billing is Failing. catalog is degraded.");
        assert_eq!(
            facts,
            vec![
                RelationalFact::Health { entity: "auth".into(), status: Health::Healthy },
                RelationalFact::Health { entity: "billing".into(), status: Health::Down },
                RelationalFact::Health { entity: "catalog".into(), status: Health::Degraded },
            ]
        );
    }

    #[test]
    fn extracts_group_reports_with_trailing_periods() {
        let facts = extract("north: healthy, healthy, down. south: degraded.");
        assert_eq!(
            facts,
            vec![
                RelationalFact::HealthList {
                    entity: "north".into(),
                    statuses: vec![Health::Healthy, Health::Healthy, Health::Down]
                },
                RelationalFact::HealthList {
                    entity: "south".into(),
                    statuses: vec![Health::Degraded]
                },
            ]
        );
    }

    #[test]
    fn extracts_restore_ordering() {
        let facts = extract(
            "dispatch comes back online only after gateway. gateway comes back online only after auth.",
        );
        assert_eq!(
            facts,
            vec![
                RelationalFact::RestoresAfter {
                    later: "dispatch".into(),
                    earlier: "gateway".into()
                },
                RelationalFact::RestoresAfter { later: "gateway".into(), earlier: "auth".into() },
            ]
        );
    }

    #[test]
    fn ignores_everything_that_is_not_a_pattern() {
        // Prose, partial patterns, wrong arity, unknown statuses.
        let text = "The billing team depends heavily on catalog uptime. \
                    billing depends on. auth is flapping. north: healthy, flapping. \
                    x comes back online only. 1234";
        assert!(extract(text).is_empty());
    }

    #[test]
    fn rejects_names_that_are_not_names() {
        // Structural words match, but the "names" contain characters the
        // grammar forbids — no fact may be emitted.
        assert!(extract("bill;ing depends on catalog.").is_empty());
        assert!(extract("billing depends on cat alog.").is_empty());
        assert!(extract("i'is is healthy.").is_empty());
    }

    #[test]
    fn is_total_on_hostile_input() {
        for text in ["", ".", "...", "   ", "a".repeat(10_000).as_str(), "depends on is :"] {
            let _ = extract(text); // must not panic; value irrelevant
        }
    }

    #[test]
    fn extracts_from_multiline_state_text() {
        let text = "deploy notes below.\ngateway depends on auth.\nauth is healthy.";
        let facts = extract(text);
        assert_eq!(
            facts,
            vec![
                RelationalFact::DependsOn {
                    dependent: "gateway".into(),
                    dependency: "auth".into()
                },
                RelationalFact::Health { entity: "auth".into(), status: Health::Healthy },
            ]
        );
    }

    #[test]
    fn long_tokens_are_prose_not_names() {
        let long = "a".repeat(MAX_NAME_LEN + 1);
        let short = "a".repeat(MAX_NAME_LEN);
        assert!(extract(&format!("{long} depends on catalog.")).is_empty());
        assert_eq!(
            extract(&format!("{short} depends on catalog.")).len(),
            1,
            "a name of exactly the maximum length parses"
        );
    }
}
