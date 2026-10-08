//! Arm coverage for the sampler's public decision points (tranche 20c).
//!
//! Every arm here was measured uncovered by the in-image census and is
//! reachable through the public API alone: hand-built [`Sampled`] values
//! drive `corrupt_to_pair`'s refusal and twin-census arms, the family
//! code pool round-trips through `from_code`, and the verifier's
//! mismatch and unbuildable-ballot refusals are provoked directly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_engine::facts::{Health, RelationalFact};
use opencodifier_itemgen::rng::Rng;
use opencodifier_itemgen::sample::{
    Distractor, DistractorKind, FAMILIES, Family, GENERATION_FAMILIES, Sampled, corrupt_to_pair,
    down_word,
};
use opencodifier_itemgen::verify::{lexical_probe, verify_score_item};
use opencodifier_itemgen::vocab::{DOMAINS, DOWN_WORDS};

const DOMAIN: &opencodifier_itemgen::vocab::Domain = &DOMAINS[0];

/// A chain sample: `root` is down, `child` depends on it, `peer` is
/// healthy. `child`'s own status is a parameter — the refusals hinge on
/// it.
fn chain_sample(child_status: Health, distractors: Vec<Distractor>) -> Sampled {
    let names: Vec<&str> = DOMAIN.names[..4].to_vec();
    let [root, child, peer, _] = names[..] else { panic!("domain has four names") };
    Sampled {
        facts: vec![
            RelationalFact::Health { entity: root.to_owned(), status: Health::Down },
            RelationalFact::DependsOn { dependent: child.to_owned(), dependency: root.to_owned() },
            RelationalFact::Health { entity: child.to_owned(), status: child_status },
            RelationalFact::Health { entity: peer.to_owned(), status: Health::Healthy },
        ],
        gold: root.to_owned(),
        distractors,
        family: Family::RootCauseChain,
        domain: DOMAIN,
        phrasing: 0,
    }
}

#[test]
fn family_codes_round_trip_through_from_code() {
    for family in FAMILIES {
        assert_eq!(
            Family::from_code(family.code()),
            Some(*family),
            "{family:?}: code() parses back"
        );
    }
    assert_eq!(Family::from_code("zz"), None, "unknown codes refuse");
    assert_eq!(
        Family::from_code("sb"),
        Some(Family::ScoreBoard),
        "the score family joins by explicit selection"
    );
}

#[test]
fn down_word_draws_from_the_pool() {
    let mut rng = Rng::new(7);
    for _ in 0..64 {
        assert!(DOWN_WORDS.contains(&down_word(&mut rng)), "draws stay in the pool");
    }
}

#[test]
fn corrupt_to_pair_refuses_non_chain_families() {
    let names: Vec<&str> = DOMAIN.names[..3].to_vec();
    let sampled = Sampled {
        facts: vec![],
        gold: names[0].to_owned(),
        distractors: vec![],
        family: Family::HealthiestBoard,
        domain: DOMAIN,
        phrasing: 0,
    };
    assert!(corrupt_to_pair(&sampled).is_none(), "minimal pairs exist for root-cause slots only");
}

#[test]
fn corrupt_to_pair_refuses_a_chain_without_a_dependent_edge() {
    // A healthy peer (the twin candidate) but no `DependsOn` edge into
    // the gold: there is no child to promote.
    let names: Vec<&str> = DOMAIN.names[..3].to_vec();
    let sampled = Sampled {
        facts: vec![
            RelationalFact::Health { entity: names[0].to_owned(), status: Health::Down },
            RelationalFact::Health { entity: names[1].to_owned(), status: Health::Healthy },
        ],
        gold: names[0].to_owned(),
        distractors: vec![Distractor {
            name: names[1].to_owned(),
            kind: DistractorKind::HealthyPeer,
        }],
        family: Family::RootCauseChain,
        domain: DOMAIN,
        phrasing: 0,
    };
    assert!(corrupt_to_pair(&sampled).is_none(), "no chain edge, no twin");
}

#[test]
fn corrupt_to_pair_refuses_when_the_child_is_not_down() {
    // The child must already be failing; synthesizing a failure would
    // make the twin a two-fact edit.
    let sampled = chain_sample(Health::Degraded, vec![]);
    assert!(corrupt_to_pair(&sampled).is_none(), "a degraded child is not a symptom");
}

