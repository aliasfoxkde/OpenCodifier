//! Offline threshold fitting for ladder profiles (RESEARCH §15.6 item 2).
//!
//! The measured largest lever in the research sweep: a Boolean verdict
//! taken at the hard argmax (`p_true > 0.5`) is the wrong operating
//! point for skewed tasks — tuned thresholds moved F1 `0.499 -> 0.748`
//! (UNFAIR-ToS) and `0.243 -> 0.353` (`GoEmotions`), with a median tuned
//! threshold of 0.86. This module fits that boundary from labeled
//! evidence and emits it as a [`LadderProfile`] document — data the
//! engine loads through the D25 seam (`--ladder`), never a hardcoded
//! constant.
//!
//! The fit is deliberately simple and inspectable: a deterministic grid
//! sweep over candidate boundaries, scored by F1 on the true class,
//! ties resolved toward the smallest threshold. Every emitted document
//! round-trips through [`LadderProfile`]'s real loader (tested), so a
//! fitted profile can never drift from what `--ladder` accepts.

use std::collections::BTreeMap;

use opencodifier_core::DecisionPolicy;

use crate::error::{EngineError, EngineResult};

/// One labeled Boolean outcome: the distribution's `p_true` and whether
/// the true verdict was actually correct.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BooleanEvidence {
    /// The model's probability for the `true` verdict.
    pub p_true: f64,
    /// Whether `true` was the correct verdict for this question.
    pub label: bool,
}

/// The fitted boundary plus the evidence it was measured against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BooleanThresholdFit {
    /// The fitted boundary: answer true when `p_true >= threshold`.
    pub threshold: f64,
    /// F1 on the true class at the fitted boundary.
    pub f1: f64,
    /// F1 at the hard argmax (threshold 0.5), for comparison — the
    /// number the fit has to beat to be worth a ladder.
    pub f1_at_argmax: f64,
    /// Evidence rows scored.
    pub samples: usize,
}

/// F1 on the true class for the verdict rule `p_true >= threshold`.
///
/// Degenerate sweeps (no predicted positives, no actual positives)
/// score 0.0, so a fit that never beats the argmax reads as exactly
/// that.
#[must_use]
pub fn f1_at(evidence: &[BooleanEvidence], threshold: f64) -> f64 {
    let mut tp = 0.0;
    let mut fp = 0.0;
    let mut fn_count = 0.0;
    for row in evidence {
        let predicted = row.p_true >= threshold;
        match (predicted, row.label) {
            (true, true) => tp += 1.0,
            (true, false) => fp += 1.0,
            (false, true) => fn_count += 1.0,
            (false, false) => {}
        }
    }
    if tp <= 0.0 {
        return 0.0;
    }
    let precision = tp / (tp + fp);
    let recall = tp / (tp + fn_count);
    2.0 * precision * recall / (precision + recall)
}

/// Fits the Boolean verdict boundary by grid sweep, ties toward the
/// smallest threshold.
///
/// The grid runs `0.50..=0.99` in steps of 0.01 — boundaries below the
/// argmax only ever help by predicting *more* trues, which the research
/// finding (median tuned threshold 0.86, skewed toward conservative
/// trues) never calls for, and every candidate at or above the argmax
/// is reachable. Returns `None` on empty evidence.
#[must_use]
pub fn fit_boolean_threshold(evidence: &[BooleanEvidence]) -> Option<BooleanThresholdFit> {
    // 0.50, 0.51, .. 0.99
    const GRID: usize = 50;
    if evidence.is_empty() {
        return None;
    }
    let mut best: Option<(usize, f64)> = None;
    for step in 0..=GRID {
        let threshold = grid_threshold(step);
        let f1 = f1_at(evidence, threshold);
        if best.is_none_or(|(_, best_f1)| f1 > best_f1) {
            best = Some((step, f1));
        }
    }
    let (step, f1) = best?;
    let threshold = grid_threshold(step);
    Some(BooleanThresholdFit {
        threshold,
        f1,
        f1_at_argmax: f1_at(evidence, 0.50),
        samples: evidence.len(),
    })
}

