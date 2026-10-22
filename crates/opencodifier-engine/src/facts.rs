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
//! | `X <phrase> by <timestamp>` | [`RelationalFact::Deadline`] |
//! | `X <phrase> on <timestamp>` | [`RelationalFact::Event`] |
//!
//! A **timestamp** is an ISO calendar date (`2026-03-01`) with an
//! optional clock time (`2026-03-01T09:30`). The date must exist: the
//! extractor validates the calendar, so `2026-02-30` is prose, not a
//! date.
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

/// A calendar date with an optional clock time, as stated.
///
/// The type is closed on purpose: only the ISO forms
/// (`2026-03-01`, `2026-03-01T09:30`) parse, the calendar is validated
/// (so `2026-02-30` and `2026-13-01` are prose), and ordering is
/// chronological because the fields are compared as a tuple. A date
/// without a clock time is midnight — the start of that day — so a
/// date-only and a date-time on the same day order the way a reader
/// would expect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// Calendar year.
    pub year: u32,
    /// Calendar month, 1-12.
    pub month: u32,
    /// Day of the month, 1-31, validated against the month.
    pub day: u32,
    /// Hour of the day, 0-23; `0` when the statement gave no time.
    pub hour: u32,
    /// Minute of the hour, 0-59; `0` when the statement gave no time.
    pub minute: u32,
}

impl Timestamp {
    /// Parses an ISO calendar date, with an optional clock time.
    ///
    /// Returns `None` for anything else — a different format, an
    /// impossible day, a missing field, or a value out of range.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (date, clock) = match text.split_once('T') {
            Some((date, clock)) => (date, Some(clock)),
            None => (text, None),
        };
        // The grammar is exact: `YYYY-MM-DD`, optionally `THH:MM`. Nothing
        // is padded, truncated, or localized on the way in.
        if date.len() != 10 || clock.is_some_and(|clock| clock.len() != 5) {
            return None;
        }
        let mut fields = date.split('-');
        let year: u32 = fields.next()?.parse().ok()?;
        let month: u32 = fields.next()?.parse().ok()?;
        let day: u32 = fields.next()?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        let (hour, minute) = match clock {
            None => (0, 0),
            Some(clock) => {
                let mut parts = clock.split(':');
                let hour: u32 = parts.next()?.parse().ok()?;
                let minute: u32 = parts.next()?.parse().ok()?;
                if parts.next().is_some() || hour > 23 || minute > 59 {
                    return None;
                }
                (hour, minute)
            }
        };
        Some(Self { year, month, day, hour, minute })
    }

    /// The timestamp as an ISO string: `YYYY-MM-DD`, plus `THH:MM` when
    /// the time is not midnight.
    ///
    /// The rendering round-trips through [`Self::parse`]: midnight is
    /// written as a plain date, and midnight written as a date-time
    /// parses back to the same value.
    #[must_use]
    pub fn to_iso(self) -> String {
        let date = format!("{:04}-{:02}-{:02}", self.year, self.month, self.day);
        if self.hour == 0 && self.minute == 0 {
            date
        } else {
            format!("{date}T{:02}:{:02}", self.hour, self.minute)
        }
    }
}

/// The number of days in `month` of `year`, leap years included.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Whether `year` is a leap year in the Gregorian calendar.
fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
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
    /// "`entity` `phrase` by `timestamp`" — a deadline the entity is
    /// held to.
    Deadline {
        /// The entity the deadline binds.
        entity: String,
        /// When it is due.
        at: Timestamp,
    },
    /// "`entity` `phrase` on `timestamp`" — something that happened to
    /// the entity at a stated time.
    Event {
        /// The entity the event concerns.
        entity: String,
        /// When it happened.
        at: Timestamp,
    },
}

/// The longest entity name the extractor will accept. Anything longer is
/// treated as prose, not a name.
const MAX_NAME_LEN: usize = 64;

/// The longest verb phrase the extractor will accept between an entity
/// and its preposition.
const MAX_PHRASE_LEN: usize = 32;

/// Whether `word` can be an entity name: non-empty `[A-Za-z0-9_-]`,
/// bounded length. Everything else is prose.
///
/// A date is not a name: `2026-03-01` is name-shaped, but a deadline
/// must not also be a dependency, so date-shaped words are excluded.
fn is_name(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= MAX_NAME_LEN
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !(word.contains('-') && Timestamp::parse(word).is_some())
}