#[test]
fn corrupt_to_pair_relabels_the_twin_ballot() {
    // The old gold becomes a healthy peer; a symptom distractor with no
    // stated dependency on the new gold becomes a standalone failure;
    // the new gold leaves the ballot entirely.
    let names: Vec<&str> = DOMAIN.names[..5].to_vec();
    let [root, child, peer, stray, extra] = names[..] else { panic!("domain has five names") };
    let sampled = Sampled {
        facts: vec![
            RelationalFact::Health { entity: root.to_owned(), status: Health::Down },
            RelationalFact::DependsOn { dependent: child.to_owned(), dependency: root.to_owned() },
            RelationalFact::Health { entity: child.to_owned(), status: Health::Down },
            RelationalFact::Health { entity: peer.to_owned(), status: Health::Healthy },
        ],
        gold: root.to_owned(),
        distractors: vec![
            Distractor { name: child.to_owned(), kind: DistractorKind::Symptom },
            Distractor { name: root.to_owned(), kind: DistractorKind::Symptom },
            Distractor { name: stray.to_owned(), kind: DistractorKind::Symptom },
            Distractor { name: peer.to_owned(), kind: DistractorKind::HealthyPeer },
            Distractor { name: extra.to_owned(), kind: DistractorKind::Unrelated },
        ],
        family: Family::RootCauseChain,
        domain: DOMAIN,
        phrasing: 0,
    };
    let twin = corrupt_to_pair(&sampled).expect("the chain carries its own symptom");
    assert_eq!(twin.gold, child, "the direct dependent is promoted");

    let kinds: Vec<(&str, DistractorKind)> =
        twin.distractors.iter().map(|d| (d.name.as_str(), d.kind)).collect();
    assert!(!kinds.iter().any(|(name, _)| *name == child), "the new gold must leave the ballot");
    assert_eq!(
        kinds.iter().find(|(name, _)| *name == root),
        Some(&(root, DistractorKind::HealthyPeer)),
        "the repaired old gold is a healthy peer"
    );
    assert_eq!(
        kinds.iter().find(|(name, _)| *name == stray),
        Some(&(stray, DistractorKind::StandaloneFailure)),
        "a symptom with no dependency on the new gold stands alone"
    );
    assert_eq!(
        kinds.iter().find(|(name, _)| *name == peer),
        Some(&(peer, DistractorKind::HealthyPeer)),
        "unrelated distractors keep their reason"
    );
}

#[test]
fn verify_score_item_reports_a_round_trip_mismatch() {
    let names: Vec<&str> = DOMAIN.names[..2].to_vec();
    let sampled = Sampled {
        facts: vec![RelationalFact::Health {
            entity: names[0].to_owned(),
            status: Health::Healthy,
        }],
        gold: "3".to_owned(),
        distractors: vec![],
        family: Family::ScoreBoard,
        domain: DOMAIN,
        phrasing: 0,
    };
    let error =
        verify_score_item(&sampled, "an unrelated paragraph with no facts at all").unwrap_err();
    assert!(error.contains("score round-trip mismatch"), "{error}");
}

#[test]
fn lexical_probe_refuses_unbuildable_ballots() {
    // A blank gold cannot become a candidate.
    let blank = Sampled {
        facts: vec![],
        gold: String::new(),
        distractors: vec![],
        family: Family::RootCauseChain,
        domain: DOMAIN,
        phrasing: 0,
    };
    assert!(!lexical_probe(&blank, ""), "a blank gold refuses the probe");

    // Duplicate ballot names cannot become a question.
    let names: Vec<&str> = DOMAIN.names[..1].to_vec();
    let duplicated = Sampled {
        facts: vec![],
        gold: names[0].to_owned(),
        distractors: vec![
            Distractor { name: names[0].to_owned(), kind: DistractorKind::Unrelated },
            Distractor { name: names[0].to_owned(), kind: DistractorKind::Symptom },
        ],
        family: Family::RootCauseChain,
        domain: DOMAIN,
        phrasing: 0,
    };
    assert!(!lexical_probe(&duplicated, ""), "duplicate ballot ids refuse the probe");
}

#[test]
fn generation_families_superset_the_default_rotation() {
    // `FAMILIES` is the default rotation (choice families only);
    // `GENERATION_FAMILIES` adds ScoreBoard, which joins by explicit
    // selection (`--families sb`).
    for family in FAMILIES {
        assert!(
            GENERATION_FAMILIES.contains(family),
            "{family:?}: every default-rotation family is generatable"
        );
    }
    assert!(GENERATION_FAMILIES.contains(&Family::ScoreBoard), "sb joins by selection");
    assert!(!FAMILIES.contains(&Family::ScoreBoard), "sb is not in the default rotation");
}
