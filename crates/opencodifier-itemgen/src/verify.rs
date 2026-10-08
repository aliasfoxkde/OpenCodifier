//! Item verification: every generated item is proved against the same
//! engine the serving rung decides with.
//!
//! The ladder of checks, per item:
//!
//! 1. **Round trip** — the rendered state text re-extracts to exactly
//!    the fact set the sampler constructed. This is the check that
//!    catches a rendering the grammar silently mis-reads (a dropped
//!    edge trains a wrong chain; a mis-parsed status trains a wrong
//!    board).
//! 2. **Solver proof** — [`opencodifier_engine::RelationalSolver`]
//!    (the zero-ML stack, wrapping the lexical classifier) decides the
//!    item's full ballot with probability 1.0 on the gold and nothing
//!    else. Because every non-gold slot is provably excluded, the
//!    prep-derived " no" supervision on those slots is *exact*, not
//!    approximate — the contrastive property the corpus exists for.
//! 3. **Probes (recorded, not enforced)** — the bare BM25 classifier's
//!    top pick, and whether the gold is the uniquely most-mentioned
//!    entity in the state. The mention-salience rate is the AFLITE
//!    claim these items make (a low rate = no surface shortcut); the
//!    BM25 rate is recorded for completeness but ties resolve to the
//!    last maximum, so on this corpus it reads as a tie-break
//!    artifact rather than a hardness signal.
//!
//! A suite-collision guard (exact normalized match plus a 12-word
//! shingle span over the state text) runs before verification;
//! colliding items are rejected and resampled, never emitted.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use opencodifier_core::{Candidate, ChoiceQuestion, DecisionQuestion, State};
use opencodifier_engine::facts::extract;
use opencodifier_engine::{Classifier, LexicalClassifier, RelationalSolver};

use crate::render::canonical_sentence;
use crate::sample::Sampled;