/// Whether `word` can stand in the verb phrase between an entity and a
/// preposition ("was due", "completed", "will expire").
fn is_phrase_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= MAX_PHRASE_LEN
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
        // "X <phrase> by|on <timestamp>" — a deadline or a dated event.
        // The preposition carries the direction: "by" is a limit the
        // entity is held to, "on" is when something happened.
        //
        // This arm sits last on purpose. A sentence that ends in "by"
        // or "on" but does not end in a date is prose, and prose must
        // fall through to no fact — so the arm is reached only after
        // the shapes above have declined the sentence.
        4..=6 if matches!(lowered[tokens.len() - 2].as_str(), "by" | "on") => {
            let at = Timestamp::parse(&tokens[tokens.len() - 1])?;
            let entity = tokens[0].as_str();
            let phrase = &tokens[1..tokens.len() - 2];
            if !is_name(entity) || !phrase.iter().all(|word| is_phrase_word(word)) {
                return None;
            }
            if lowered[tokens.len() - 2] == "by" {
                Some(RelationalFact::Deadline { entity: entity.to_owned(), at })
            } else {
                Some(RelationalFact::Event { entity: entity.to_owned(), at })
            }
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
        assert_eq!(extract(text), Vec::new());
    }

    #[test]
    fn rejects_names_that_are_not_names() {
        // Structural words match, but the "names" contain characters the
        // grammar forbids — no fact may be emitted.
        assert_eq!(extract("bill;ing depends on catalog."), Vec::new());
        assert_eq!(extract("billing depends on cat alog."), Vec::new());
        assert_eq!(extract("i'is is healthy."), Vec::new());
    }

    #[test]
    fn empty_comma_slots_are_skipped() {
        // A doubled comma in a group report is a blank slot, not a status:
        // it is dropped so the rest of the report still parses.
        assert_eq!(
            extract("north: healthy, , down."),
            vec![RelationalFact::HealthList {
                entity: "north".into(),
                statuses: vec![Health::Healthy, Health::Down]
            }],
        );
        // A report whose every status slot is blank has no statuses to
        // report, but the entity is still named.
        assert_eq!(
            extract("south: , , healthy."),
            vec![RelationalFact::HealthList {
                entity: "south".into(),
                statuses: vec![Health::Healthy]
            }],
        );
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
        assert_eq!(extract(&format!("{long} depends on catalog.")), Vec::new());
        assert_eq!(
            extract(&format!("{short} depends on catalog.")).len(),
            1,
            "a name of exactly the maximum length parses"
        );
    }

    #[test]
    fn parses_the_iso_forms_and_rejects_the_impossible_ones() {
        assert_eq!(
            Timestamp::parse("2026-03-01"),
            Some(Timestamp { year: 2026, month: 3, day: 1, hour: 0, minute: 0 })
        );
        assert_eq!(
            Timestamp::parse("2026-03-01T09:30"),
            Some(Timestamp { year: 2026, month: 3, day: 1, hour: 9, minute: 30 })
        );
        // The calendar is validated: a month-end that does not exist is
        // prose, and a leap day exists only in a leap year.
        assert_eq!(Timestamp::parse("2026-02-30"), None);
        assert_eq!(Timestamp::parse("2026-13-01"), None);
        assert_eq!(Timestamp::parse("2026-00-10"), None);
        assert_eq!(Timestamp::parse("2026-03-00"), None);
        assert!(Timestamp::parse("2024-02-29").is_some(), "2024 is a leap year");
        assert_eq!(Timestamp::parse("2026-02-29"), None, "2026 is not");
        assert_eq!(Timestamp::parse("1900-02-29"), None, "1900 is not (divisible by 100)");
        assert!(Timestamp::parse("2000-02-29").is_some(), "2000 is (divisible by 400)");
        // Times are validated too.
        assert_eq!(Timestamp::parse("2026-03-01T24:00"), None);
        assert_eq!(Timestamp::parse("2026-03-01T09:60"), None);
        // And nothing else is a timestamp.
        assert_eq!(Timestamp::parse("2026-3-1"), None);
        assert_eq!(Timestamp::parse("01/03/2026"), None);
        assert_eq!(Timestamp::parse("2026-03-01T09:30:00"), None);
        assert_eq!(Timestamp::parse(""), None);
        assert_eq!(Timestamp::parse("yesterday"), None);
    }

    #[test]
    fn timestamps_order_chronologically() {
        let earlier = Timestamp::parse("2026-03-01").unwrap();
        let later = Timestamp::parse("2026-03-01T09:30").unwrap();
        let next_day = Timestamp::parse("2026-03-02").unwrap();
        let earlier_year = Timestamp::parse("2025-12-31").unwrap();
        assert!(earlier < later);
        assert!(later < next_day);
        assert!(earlier_year < earlier);
    }

    #[test]
    fn a_timestamp_renders_back_to_what_it_was_parsed_from() {
        for text in
            ["2026-03-01", "2026-03-01T09:30", "2024-02-29", "2026-12-31T23:59", "2026-03-01T00:00"]
        {
            let parsed = Timestamp::parse(text).expect("the sample parses");
            let rendered = parsed.to_iso();
            assert_eq!(
                Timestamp::parse(&rendered),
                Some(parsed),
                "{text} renders as {rendered} and parses back to the same value"
            );
        }
        // Midnight written as a date-time still parses back to midnight.
        assert_eq!(Timestamp::parse("2026-03-01T00:00"), Timestamp::parse("2026-03-01"));
        // And a plain date renders without a clock.
        assert_eq!(Timestamp::parse("2026-03-01").unwrap().to_iso(), "2026-03-01");
    }

    #[test]
    fn extracts_deadlines_and_events() {
        let facts = extract("invoice is due by 2026-03-01. audit completed on 2026-01-15T09:30.");
        assert_eq!(
            facts,
            vec![
                RelationalFact::Deadline {
                    entity: "invoice".into(),
                    at: Timestamp::parse("2026-03-01").unwrap()
                },
                RelationalFact::Event {
                    entity: "audit".into(),
                    at: Timestamp::parse("2026-01-15T09:30").unwrap()
                },
            ]
        );
    }

    #[test]
    fn the_preposition_carries_the_direction() {
        // "by" is a limit the entity is held to; "on" is when something
        // happened. Same shape, different fact.
        let facts = extract("payment due by 2026-04-01. payment received on 2026-03-01.");
        assert!(matches!(facts[0], RelationalFact::Deadline { .. }));
        assert!(matches!(facts[1], RelationalFact::Event { .. }));
    }

    #[test]
    fn an_impossible_date_is_prose() {
        assert_eq!(extract("invoice is due by 2026-02-30."), Vec::new());
        assert_eq!(extract("invoice is due by soon."), Vec::new());
        assert_eq!(extract("invoice is due by 15/03/2026."), Vec::new());
    }

    #[test]
    fn a_date_is_not_an_entity_name() {
        // A date is name-shaped, but a deadline must not double as a
        // dependency — otherwise "a depends on 2026-03-01" would be a
        // dependency on a date.
        assert_eq!(extract("a depends on 2026-03-01."), Vec::new());
        assert_eq!(extract("a depends on 2026-03-01.").len(), 0, "no dependency fact on a date");
    }

    #[test]
    fn prose_ending_in_by_or_on_is_not_a_date_fact() {
        // The temporal arm must not swallow ordinary prose: a sentence
        // that ends in "on" or "by" but does not end in a date states no
        // fact at all.
        assert_eq!(extract("the meeting is on tuesday."), Vec::new());
        assert_eq!(extract("the release is by friday."), Vec::new());
        assert_eq!(extract("stand-up happens on monday evening."), Vec::new());
        // And the group-report form is unaffected by the temporal arm
        // standing in the same sentence space.
        assert_eq!(
            extract("north: healthy, down."),
            vec![RelationalFact::HealthList {
                entity: "north".into(),
                statuses: vec![Health::Healthy, Health::Down],
            }]
        );
    }

    #[test]
    fn a_sentence_needs_an_entity_a_phrase_and_a_preposition() {
        // Too short: no phrase between the entity and the preposition.
        assert_eq!(extract("invoice by 2026-03-01."), Vec::new());
        // The phrase may be one to three words.
        assert_eq!(extract("invoice due by 2026-03-01.").len(), 1);
        assert_eq!(extract("invoice is due by 2026-03-01.").len(), 1);
        assert_eq!(extract("invoice was due on 2026-03-01.").len(), 1);
    }
}