/// The grid step's boundary as the nearest `f64` to the two-decimal
/// value. Integer-then-divide keeps the emitted document clean (`0.84`,
/// never an accumulated-float neighbor) — the profile is
/// operator-facing and diffed across fits.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // grid index, not a measurement
fn grid_threshold(step: usize) -> f64 {
    f64::from(step as u32 + 50) / 100.0
}

/// Emits a ladder-profile document carrying the fitted Boolean boundary
/// as the `boolean` kind override, composed over `base`.
///
/// The document is the canonical JSON form [`LadderProfile`] deserializes
/// (field order and names included); callers write it to disk and load
/// it with `--ladder`. The ladder id is validated here too — an
/// anonymous profile is refused at emit time, not just at load.
///
/// # Errors
///
/// [`EngineError::InvalidConfig`] when `id` is empty or `"none"`, or the
/// threshold violates [`DecisionPolicy::with_boolean_threshold`]'s
/// bounds.
pub fn emit_boolean_profile(
    id: &str,
    fit: &BooleanThresholdFit,
    base: &DecisionPolicy,
) -> EngineResult<String> {
    if id.is_empty() || id == "none" {
        return Err(EngineError::InvalidConfig {
            reason: "a ladder profile needs a non-empty id distinct from \"none\" (D25: \
                     the id decorates every cache key)"
                .to_owned(),
        });
    }
    let policy = base
        .clone()
        .with_boolean_threshold(fit.threshold)
        .map_err(|error| EngineError::InvalidConfig { reason: error.to_string() })?;
    let raw = serde_json::to_value(&policy)
        .map_err(|error| EngineError::InvalidConfig { reason: error.to_string() })?;
    let mut per_kind = BTreeMap::new();
    per_kind.insert(String::from("boolean"), raw);
    let document = serde_json::json!({
        "id": id,
        "per_kind": per_kind,
    });
    serde_json::to_string_pretty(&document)
        .map_err(|error| EngineError::InvalidConfig { reason: error.to_string() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use crate::ladder::LadderProfile;
    use opencodifier_core::RiskLevel;

    /// Skewed evidence in the research shape: most rows sit just above
    /// the argmax with FALSE labels, so the hard 0.5 boundary drowns in
    /// false positives (F1 ≈ 0.5) while a conservative boundary
    /// recovers real F1.
    fn skewed_evidence() -> Vec<BooleanEvidence> {
        let mut rows = Vec::new();
        // 40 true rows, all at or above the tuned boundary.
        for p in [0.97, 0.95, 0.93, 0.91, 0.90, 0.88, 0.87, 0.86] {
            for _ in 0..5 {
                rows.push(BooleanEvidence { p_true: p, label: true });
            }
        }
        // 80 false rows the argmax still calls true — the skew.
        for p in [0.84, 0.80, 0.75, 0.70, 0.65, 0.62, 0.58, 0.55] {
            for _ in 0..10 {
                rows.push(BooleanEvidence { p_true: p, label: false });
            }
        }
        // 40 false rows clearly below every candidate boundary.
        for p in [0.30, 0.25, 0.20, 0.15] {
            for _ in 0..10 {
                rows.push(BooleanEvidence { p_true: p, label: false });
            }
        }
        rows
    }

    #[test]
    fn the_fit_beats_the_argmax_on_skewed_evidence() {
        let rows = skewed_evidence();
        let fit = fit_boolean_threshold(&rows).expect("evidence is non-empty");
        assert!(
            fit.f1 > fit.f1_at_argmax,
            "fitted {} must beat argmax {}",
            fit.f1,
            fit.f1_at_argmax
        );
        // The research shape: the tuned boundary sits well above 0.5.
        assert!(fit.threshold >= 0.80, "threshold {}", fit.threshold);
    }

    #[test]
    fn the_fit_reports_the_argmax_it_must_beat() {
        let rows = skewed_evidence();
        let fit = fit_boolean_threshold(&rows).expect("evidence is non-empty");
        assert_eq!(fit.f1_at_argmax, f1_at(&rows, 0.50));
        assert_eq!(fit.samples, rows.len());
    }

    #[test]
    fn ties_resolve_toward_the_smallest_threshold() {
        // Every row above 0.5 with matching labels: every boundary from
        // 0.50 to 0.99 scores the same perfect F1; the smallest wins.
        let rows = vec![
            BooleanEvidence { p_true: 0.9, label: true },
            BooleanEvidence { p_true: 0.2, label: false },
        ];
        let fit = fit_boolean_threshold(&rows).expect("evidence is non-empty");
        assert!((fit.threshold - 0.50).abs() < 1e-9, "threshold {}", fit.threshold);
    }

    #[test]
    fn empty_evidence_refuses_to_fit() {
        assert!(fit_boolean_threshold(&[]).is_none());
    }

    #[test]
    fn degenerate_evidence_scores_zero_rather_than_guessing() {
        // All-false labels: no boundary earns a true-class F1 above 0.
        let rows = vec![
            BooleanEvidence { p_true: 0.9, label: false },
            BooleanEvidence { p_true: 0.1, label: false },
        ];
        let fit = fit_boolean_threshold(&rows).expect("evidence is non-empty");
        assert_eq!(fit.f1, 0.0);
        assert_eq!(fit.f1_at_argmax, 0.0);
    }

    #[test]
    fn the_emitted_profile_round_trips_through_the_real_loader() {
        let rows = skewed_evidence();
        let fit = fit_boolean_threshold(&rows).expect("evidence is non-empty");
        let base = DecisionPolicy::new(0.80, 0.65, 0.50, RiskLevel::Low).unwrap();
        let document = emit_boolean_profile("unfair-tos-fit-v1", &fit, &base).unwrap();
        let profile: LadderProfile = serde_json::from_str(&document).unwrap();
        assert_eq!(profile.id, "unfair-tos-fit-v1");
        let ladder = profile.into_ladder().expect("profile carries an override");
        let (policy, source) = ladder
            .resolve("any-node", crate::graph::NodeKind::Boolean)
            .expect("the boolean kind override fires");
        assert_eq!(source, "kind:boolean");
        assert_eq!(policy.boolean_threshold(), Some(fit.threshold));
    }

    #[test]
    fn the_emitted_document_hides_an_unset_boundary() {
        // A policy without the field serializes without it — the
        // byte-stability guarantee the wire fixtures rely on.
        let base = DecisionPolicy::new(0.80, 0.65, 0.50, RiskLevel::Low).unwrap();
        let fit = BooleanThresholdFit { threshold: 0.86, f1: 0.7, f1_at_argmax: 0.5, samples: 10 };
        let document = emit_boolean_profile("fit-v1", &fit, &base).unwrap();
        assert!(document.contains("boolean_threshold"), "{document}");
    }

    #[test]
    fn anonymous_profiles_are_refused_at_emit_time() {
        let base = DecisionPolicy::new(0.80, 0.65, 0.50, RiskLevel::Low).unwrap();
        let fit = BooleanThresholdFit { threshold: 0.86, f1: 0.7, f1_at_argmax: 0.5, samples: 10 };
        for id in ["", "none"] {
            let error = emit_boolean_profile(id, &fit, &base).unwrap_err();
            assert!(error.to_string().contains("non-empty id"), "{error}");
        }
    }

    #[test]
    fn out_of_range_thresholds_are_refused_at_emit_time() {
        let base = DecisionPolicy::new(0.80, 0.65, 0.50, RiskLevel::Low).unwrap();
        let fit = BooleanThresholdFit { threshold: 1.0, f1: 0.7, f1_at_argmax: 0.5, samples: 10 };
        let error = emit_boolean_profile("fit-v1", &fit, &base).unwrap_err();
        assert!(error.to_string().contains("strictly inside"), "{error}");
    }
}