/// Verifies a sampled item against its rendered state text.
///
/// # Errors
///
/// A description of the first failed check — a round-trip mismatch, or
/// a solver outcome that is not a unique probability-1.0 proof of the
/// gold. Both are generation bugs, never acceptable output.
pub fn verify_item(sampled: &Sampled, state: &str) -> Result<(), String> {
    // 1. Round trip: extraction must recover the constructed fact set.
    let extracted = extract(state);
    let expected: BTreeSet<String> = sampled.facts.iter().map(canonical_sentence).collect();
    let got: BTreeSet<String> = extracted.iter().map(canonical_sentence).collect();
    if got != expected {
        let missing: Vec<&String> = expected.difference(&got).collect();
        let extra: Vec<&String> = got.difference(&expected).collect();
        return Err(format!(
            "round-trip mismatch: text={state:?} missing={missing:?} extra={extra:?}"
        ));
    }

    // 2. Solver proof over the full ballot.
    let mut names: Vec<&str> = vec![&sampled.gold];
    names.extend(sampled.distractors.iter().map(|d| d.name.as_str()));
    let candidates = names
        .iter()
        .map(|name| Candidate::new((*name).to_owned(), format!("{} {name}", sampled.domain.noun)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("candidate construction failed: {error}"))?;
    let question = ChoiceQuestion::new("q", crate::sample::question_text(sampled), candidates)
        .map_err(|error| format!("question construction failed: {error}"))?;
    let state = State::from_text(state);
    let distribution = RelationalSolver::lexical()
        .decide(&state, &DecisionQuestion::Choice(question))
        .map_err(|error| format!("solver failed: {error}"))?;
    let top = distribution.top();
    if (top.probability - 1.0).abs() > 1e-9 || top.key != sampled.gold {
        return Err(format!(
            "solver did not prove the gold: top={} p={}",
            top.key, top.probability
        ));
    }
    Ok(())
}

/// Verifies a score-board item: round trip, then the gold level
/// re-derived from the **extracted** facts through the same
/// [`crate::sample::count_statuses`] + [`crate::sample::health_level`]
/// the record side used. There is no solver call — a score item has no
/// candidate ballot — so the proof here is that the extraction grammar
/// reads the board into exactly the tally the label claims.
///
/// # Errors
///
/// A description of the first failed check, as in [`verify_item`].
pub fn verify_score_item(sampled: &Sampled, state: &str) -> Result<(), String> {
    let extracted = extract(state);
    let expected: BTreeSet<String> = sampled.facts.iter().map(canonical_sentence).collect();
    let got: BTreeSet<String> = extracted.iter().map(canonical_sentence).collect();
    if got != expected {
        return Err(format!("score round-trip mismatch: text={state:?}"));
    }
    let (healthy, degraded, down) = crate::sample::count_statuses(&extracted);
    let level = crate::sample::health_level(healthy, degraded, down);
    if level.to_string() != sampled.gold {
        return Err(format!(
            "score level diverged after extraction: label={} extracted=({healthy},{degraded},{down}) rubric={level}",
            sampled.gold
        ));
    }
    Ok(())
}

/// Records which way the bare lexical classifier leaned on an item —
/// the AFLITE-style probe. `true` means the lexical baseline picks the
/// gold, i.e. the item is lexically separable.
#[must_use]
pub fn lexical_probe(sampled: &Sampled, state: &str) -> bool {
    let mut names: Vec<&str> = vec![&sampled.gold];
    names.extend(sampled.distractors.iter().map(|d| d.name.as_str()));
    let Ok(candidates) = names
        .iter()
        .map(|name| Candidate::new((*name).to_owned(), format!("{} {name}", sampled.domain.noun)))
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    let Ok(question) = ChoiceQuestion::new("q", crate::sample::question_text(sampled), candidates)
    else {
        return false;
    };
    let Ok(distribution) = LexicalClassifier::new()
        .decide(&State::from_text(state), &DecisionQuestion::Choice(question))
    else {
        return false;
    };
    distribution.top().key == sampled.gold
}

/// Whether the gold is the uniquely most-mentioned name in the state —
/// the surface-salience shortcut. `true` means a model could pick the
/// gold by counting mentions alone, so a LOW rate here is the AFLITE
/// claim these items make. Recorded per family in the manifest.
///
/// This probe, not the BM25 one, is the meaningful separability
/// number for this corpus: the BM25 probe ties whenever every
/// mentioned entity appears exactly once (our construction), and
/// [`Distribution::top`](opencodifier_core::answer::Distribution::top)
/// resolves exact ties to the *last* maximum,
/// so its rate is a tie-break artifact, not a hardness signal.
#[must_use]
pub fn mention_probe(sampled: &Sampled, state: &str) -> bool {
    let normalized = normalize(state);
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    let count = |name: &str| tokens.iter().filter(|token| **token == name).count();
    let gold_count = count(&sampled.gold);
    gold_count > 0 && sampled.distractors.iter().all(|d| count(&d.name) < gold_count)
}

/// The suite-collision guard: exact normalized matches plus shared
/// 12-word shingle spans against the decision suite.
#[derive(Debug)]
pub struct SuiteGuard {
    exact: HashSet<String>,
    shingles: HashSet<u64>,
}

/// FNV-1a 64-bit over a byte string — stable, dependency-free.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

/// Lowercases, strips everything that is not a letter or digit, and
/// collapses whitespace — the normalization both sides of the guard
/// are compared in.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim_end().to_owned()
}

impl SuiteGuard {
    /// Loads the guard from a suite JSON (`{"items": [...]}` with
    /// `context` and `question` per item).
    ///
    /// # Errors
    ///
    /// Unreadable file, malformed JSON, or an item missing the fields
    /// the guard normalizes.
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read suite {}: {error}", path.display()))?;
        let suite: serde_json::Value =
            serde_json::from_str(&raw).map_err(|error| format!("malformed suite: {error}"))?;
        let items = suite
            .get("items")
            .and_then(serde_json::Value::as_array)
            .ok_or("suite has no items array")?;
        let mut guard = Self { exact: HashSet::new(), shingles: HashSet::new() };
        for item in items {
            let context = item
                .get("context")
                .and_then(serde_json::Value::as_str)
                .ok_or("suite item without context")?;
            let question = item
                .get("question")
                .and_then(serde_json::Value::as_str)
                .ok_or("suite item without question")?;
            let normalized = format!("{}\n{}", normalize(context), normalize(question));
            guard.exact.insert(normalized);
            for shingle in shingles(&normalize(context), 12) {
                guard.shingles.insert(shingle);
            }
        }
        Ok(guard)
    }

    /// Whether a rendered item collides with any suite item: an exact
    /// normalized state+question match, or a shared 12-word span in
    /// the state text.
    #[must_use]
    pub fn collides(&self, state: &str, question: &str) -> bool {
        let normalized_question = normalize(question);
        if self.exact.contains(&format!("{}\n{normalized_question}", normalize(state))) {
            return true;
        }
        shingles(&normalize(state), 12).iter().any(|shingle| self.shingles.contains(shingle))
    }
}

