//! Solver-verified relational/contrastive item generation for the E1
//! decision-model corpus (TRAINING.md §9.7 B3 / decision record D37).
//!
//! The r2 post-mortem traced the model's relational weakness to the
//! data: the corpus carries almost no relational-analog rows, and its
//! labels are not contrastive (nothing teaches why a plausible-looking
//! wrong answer is wrong). This crate is the root-cause fix's
//! implementation home: it samples symbolic fact graphs, renders them
//! through the engine's own extraction grammar, and **verifies every
//! item against [`opencodifier_engine::RelationalSolver`] itself** —
//! the same zero-ML stack the serving rung decides with. A generated
//! item ships only when the solver proves its gold at probability 1.0
//! over the full ballot, which makes every non-gold slot's " no"
//! supervision provably exact and the negatives genuinely hard (each
//! carries a named, structural reason it is wrong — symptom, runner-up,
//! waiter, degraded-heavy).
//!
//! # Output contract
//!
//! Records are ordinary corpus rows in the `decision_sft_prep` input
//! shape (record/request/target), so generated items flow through the
//! unchanged prep, its macjev renderer, and its suite-exclusion gate.
//! A deterministic split (`--probe-every`) routes a held-out slice to
//! a separate file — the generator probe set the pilots read to tell
//! generator overfitting from rule learning.
//!
//! # Determinism
//!
//! The same `--seed` and `--n` produce byte-identical output: all
//! sampling goes through [`rng::Rng`] (`SplitMix64`), no maps are
//! iterated in hash order, and per-item seeds are a pure function of
//! the run seed and the item index.
//!
//! # Guardrails
//!
//! * Suite collision: exact normalized state+question match plus a
//!   12-word shared span — colliding items are rejected and
//!   resampled, and the manifest reports the count (always 0 emitted).
//! * Verification retries: an item that fails [`verify`] is a sampler
//!   or rendering bug; generation retries a bounded number of times
//!   with a bumped sub-seed, then fails the run loudly rather than
//!   emitting an unverifiable row.

pub mod render;
pub mod rng;
pub mod sample;
pub mod verify;
pub mod vocab;

pub use crate::sample::{Family, Sampled, sample};
pub use crate::verify::SuiteGuard;

use std::collections::BTreeMap;

use serde_json::json;

use crate::rng::Rng;
use crate::sample::{DistractorKind, FAMILIES};
use crate::verify::{lexical_probe, mention_probe, verify_item, verify_score_item};

/// Generation parameters.
#[derive(Debug, Clone)]
pub struct GenConfig {
    /// Number of items to emit (train + probe).
    pub n: usize,
    /// Run seed: the whole corpus is a pure function of this.
    pub seed: u64,
    /// Every `probe_every`-th item (0-based) goes to the probe file
    /// instead of the train file. 0 sends everything to train.
    pub probe_every: usize,
    /// The family rotation pool (codes in [`Family`] order). Empty
    /// means the four choice families ([`FAMILIES`]); score items
    /// join a run only by explicit selection.
    pub families: Vec<Family>,
    /// In [`Family::RootCauseChain`] slots, every `pair_every`-th item
    /// is emitted as a minimal pair (§9.7 B4): the base record plus its
    /// one-fact-corrupted twin, both solver-verified. 0 disables pairs.
    pub pair_every: usize,
}

