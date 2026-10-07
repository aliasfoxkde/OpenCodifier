//! The vocabulary pools: entity domains, question phrasings, and the
//! surface words each fact kind can be rendered with.
//!
//! Diversity here is the point (TRAINING.md §9.7 B3, after Kabra et
//! al. ICLR 2026: generator *structural* diversity — not more rows from
//! one template — is what makes synthetic supervision transfer). Four
//! entity domains × four phrasings per operator × per-item graph shapes
//! and fact mixes give the corpus families that look different on the
//! surface while sharing one proof structure. The fact sentence shapes
//! themselves are deliberately NOT diversified beyond word choice:
//! `crate::render` may only emit what the engine's extraction grammar
//! accepts, because the serving path extracts with that exact grammar.

/// One entity-naming domain: a code (used in record ids), the noun the
/// question phrasings refer to, and the entity name pool.
#[derive(Debug, Clone, Copy)]
pub struct Domain {
    /// Short code embedded in generated record ids.
    pub code: &'static str,
    /// The noun question phrasings substitute for `{noun}`.
    pub noun: &'static str,
    /// Candidate descriptions are "{noun} {name}" — the suite's own
    /// convention ("component auth").
    pub names: &'static [&'static str],
}

/// The entity domains. Name pools must stay disjoint in spirit from
/// one another and must only contain names the extractor accepts
/// (`[A-Za-z0-9_-]`, no dots).
pub const DOMAINS: &[Domain] = &[
    Domain {
        code: "svc",
        noun: "component",
        names: &[
            "auth", "billing", "gateway", "index", "feed", "dispatch", "export", "catalog",
            "search", "worker",
        ],
    },
    Domain {
        code: "region",
        noun: "region",
        names: &["north", "south", "east", "west", "central", "coast"],
    },
    Domain {
        code: "module",
        noun: "module",
        names: &["parser", "lexer", "runtime", "linker", "driver", "codec", "serial", "fsm"],
    },
    Domain {
        code: "store",
        noun: "store",
        names: &["cache", "ledger", "queue", "blob", "meta", "wal", "tier", "replica"],
    },
];

/// Question phrasings for root-cause items. `{noun}` is substituted
/// with the domain noun at render time.
pub const ROOT_CAUSE_QUESTIONS: &[&str] = &[
    "Fixing which single {noun} restores every part of the chain?",
    "Which {noun} is failing at the root, rather than because of another failure?",
    "Exactly one {noun}'s repair clears every failure at once. Which one?",
    "All of the traced failures come back if one {noun} is repaired. Which?",
];

/// Question phrasings for healthiest-board items.
pub const HEALTHIEST_QUESTIONS: &[&str] = &[
    "Which {noun} is healthiest overall?",
    "Which {noun} reports the most healthy checks?",
    "One {noun} is strictly ahead on healthy reports. Which one?",
    "Which {noun} has the best healthy record on the board?",
];

/// Question phrasings for first-restored items.
pub const FIRST_RESTORED_QUESTIONS: &[&str] = &[
    "Which {noun} comes back online first?",
    "Bring-up starts at exactly one {noun}. Which one?",
    "Which {noun} can be restored without waiting on any other?",
    "Which {noun} gates the rest of the bring-up order?",
];

/// Question phrasings for score-board items. Each states the full
/// rubric — the supervision is only learnable (and only provable) if
/// the level mapping is part of the question, so the rubric text and
/// [`crate::sample::health_level`] must stay word-for-word synced (a
/// truth-table test in `sample` enforces it).
pub const SCORE_QUESTIONS: &[&str] = &[
    "Rate the overall operational health of the {noun} fleet on the reported scale. \
     Level rubric over all reports: critical means more than half of all reports are down; \
     poor means some reports are down, not a majority, and at least half of all reports \
     are failing or degraded; mixed means failures are present but healthy is the largest \
     share, or nothing is down and degraded outnumbers healthy; healthy means nothing is \
     down and healthy reports are at least as many as degraded.",
    "Score the fleet state of the {noun} set using the given levels. Count every report on \
     the board: critical if more than half of all reports are down; poor if downs are \
     present without being a majority while failing-or-degraded reports reach half of all \
     reports; mixed if failures exist yet healthy reports lead, or if nothing is down and \
     degraded leads; healthy if nothing is down and healthy reports equal or outnumber \
     degraded ones.",
];

/// The fixed 4-level score scale (index = level). Descriptions are the
/// per-level summaries of the same rubric the instructions state; the
/// prep renders them as `level <i>: <description>` option lines.
pub const SCORE_LEVELS: &[&str] = &[
    "critical: more than half of all reports are down",
    "poor: downs present, not a majority, with at least half of all reports failing or degraded",
    "mixed: failures present but healthy leads, or nothing down with degraded leading",
    "healthy: nothing down and healthy reports at least equal degraded",
];

/// Status words for a single-entity
/// [`Health::Down`](opencodifier_engine::facts::Health::Down) statement —
/// all
/// parse to the same fact, so the variety is free surface diversity.
pub const DOWN_WORDS: &[&str] = &["failing", "down", "offline"];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn domain_names_are_extractor_safe_and_pooled_large_enough() {
        for domain in DOMAINS {
            assert!(domain.names.len() >= 6, "{} pool too small", domain.code);
            for name in domain.names {
                assert!(
                    !name.is_empty()
                        && name.len() <= 64
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                    "name {name:?} is not extractor-safe"
                );
            }
        }
    }

    #[test]
    fn phrasings_carry_the_noun_placeholder_and_no_names() {
        for pool in
            [ROOT_CAUSE_QUESTIONS, HEALTHIEST_QUESTIONS, FIRST_RESTORED_QUESTIONS, SCORE_QUESTIONS]
        {
            for phrasing in pool {
                assert!(phrasing.contains("{noun}"), "phrasing lacks {{noun}}: {phrasing}");
                assert!(
                    !phrasing.contains("auth") && !phrasing.contains("north"),
                    "phrasing leaks an entity name: {phrasing}"
                );
            }
        }
    }

    #[test]
    fn score_scale_has_one_description_per_level() {
        assert_eq!(SCORE_LEVELS.len(), 4);
        for (i, desc) in SCORE_LEVELS.iter().enumerate() {
            assert!(!desc.is_empty(), "level {i} description empty");
        }
    }
}