/// The 64-bit hashes of every `size`-word window in `text`.
fn shingles(text: &str, size: usize) -> Vec<u64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let hashed: Vec<u64> = words.iter().map(|w| fnv1a(w.as_bytes())).collect();
    hashed
        .windows(size)
        .map(|window| {
            // Order-sensitive combine: a span matches only when the
            // words match in sequence.
            window.iter().fold(0xcbf2_9ce4_8422_2325u64, |acc, word| {
                (acc ^ word).wrapping_mul(0x0000_0100_0000_01B3)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::render::state_text;
    use crate::rng::Rng;
    use crate::sample::{Family, sample};

    #[test]
    fn every_sampled_item_verifies_against_the_solver() {
        for family in crate::sample::FAMILIES {
            let mut rng = Rng::new(21);
            for _ in 0..100 {
                let sampled = sample(*family, &mut rng);
                let text = state_text(&sampled.facts, &mut rng);
                verify_item(&sampled, &text).unwrap_or_else(|error| panic!("{family:?}: {error}"));
            }
        }
    }

    #[test]
    fn a_corrupted_fact_set_fails_the_round_trip() {
        let mut rng = Rng::new(22);
        let mut sampled = sample(Family::RootCauseChain, &mut rng);
        let text = state_text(&sampled.facts, &mut rng);
        // Corrupt the *expected* side after rendering: a fact the
        // sampler claims but the text never states. Round-trip
        // verification must catch the divergence.
        sampled.facts.push(opencodifier_engine::facts::RelationalFact::Health {
            entity: "phantom".to_owned(),
            status: opencodifier_engine::facts::Health::Healthy,
        });
        assert!(verify_item(&sampled, &text).is_err());
    }

    #[test]
    fn a_detached_side_starter_breaks_first_restored_verification() {
        // The interference rule from the module docs, demonstrated: a
        // detached side chain creates a second wait-for-nothing
        // starter, the solver abstains, and verification must fail.
        // Seed 3 redraws before accepting (seed 23 accepted first draw),
        // so the scan's iterate edge below genuinely runs.
        let mut rng = Rng::new(3);
        let sampled = loop {
            let candidate = sample(Family::FirstRestoredChain, &mut rng);
            let unused = candidate
                .domain
                .names
                .iter()
                .filter(|n| !candidate.facts.iter().any(|f| fact_mentions(f, n)))
                .count();
            if unused >= 2 {
                break candidate;
            }
        };
        let spare: Vec<&str> = sampled
            .domain
            .names
            .iter()
            .copied()
            .filter(|n| !sampled.facts.iter().any(|f| fact_mentions(f, n)))
            .take(2)
            .collect();
        let mut facts = sampled.facts.clone();
        facts.push(opencodifier_engine::facts::RelationalFact::RestoresAfter {
            later: spare[1].to_owned(),
            earlier: spare[0].to_owned(),
        });
        let broken = Sampled { facts, ..sampled };
        let text = state_text(&broken.facts, &mut rng);
        assert!(verify_item(&broken, &text).is_err(), "a detached starter must not verify");
    }

    fn fact_mentions(fact: &opencodifier_engine::facts::RelationalFact, name: &str) -> bool {
        use opencodifier_engine::facts::RelationalFact;
        match fact {
            RelationalFact::DependsOn { dependent, dependency } => {
                dependent == name || dependency == name
            }
            RelationalFact::Health { entity, .. } | RelationalFact::HealthList { entity, .. } => {
                entity == name
            }
            RelationalFact::RestoresAfter { later, earlier } => later == name || earlier == name,
        }
    }

    #[test]
    fn the_gold_is_never_the_uniquely_most_mentioned_entity() {
        // The AFLITE claim, by construction: no family leaves the gold
        // as the surface-salience shortcut.
        for family in crate::sample::FAMILIES {
            let mut rng = Rng::new(24);
            for _ in 0..200 {
                let sampled = sample(*family, &mut rng);
                let text = state_text(&sampled.facts, &mut rng);
                assert!(
                    !mention_probe(&sampled, &text),
                    "{family:?}: gold {} is uniquely most-mentioned in {text:?}",
                    sampled.gold
                );
            }
        }
    }

    #[test]
    fn every_score_item_rederives_its_level_from_extracted_facts() {
        let mut rng = Rng::new(25);
        let mut levels_seen = [0usize; 4];
        for _ in 0..200 {
            let sampled = sample(Family::ScoreBoard, &mut rng);
            let text = state_text(&sampled.facts, &mut rng);
            verify_score_item(&sampled, &text)
                .unwrap_or_else(|error| panic!("score item: {error}"));
            levels_seen[sampled.gold.parse::<usize>().unwrap()] += 1;
        }
        for (level, count) in levels_seen.iter().enumerate() {
            assert!(*count >= 20, "level {level} drew only {count} of 200");
        }
    }

    #[test]
    fn a_flipped_score_label_fails_verification() {
        let mut rng = Rng::new(26);
        let sampled = sample(Family::ScoreBoard, &mut rng);
        let text = state_text(&sampled.facts, &mut rng);
        verify_score_item(&sampled, &text).unwrap();
        let mut wrong = sampled.clone();
        wrong.gold = ((sampled.gold.parse::<usize>().unwrap() + 1) % 4).to_string();
        let error = verify_score_item(&wrong, &text).unwrap_err();
        assert!(error.contains("score level diverged"), "{error}");
    }

    #[test]
    fn the_suite_guard_detects_exact_and_span_collisions() {
        let suite = serde_json::json!({
            "items": [
                {"context": "billing is failing. auth is failing. billing depends on index. index depends on feed.",
                 "question": "Fixing which single component restores every part of the chain?"}
            ]
        });
        let dir = std::env::temp_dir().join(format!("oc-itemgen-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("suite.json");
        std::fs::write(&path, serde_json::to_string(&suite).unwrap()).unwrap();
        let guard = SuiteGuard::load(&path).unwrap();

        // Exact state + question: collision.
        assert!(guard.collides(
            "billing is failing. auth is failing. billing depends on index. index depends on feed.",
            "Fixing which single component restores every part of the chain?"
        ));
        // Shared 12-word span in the state, different question: still a
        // collision — the near-dup rule.
        assert!(guard.collides(
            "billing is failing. auth is failing. billing depends on index. index depends on feed. worker is healthy.",
            "Which component is failing at the root, rather than because of another failure?"
        ));
        // Unrelated state: no collision.
        assert!(!guard.collides(
            "north: healthy, healthy, down. south: healthy, down.",
            "Which region is healthiest overall?"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `fact_mentions` reads every fact kind by its entity slots — the
    /// predicate the detached-side scan's dedupe filter leans on.
    #[test]
    fn fact_mentions_reads_each_fact_kind_by_name() {
        use opencodifier_engine::facts::{Health, RelationalFact};
        let depends = RelationalFact::DependsOn {
            dependent: "cache-a".to_owned(),
            dependency: "db-a".to_owned(),
        };
        let health = RelationalFact::Health { entity: "queue-a".to_owned(), status: Health::Down };
        let list = RelationalFact::HealthList {
            entity: "store-a".to_owned(),
            statuses: vec![Health::Healthy],
        };
        // A dependency edge matches on either endpoint and nothing else.
        assert!(fact_mentions(&depends, "cache-a"));
        assert!(fact_mentions(&depends, "db-a"));
        assert!(!fact_mentions(&depends, "queue-a"));
        // A single health report matches only its entity.
        assert!(fact_mentions(&health, "queue-a"));
        assert!(!fact_mentions(&health, "db-a"));
        // A report list matches only its entity.
        assert!(fact_mentions(&list, "store-a"));
        assert!(!fact_mentions(&list, "cache-a"));
    }

    /// The detached-side scan usually accepts its first draw, but it
    /// must genuinely iterate: some seeds accept only after the
    /// `unused < 2` branch has sent a candidate back.
    #[test]
    fn the_detached_side_scan_iterates_before_accepting() {
        let mut iterating_seeds = 0;
        for seed in 1..=64u64 {
            let mut rng = Rng::new(seed);
            let mut draws = 0;
            loop {
                draws += 1;
                assert!(draws <= 200, "seed {seed}: no detached-side draw in 200 tries");
                let candidate = sample(Family::FirstRestoredChain, &mut rng);
                let unused = candidate
                    .domain
                    .names
                    .iter()
                    .filter(|n| !candidate.facts.iter().any(|f| fact_mentions(f, n)))
                    .count();
                if unused >= 2 {
                    if draws > 1 {
                        iterating_seeds += 1;
                    }
                    break;
                }
            }
        }
        assert!(iterating_seeds > 0);
    }
}