/// Tallies the run reports into its manifest.
#[derive(Debug, Default)]
pub struct GenStats {
    /// Items per family.
    pub families: BTreeMap<String, usize>,
    /// Items per entity domain.
    pub domains: BTreeMap<String, usize>,
    /// Question phrasings per family.
    pub phrasings: BTreeMap<String, BTreeMap<String, usize>>,
    /// Distractor slots per known-wrong kind.
    pub distractors: BTreeMap<String, usize>,
    /// Lexical-probe solve rate per family: (lexical-picks-gold, total).
    pub lexical: BTreeMap<String, (usize, usize)>,
    /// Mention-salience rate per family: (gold-uniquely-most-mentioned,
    /// total) — the AFLITE shortcut check; low is the claim.
    pub mentions: BTreeMap<String, (usize, usize)>,
    /// Items rejected by verification before an acceptable one landed.
    pub rejected: usize,
    /// Items rejected by the suite-collision guard.
    pub collisions: usize,
    /// Items written to the train output.
    pub train: usize,
    /// Items written to the probe output.
    pub probe: usize,
    /// Gold-level census over score items (`"0"`..`"3"`).
    pub score_levels: BTreeMap<String, usize>,
    /// Minimal pairs emitted (one pair = two records sharing an id
    /// prefix).
    pub pairs: usize,
    /// Pair attempts where the twin failed verification — the base was
    /// emitted alone, and the count is the alarm.
    pub pairs_rejected: usize,
}

/// Why generation stopped.
#[derive(Debug)]
pub enum GenError {
    /// An item never produced a verifier-acceptable render within the
    /// retry budget — a sampler or rendering bug, surfaced with the
    /// last failure description.
    Unverifiable {
        /// The family that would not verify.
        family: Family,
        /// The item slot.
        index: usize,
        /// The last verification failure text.
        last_error: String,
    },
}

/// How many times one item slot may be resampled before the run fails.
const MAX_ATTEMPTS: usize = 9;

/// Runs generation, streaming each rendered record to `emit` as a
/// `serde_json` value (the caller owns serialization and file writing).
/// `suite` guards against near-duplicate renders of suite items.
///
/// # Errors
///
/// [`GenError::Unverifiable`] when an item slot exhausts its retries.
pub fn generate(
    config: &GenConfig,
    suite: Option<&SuiteGuard>,
    mut emit: impl FnMut(bool /* probe */, serde_json::Value, &Sampled),
) -> Result<GenStats, GenError> {
    // Default rotation is the four CHOICE families — score items join
    // a run only by explicit selection (`--families sb`), so the
    // relational slice's composition stays stable across runs (§9.7
    // B3). GENERATION_FAMILIES defines the code order, not the
    // default.
    let rotation: Vec<Family> =
        if config.families.is_empty() { FAMILIES.to_vec() } else { config.families.clone() };
    let mut stats = GenStats::default();
    for index in 0..config.n {
        let item_seed =
            config.seed.wrapping_add((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let family = rotation[index % rotation.len()];
        let mut accepted: Option<(Sampled, String)> = None;
        let mut last_error = String::from("no attempt was made");
        for attempt in 0..MAX_ATTEMPTS {
            let mut rng = Rng::new(item_seed.wrapping_add(attempt as u64));
            let sampled = sample(family, &mut rng);
            let text = render::state_text(&sampled.facts, &mut rng);
            let question = sample::question_text(&sampled);
            if let Some(guard) = suite
                && guard.collides(&text, &question)
            {
                stats.collisions += 1;
                continue;
            }
            let verified = if family == Family::ScoreBoard {
                verify_score_item(&sampled, &text)
            } else {
                verify_item(&sampled, &text)
            };
            match verified {
                Ok(()) => {
                    accepted = Some((sampled, text));
                    break;
                }
                Err(error) => {
                    stats.rejected += 1;
                    last_error = error;
                }
            }
        }
        let Some((sampled, text)) = accepted else {
            return Err(GenError::Unverifiable { family, index, last_error });
        };
        let record_id =
            format!("ig-{}-{}-{index:06}", render::family_code(family), sampled.domain.code);
        let is_probe = config.probe_every > 0 && index % config.probe_every == 0;
        emit_record(&mut emit, is_probe, &record_id, &sampled, &text, &mut stats);

        // Minimal-pair twin (§9.7 B4): root-cause slots only, on a
        // deterministic cadence. The twin re-rolls its own render rng
        // (fresh surface order over the SAME one-fact-different fact
        // set) and must pass the same suite guard and solver proof
        // before it may join the base record.
        if family == Family::RootCauseChain
            && config.pair_every > 0
            && index % config.pair_every == 0
            && let Some(twin) = sample::corrupt_to_pair(&sampled)
        {
            let mut accepted_twin: Option<String> = None;
            for attempt in 0..MAX_ATTEMPTS {
                let mut rng = Rng::new(item_seed.wrapping_add(0x51D5_5EED + attempt as u64));
                let twin_text = render::state_text(&twin.facts, &mut rng);
                let twin_question = sample::question_text(&twin);
                if let Some(guard) = suite
                    && guard.collides(&twin_text, &twin_question)
                {
                    stats.collisions += 1;
                    continue;
                }
                match verify_item(&twin, &twin_text) {
                    Ok(()) => {
                        accepted_twin = Some(twin_text);
                        break;
                    }
                    Err(error) => {
                        stats.pairs_rejected += 1;
                        last_error = error;
                    }
                }
            }
            if let Some(twin_text) = accepted_twin {
                let twin_id = format!("{record_id}-p{index:06}");
                emit_record(&mut emit, is_probe, &twin_id, &twin, &twin_text, &mut stats);
                stats.pairs += 1;
            }
        }
    }
    Ok(stats)
}

/// Emits one verified item: the corpus record through `emit`, plus the
/// per-family tallies. Score items census their gold level instead of
/// distractor kinds, and the choice probes are meaninglessness for
/// them, so only choice items feed the probe rates.
fn emit_record(
    emit: &mut impl FnMut(bool, serde_json::Value, &Sampled),
    probe: bool,
    record_id: &str,
    sampled: &Sampled,
    text: &str,
    stats: &mut GenStats,
) {
    let family = sampled.family;
    let record = render::corpus_record(sampled, text, record_id);
    emit(probe, record, sampled);

    let family_key = format!("{family:?}");
    *stats.families.entry(family_key.clone()).or_default() += 1;
    *stats.domains.entry(sampled.domain.code.to_owned()).or_default() += 1;
    *stats
        .phrasings
        .entry(family_key.clone())
        .or_default()
        .entry(sample::question_text(sampled))
        .or_default() += 1;
    if family == Family::ScoreBoard {
        *stats.score_levels.entry(sampled.gold.clone()).or_default() += 1;
    } else {
        for d in &sampled.distractors {
            let kind = match d.kind {
                DistractorKind::Symptom => "symptom",
                DistractorKind::StandaloneFailure => "standalone_failure",
                DistractorKind::HealthyPeer => "healthy_peer",
                DistractorKind::Unrelated => "unrelated",
                DistractorKind::RunnerUp => "runner_up",
                DistractorKind::DegradedHeavy => "degraded_heavy",
                DistractorKind::ZeroReport => "zero_report",
                DistractorKind::Waiter => "waiter",
                DistractorKind::LastInOrder => "last_in_order",
            };
            *stats.distractors.entry(kind.to_owned()).or_default() += 1;
        }
        let solved = lexical_probe(sampled, text);
        let salient = mention_probe(sampled, text);
        let lexical = stats.lexical.entry(family_key.clone()).or_default();
        lexical.1 += 1;
        if solved {
            lexical.0 += 1;
        }
        let mention = stats.mentions.entry(family_key).or_default();
        mention.1 += 1;
        if salient {
            mention.0 += 1;
        }
    }
    if probe {
        stats.probe += 1;
    } else {
        stats.train += 1;
    }
}

/// The run manifest: every number a downstream consumer needs to trust
/// the corpus (counts, censuses, probe rates, guard outcomes).
#[must_use]
pub fn manifest_value(config: &GenConfig, stats: &GenStats) -> serde_json::Value {
    let rate_block =
        |map: &BTreeMap<String, (usize, usize)>| -> BTreeMap<String, serde_json::Value> {
            map.iter()
                .map(|(family, (solved, total))| {
                    #[allow(clippy::cast_precision_loss)] // census counts are tiny
                    let rate = if *total > 0 { (*solved as f64) / (*total as f64) } else { 0.0 };
                    (
                        family.clone(),
                        json!({"solved": solved, "total": total,
                           "rate": (rate * 10_000.0).round() / 10_000.0}),
                    )
                })
                .collect()
        };
    let lexical = rate_block(&stats.lexical);
    let mentions = rate_block(&stats.mentions);
    let family_codes: Vec<&str> = if config.families.is_empty() {
        FAMILIES.iter().map(|family| family.code()).collect()
    } else {
        config.families.iter().map(|family| family.code()).collect()
    };
    json!({
        "manifest_version": "opencodifier.itemgen/1",
        "seed": config.seed,
        "n": config.n,
        "probe_every": config.probe_every,
        "families": family_codes,
        "pair_every": config.pair_every,
        "train": stats.train,
        "probe": stats.probe,
        "minimal_pairs": {"emitted": stats.pairs, "twins_rejected": stats.pairs_rejected},
        "rejected_by_verification": stats.rejected,
        "rejected_by_suite_collision": stats.collisions,
        "families_census": stats.families,
        "domains": stats.domains,
        "phrasings": stats.phrasings,
        "distractors": stats.distractors,
        "score_levels": stats.score_levels,
        "lexical_probe": lexical,
        "mention_salience_probe": mentions,
        "verification": {
            "round_trip": "extracted facts must equal constructed facts",
            "solver": "relational-v1|lexical, probability 1.0 on gold (choice families)",
            "score": "gold level re-derived from extracted facts through the rubric",
            "lexical": "recorded, not enforced (AFLITE-style probe)"
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::sample::FAMILIES;

    fn config(n: usize, seed: u64, probe_every: usize) -> GenConfig {
        GenConfig { n, seed, probe_every, families: Vec::new(), pair_every: 0 }
    }

    #[test]
    fn generation_produces_the_requested_item_count_with_a_clean_manifest() {
        let cfg = config(200, 42, 20);
        let mut lines = Vec::new();
        let stats = generate(&cfg, None, |probe, record, _| {
            lines.push((probe, record));
        })
        .unwrap();
        assert_eq!(lines.len(), 200);
        assert_eq!(stats.train + stats.probe, 200);
        assert_eq!(stats.probe, 10);
        assert_eq!(stats.collisions, 0, "no suite passed, so no collisions");
        for family in FAMILIES {
            assert_eq!(stats.families.get(&format!("{family:?}")), Some(&50));
        }
        assert_eq!(stats.families.get("ScoreBoard"), None, "score stays out of rotation");
        // Every record is a well-formed corpus row with one "yes" target.
        for (probe, record) in &lines {
            let _ = probe;
            let criteria = record["request"]["questions"]["root_cause"]["criteria"]
                .as_object()
                .or_else(|| record["request"]["questions"]["healthiest"]["criteria"].as_object())
                .or_else(|| {
                    record["request"]["questions"]["first_restored"]["criteria"].as_object()
                })
                .unwrap();
            assert!(criteria.len() >= 5);
            let label = record["target"].as_object().unwrap().values().next().unwrap()["label"]
                .as_str()
                .unwrap();
            assert!(criteria.contains_key(label));
        }
    }

    #[test]
    fn score_generation_spreads_levels_and_carries_the_score_shape() {
        let cfg = GenConfig { families: vec![Family::ScoreBoard], ..config(120, 77, 0) };
        let mut records = Vec::new();
        let stats = generate(&cfg, None, |_, record, _| records.push(record)).unwrap();
        assert_eq!(records.len(), 120);
        assert_eq!(stats.score_levels.values().sum::<usize>(), 120);
        assert!(stats.score_levels.len() >= 3, "level census collapsed: {:?}", stats.score_levels);
        for record in &records {
            assert_eq!(record["source"], "itemgen/score-board");
            let question = &record["request"]["questions"]["health_score"];
            assert_eq!(question["type"], "score");
            let criteria = question["criteria"].as_array().unwrap();
            assert_eq!(criteria.len(), 4);
            let label = record["target"]["health_score"]["label"].as_str().unwrap();
            assert!(["0", "1", "2", "3"].contains(&label), "gold is not a level index: {label}");
            assert_eq!(record["target"]["health_score"]["type"], "score");
        }
        assert!(stats.distractors.is_empty(), "score items have no entity ballot");
    }

    #[test]
    fn minimal_pairs_flip_the_gold_with_the_same_ballot() {
        // pair_every 1: every root-cause slot attempts a pair; the
        // healthy-pair precondition (~55% of draws) gates emission, so
        // a large n keeps the ≥3 floor deterministic.
        let cfg = GenConfig { pair_every: 1, ..config(80, 55, 0) };
        let mut records = Vec::new();
        let stats = generate(&cfg, None, |_, record, _| records.push(record)).unwrap();
        assert!(stats.pairs >= 3, "no pairs emitted: {:?}", stats.pairs);
        assert_eq!(stats.pairs_rejected, 0, "twins must verify: {:?}", stats.pairs_rejected);
        // Each pair: the twin's id is the base id plus the -pNNNNNN
        // suffix; question text matches, golds differ, ballots equal.
        let by_id: std::collections::BTreeMap<&str, &serde_json::Value> =
            records.iter().map(|r| (r["record_id"].as_str().unwrap(), r)).collect();
        let twin_ids: Vec<&str> = by_id.keys().copied().filter(|id| id.contains("-p0")).collect();
        assert_eq!(twin_ids.len(), stats.pairs, "pair count vs emitted twin ids");
        let mut checked = 0;
        for twin_id in twin_ids {
            let base_id = twin_id.split("-p").next().unwrap();
            let base = by_id
                .get(base_id)
                .unwrap_or_else(|| panic!("twin {twin_id} has no base {base_id}"));
            let twin = by_id[twin_id];
            let gold_of = |r: &serde_json::Value| {
                r["target"]["root_cause"]["label"].as_str().unwrap().to_owned()
            };
            assert_ne!(gold_of(base), gold_of(twin), "pair {base_id} does not flip the gold");
            let ballot_of = |r: &serde_json::Value| {
                let mut keys: Vec<String> = r["request"]["questions"]["root_cause"]["criteria"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect();
                keys.sort();
                keys.join(",")
            };
            assert_eq!(ballot_of(base), ballot_of(twin), "pair {base_id} changed the ballot");
            assert_eq!(
                base["request"]["questions"]["root_cause"]["instructions"],
                twin["request"]["questions"]["root_cause"]["instructions"],
                "pair {base_id} changed the question"
            );
            checked += 1;
        }
        assert!(checked >= 3, "only {checked} pairs checked");
    }

    #[test]
    fn generation_is_byte_deterministic_per_seed() {
        let run = |seed: u64| {
            let cfg = config(40, seed, 7);
            let mut lines = Vec::new();
            generate(&cfg, None, |_, record, _| {
                lines.push(record.to_string());
            })
            .unwrap();
            lines
        };
        assert_eq!(run(9), run(9));
        assert_ne!(run(9), run(10));
    }

    #[test]
    fn the_manifest_reports_probe_rates_and_censuses() {
        let cfg = config(80, 3, 0);
        let mut stats = generate(&cfg, None, |_, _, _| {}).unwrap();
        stats.lexical.insert("RootCauseChain".to_owned(), (7, 20));
        let manifest = manifest_value(&cfg, &stats);
        assert_eq!(manifest["manifest_version"], "opencodifier.itemgen/1");
        assert_eq!(manifest["lexical_probe"]["RootCauseChain"]["solved"], 7);
        assert_eq!(manifest["lexical_probe"]["RootCauseChain"]["rate"], 0.35);
        assert_eq!(manifest["families_census"].as_object().unwrap().len(), 4);
        assert!(manifest["distractors"].as_object().unwrap().len() >= 3);
        assert_eq!(manifest["families"].as_array().unwrap().len(), 4);
    }

    /// Writes `(context, question)` pairs as a suite file and loads the
    /// guard from it.
    fn guard_from(dir: &std::path::Path, items: &[(String, String)]) -> SuiteGuard {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("suite.json");
        let suite = serde_json::json!({
            "items": items
                .iter()
                .map(|(context, question)| serde_json::json!({ "context": context, "question": question }))
                .collect::<Vec<_>>(),
        });
        std::fs::write(&path, suite.to_string()).unwrap();
        SuiteGuard::load(&path).unwrap()
    }

    /// A guard built from a run's own corpus blocks that run's
    /// regeneration: identical seeds re-render identical first attempts,
    /// so every base slot and every twin slot exact-matches the corpus
    /// at least once before a re-roll escapes, and both rejection
    /// counters say so.
    #[test]
    fn a_corpus_guard_blocks_its_own_regeneration() {
        let seed = 0x0C0D_1F00_2026;
        let cfg = GenConfig {
            families: vec![Family::RootCauseChain],
            pair_every: 2,
            ..config(16, seed, 0)
        };

        // Run 1: collect every render (base and twin) the corpus holds.
        let mut corpus = Vec::new();
        generate(&cfg, None, |_, record, sampled| {
            let state = record["request"]["state"].as_str().unwrap().to_owned();
            let question = crate::sample::question_text(sampled).clone();
            corpus.push((state, question));
        })
        .unwrap();

        let dir = std::env::temp_dir().join(format!("itemgen-block-{}-regen", std::process::id()));
        let guard = guard_from(&dir, &corpus);

        // Run 2: the same seed against its own corpus — every first
        // attempt is an exact collision by construction.
        let stats = generate(&cfg, Some(&guard), |_, _, _| {}).unwrap();
        // Twin attempts collide into the same counter (the pair's own
        // `pairs_rejected` is the verification alarm, not the guard's).
        assert!(stats.collisions >= cfg.n, "collisions {}", stats.collisions);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A guard that holds every render a slot's retry budget can reach
    /// refuses the run outright — the loud failure half of the §9.7 B3
    /// guardrail, never a silent emit.
    ///
    /// The retry seeds are deterministic (`item_seed = seed + index *
    /// GOLDEN`, attempt `a` renders from `item_seed + a`), so the guard
    /// can cover the slot's whole reachable render space exactly: for
    /// each slot and attempt, a one-item run at that seed renders the
    /// very text the guarded run will try.
    #[test]
    fn a_saturated_guard_refuses_the_run_loudly() {
        const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
        let seed = 0x0C0D_1F00_2026_u64;
        let cfg = GenConfig { families: vec![Family::RootCauseChain], ..config(4, seed, 0) };

        let mut corpus = Vec::new();
        for index in 0..cfg.n {
            let item_seed = seed.wrapping_add((index as u64).wrapping_mul(GOLDEN));
            for attempt in 0..MAX_ATTEMPTS as u64 {
                let probe = GenConfig {
                    families: vec![Family::RootCauseChain],
                    ..config(1, item_seed.wrapping_add(attempt), 0)
                };
                generate(&probe, None, |_, record, sampled| {
                    let state = record["request"]["state"].as_str().unwrap().to_owned();
                    let question = crate::sample::question_text(sampled).clone();
                    corpus.push((state, question));
                })
                .unwrap();
            }
        }

        let dir =
            std::env::temp_dir().join(format!("itemgen-block-{}-saturate", std::process::id()));
        let guard = guard_from(&dir, &corpus);

        match generate(&cfg, Some(&guard), |_, _, _| {}) {
            Err(GenError::Unverifiable { family, index, last_error }) => {
                assert_eq!(family, Family::RootCauseChain);
                assert!(index < cfg.n, "slot {index} outside the run");
                assert!(!last_error.is_empty(), "the refusal names its cause");
            }
            Ok(stats) => panic!(
                "a saturated guard must refuse, not emit: {} collisions across {} items",
                stats.collisions, cfg.n
            ),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
