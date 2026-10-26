//! Rotation readout: position-bias-corrected decision distributions (§31).
//!
//! A readout whose logits depend on *where* an option sits in the list
//! flips answers when the list is reordered — the board's own position
//! bias (footnote 18's by-pos spread 0.58–1.00 is a clean signature).
//! The `AnyJev` `L0` mechanism (RESEARCH.md §31) corrects it label-free:
//! decide the same question over the K cyclic permutations of the
//! candidate list, combine the K distributions in log space, and divide
//! out a label prior estimated from the same K reads (no labels).
//!
//! This module never touches the wire format or the executor: it is a
//! [`Classifier`] decorator, so the ladder, cache, calibration, and gate
//! machinery behave exactly as before — the decorator's `model_id()`
//! re-keys cached decisions mechanically, like any classifier swap.
//!
//! Determinism: the permutation set is fully determined by the
//! candidate count; the combination is plain arithmetic over the inner
//! classifier's deterministic distributions. Same inputs → same output,
//! the [`Classifier`] contract.

use std::sync::Arc;

use opencodifier_core::{Candidate, DecisionQuestion, Distribution, DistributionEntry};

use crate::classifier::Classifier;
use crate::error::{EngineError, EngineResult};

/// How the per-position reads are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combine {
    /// Log-space mean over the K reads, after prior division. The §31
    /// default: one position's outlier bias cannot dominate.
    LogMean,
    /// Plain arithmetic mean over the K reads.
    ArithmeticMean,
}

/// A [`Classifier`] that decides over the cyclic rotations of a choice
/// question's candidate list and combines the reads. Boolean and score
/// questions have no option list to rotate; they pass through with a
/// single inner read (the decorator is a no-op for them).
///
/// The estimated label prior is the geometric mean of each candidate's
/// probability across rotations; dividing each read by it before
/// combining removes the systematic "this position scores high" bias
/// without any labelled data.
pub struct RotationReadout {
    inner: Arc<dyn Classifier>,
    combine: Combine,
    id: String,
    /// Early-exit budget (§31 item 2). `None` — the default — always
    /// reads all K rotations. `Some(k_min)` reads rotations in order,
    /// stopping once the running winner's log-odds margin over the
    /// runner-up certifies, at `target_agreement` via a Clopper–Pearson
    /// upper bound, that the partial readout's winner agrees with the
    /// full-K winner at the target rate. Fewer inner reads on easy
    /// questions, full-K certification on hard ones.
    early_exit: Option<EarlyExit>,
}

/// Early-exit parameters. `k_min` is the floor (never certify from one
/// read); `target_agreement` is the certified agreement rate with the
/// full-K readout (e.g. `0.99` = certify at the 1% disagreement level,
/// the §31 default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EarlyExit {
    /// Floor on the number of reads before the certificate is even
    /// checked — never certify from fewer.
    pub k_min: usize,
    /// The agreement rate with the full-K readout that the
    /// Clopper–Pearson bound must certify (e.g. `0.99`).
    pub target_agreement: f64,
}

impl RotationReadout {
    /// Wraps `inner`. `id` becomes the `model_id` suffix — pick a new
    /// one whenever the combination policy changes, so cached decisions
    /// re-key.
    pub fn new(inner: Arc<dyn Classifier>, id: impl Into<String>) -> Self {
        Self { inner, combine: Combine::LogMean, id: id.into(), early_exit: None }
    }

    /// Sets the combination rule, consuming and returning `self`.
    #[must_use]
    pub fn with_combine(mut self, combine: Combine) -> Self {
        self.combine = combine;
        self
    }

    /// Enables the Clopper–Pearson-certified early exit, consuming and
    /// returning `self`.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidConfig`] when `k_min` is 0 or
    /// `target_agreement` is outside `(0.5, 1.0)` — a floor of 0 would
    /// certify from a single read, and a target at or below chance is
    /// not a certification.
    pub fn with_early_exit(mut self, k_min: usize, target_agreement: f64) -> EngineResult<Self> {
        if k_min == 0 {
            return Err(EngineError::InvalidConfig {
                reason: "rotation early exit: k_min must be >= 1".into(),
            });
        }
        if !(0.5 < target_agreement && target_agreement < 1.0) {
            return Err(EngineError::InvalidConfig {
                reason: format!(
                    "rotation early exit: target_agreement {target_agreement} outside (0.5, 1.0)"
                ),
            });
        }
        self.early_exit = Some(EarlyExit { k_min, target_agreement });
        Ok(self)
    }

    /// The cyclic rotations of `candidates` — K rotations for K
    /// candidates, rotation 0 the identity.
    fn rotations(candidates: &[Candidate]) -> Vec<Vec<Candidate>> {
        let k = candidates.len();
        (0..k)
            .map(|shift| candidates.iter().cycle().skip(shift).take(k).cloned().collect::<Vec<_>>())
            .collect()
    }
}

impl std::fmt::Debug for RotationReadout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RotationReadout")
            .field("inner", &self.inner)
            .field("combine", &self.combine)
            .field("id", &self.id)
            .field("early_exit", &self.early_exit)
            .finish()
    }
}

impl Classifier for RotationReadout {
    fn decide(
        &self,
        state: &opencodifier_core::State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        let DecisionQuestion::Choice(choice) = question else {
            // No option list to rotate: a single pass-through read.
            return self.inner.decide(state, question);
        };
        let candidates = choice.candidates();
        if candidates.len() < 2 {
            return self.inner.decide(state, question);
        }

        let all_rotations = Self::rotations(candidates);

        // Read rotations in order. With the early exit configured, stop
        // once the partial readout's winner is CP-certified (at
        // `target_agreement`) to agree with the full-K winner; without
        // it, read all K. One pass, each read taken exactly once.
        let mut reads: Vec<Distribution> = Vec::with_capacity(all_rotations.len());
        for rotated in &all_rotations {
            let rotated_choice = opencodifier_core::ChoiceQuestion::new(
                choice.id().as_str(),
                choice.text(),
                rotated.clone(),
            )
            .map_err(|e| EngineError::InvalidConfig {
                reason: format!("rotation re-render refused: {e}"),
            })?;
            reads.push(self.inner.decide(state, &DecisionQuestion::Choice(rotated_choice))?);
            if let Some(ee) = &self.early_exit {
                let certified = reads.len() >= ee.k_min
                    && Self::certified(&reads, all_rotations.len(), ee.target_agreement);
                if certified {
                    break;
                }
            }
        }

        // Label prior per candidate: geometric mean of its probability
        // across the K reads. Pure arithmetic on the reads themselves —
        // no labels, no state mutation.
        let k = reads.len();
        #[allow(clippy::cast_precision_loss)]
        let kf = k as f64;
        let mut prior = std::collections::BTreeMap::<String, f64>::new();
        for read in &reads {
            for entry in read.entries() {
                *prior.entry(entry.key.clone()).or_insert(0.0) += entry.probability.ln();
            }
        }
        for v in prior.values_mut() {
            *v = (*v / kf).exp();
        }

        // Divide out the prior, then combine the K corrected reads.
        let mut combined = std::collections::BTreeMap::<String, f64>::new();
        for read in &reads {
            for entry in read.entries() {
                let base = prior.get(&entry.key).copied().unwrap_or(1.0);
                let corrected =
                    if base > f64::EPSILON { entry.probability / base } else { entry.probability };
                *combined.entry(entry.key.clone()).or_insert(0.0) += match self.combine {
                    Combine::ArithmeticMean => corrected,
                    Combine::LogMean => corrected.ln(),
                };
            }
        }
        let scale = match self.combine {
            Combine::ArithmeticMean => kf,
            Combine::LogMean => {
                for v in combined.values_mut() {
                    *v = (*v / kf).exp();
                }
                // log-mean outputs are not yet normalized; normalize below.
                1.0
            }
        };

        let mut entries: Vec<DistributionEntry> = combined
            .into_iter()
            .map(|(key, v)| {
                let p = if scale > 1.0 { v / scale } else { v };
                DistributionEntry { key, probability: p }
            })
            .collect();
        normalize(&mut entries)?;
        Distribution::new(entries).map_err(|e| EngineError::InvalidConfig {
            reason: format!("rotation combination produced an invalid distribution: {e}"),
        })
    }

    fn decide_extended(
        &self,
        state: &opencodifier_core::State,
        question: &DecisionQuestion,
        index: Option<&crate::lexical::Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        let _ = index;
        Ok((self.decide(state, question)?, None))
    }

    fn model_id(&self) -> &str {
        &self.id
    }
}

impl RotationReadout {
    /// The early-exit certificate.
    ///
    /// Claim to certify: the winner of the partial readout (n reads) is
    /// the winner the full-K readout would produce. The unread m = K − n
    /// rotations are adversarially worst-cased: every one of them is
    /// assumed to give the runner-up 1.0 and the current winner 0.0.
    /// The partial winner keeps the full-K lead only if
    /// `winner_sum > runnerup_sum + m` in raw per-read winner mass.
    /// That deterministic bound is then hardened the Clopper–Pearson
    /// way: the partial winner's per-read share `p̂ = winner_sum / n` is
    /// a binomial estimate, and its one-sided `1 − α` lower bound
    /// (`α = 1 − target_agreement`) must exceed `m / K` — the share the
    /// winner needs in the full-K vote to survive the unread block.
    /// Both conditions failing → not certified; keep reading.
    fn certified(reads: &[Distribution], total_k: usize, target_agreement: f64) -> bool {
        let n = reads.len();
        let m = total_k.saturating_sub(n);
        if m == 0 {
            return true; // full readout taken; trivially "certified"
        }
        // Winner/runner-up mass by the corrected combination — but the
        // certificate must be conservative, so it uses RAW winner counts:
        // each read votes for its argmax candidate. Counting raw argmax
        // wins avoids trusting the calibrated shapes of the unread
        // adversarial reads.
        let mut wins = std::collections::BTreeMap::<String, usize>::new();
        for read in reads {
            if let Some(best) =
                read.entries().iter().max_by(|a, b| a.probability.total_cmp(&b.probability))
            {
                *wins.entry(best.key.clone()).or_insert(0) += 1;
            }
        }
        let mut counts: Vec<usize> = wins.values().copied().collect();
        counts.sort_unstable();
        counts.reverse();
        let (w, r) = (counts.first().copied().unwrap_or(0), counts.get(1).copied().unwrap_or(0));
        #[allow(clippy::cast_precision_loss)]
        let (nf, mf, kf) = (n as f64, m as f64, total_k as f64);

        // Deterministic worst case: unread block unanimous for the runner-up.
        if f64::from(u16::try_from(w).unwrap_or(u16::MAX))
            <= f64::from(u16::try_from(r).unwrap_or(u16::MAX)) + mf
        {
            return false;
        }
        // CP lower bound on the winner's true per-read share.
        let alpha = 1.0 - target_agreement;
        let phat = f64::from(u16::try_from(w).unwrap_or(u16::MAX)) / nf;
        let lower = cp_lower_bound(w, n, alpha);
        // The winner needs share > m/K in the full-K vote.
        lower > mf / kf && phat > 0.5
    }
}

/// One-sided Clopper–Pearson lower bound for a binomial proportion:
/// the largest `p` with `P(X ≥ w | n, p) ≤ alpha` — below it, observing
/// `w` wins is surprising at level `alpha`. Equals the exact
/// Beta-quantile `BetaInv(alpha; w, n − w + 1)`. Computed by bisection
/// on the binomial tail (increasing in `p`): when `tail(mid) > alpha`
/// the bound lies below `mid`. 60 iterations on [0, 1] is exact to
/// f64 epsilon; cross-checked against `scipy.stats.beta.ppf`
/// (L(4,5,0.01) = 0.22207, L(3,3,0.01) = 0.10564).
fn cp_lower_bound(w: usize, n: usize, alpha: f64) -> f64 {
    if w == 0 {
        return 0.0;
    }
    if w == n {
        // P(X ≥ n | n, p) = p^n = alpha  ⇔  p = alpha^(1/n)
        #[allow(clippy::cast_precision_loss)]
        return alpha.powf(1.0 / f64::from(u16::try_from(n).unwrap_or(u16::MAX)));
    }
    #[allow(clippy::cast_precision_loss)]
    let nf = n as f64;
    let tail = |p: f64| -> f64 {
        // P(X >= w | n, p) = 1 - CDF(w-1)
        1.0 - binom_cdf(w - 1, nf, p)
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..60 {
        let mid = f64::midpoint(lo, hi);
        if tail(mid) > alpha {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    f64::midpoint(lo, hi)
}

/// Binomial CDF via the regularized incomplete beta identity:
/// P(X ≤ successes) = `BetaInc` evaluated by continued fraction
/// (Numerical-Recipes `betacf`), no external dependency.
#[allow(clippy::cast_precision_loss)]
fn binom_cdf(successes: usize, trials: f64, prob: f64) -> f64 {
    if f64::from(u16::try_from(successes).unwrap_or(u16::MAX)) >= trials {
        return 1.0;
    }
    if prob <= 0.0 {
        return 1.0;
    }
    if prob >= 1.0 {
        return 0.0;
    }
    let a = f64::from(u16::try_from(successes).unwrap_or(u16::MAX)) + 1.0;
    let b = trials - a + 1.0;
    // P(X ≤ successes) = I_{1-prob}(b, a)
    betainc(1.0 - prob, b, a)
}

/// Regularized incomplete beta `I_x(a, b)` via the standard continued
/// fraction (NR §6.4), with the symmetrical reflection for x > (a+1)/(a+b+2).
fn betainc(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln();
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(x, a, b) / a
    } else {
        1.0 - (front * betacf(1.0 - x, b, a) / b)
    }
}

/// Continued fraction for the incomplete beta (NR §6.4 betacf),
/// modified Lentz's method.
fn betacf(x: f64, aa: f64, bb: f64) -> f64 {
    const MAX_IT: usize = 200;
    const EPS: f64 = 3.0e-12;
    const FPMIN: f64 = 1.0e-300;
    let qab = aa + bb;
    let qap = aa + 1.0;
    let qam = aa - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAX_IT {
        #[allow(clippy::cast_precision_loss)]
        let mf = m as f64;
        let m2 = 2.0 * mf;
        let mut term_a = mf * (bb - mf) * x / ((qam + m2) * (aa + m2));
        d = 1.0 + term_a * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + term_a / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        term_a = -(aa + mf) * (qab + mf) * x / ((aa + m2) * (qap + m2));
        d = 1.0 + term_a * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + term_a / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// ln Γ(x) — Lanczos, g=7, n=9 coefficients.
fn ln_gamma(x: f64) -> f64 {
    const COF: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // reflection: Γ(x) = π / (sin(πx) Γ(1−x))
        (std::f64::consts::PI / (std::f64::consts::PI * x).sin() / ln_gamma(1.0 - x)).ln()
    } else {
        let x = x - 1.0;
        let mut a = COF[0];
        let t = x + 7.5;
        for (i, cof) in COF.iter().enumerate().skip(1) {
            #[allow(clippy::cast_precision_loss)]
            let idx = i as f64;
            a += cof / (x + idx);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (t).ln() * (x + 0.5) - t + a.ln()
    }
}

/// Renormalizes to sum 1 within the core's tolerance, guarding against
/// an all-zero correction.
fn normalize(entries: &mut [DistributionEntry]) -> EngineResult<()> {
    let sum: f64 = entries.iter().map(|e| e.probability).sum();
    if sum <= 0.0 || !sum.is_finite() {
        return Err(EngineError::InvalidConfig {
            reason: "rotation combination collapsed to zero mass".into(),
        });
    }
    for e in entries.iter_mut() {
        e.probability /= sum;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::RotationReadout;
    use crate::classifier::{Classifier, MockClassifier};
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionQuestion, Distribution, DistributionEntry, State,
    };
    use std::sync::Arc;

    /// A test classifier that is deliberately position-biased: it always
    /// answers with the FIRST candidate in the list it is shown, at the
    /// same confidence. An unbiased readout must defeat this.
    #[derive(Debug)]
    struct FirstBiased;

    impl Classifier for FirstBiased {
        fn decide(
            &self,
            _state: &State,
            question: &DecisionQuestion,
        ) -> Result<Distribution, crate::error::EngineError> {
            let DecisionQuestion::Choice(choice) = question else {
                return Err(crate::error::EngineError::InvalidConfig {
                    reason: "first-biased mock only answers choice".into(),
                });
            };
            #[allow(clippy::cast_precision_loss)]
            let denom = choice.candidates().len() as f64 - 1.0;
            let mut entries: Vec<DistributionEntry> = choice
                .candidates()
                .iter()
                .enumerate()
                .map(|(i, c)| DistributionEntry {
                    key: c.id().as_str().to_owned(),
                    probability: if i == 0 { 0.9 } else { 0.1 / denom },
                })
                .collect();
            let sum: f64 = entries.iter().map(|e| e.probability).sum();
            for e in &mut entries {
                e.probability /= sum;
            }
            Ok(Distribution::new(entries).unwrap())
        }

        fn model_id(&self) -> &'static str {
            "test/first-biased"
        }
    }

    fn three_candidates() -> Vec<Candidate> {
        vec![
            Candidate::new("a", "alpha").unwrap(),
            Candidate::new("b", "beta").unwrap(),
            Candidate::new("c", "gamma").unwrap(),
        ]
    }

    fn choice_question(candidates: Vec<Candidate>) -> DecisionQuestion {
        DecisionQuestion::Choice(ChoiceQuestion::new("q1", "which one", candidates).unwrap())
    }

    #[test]
    fn boolean_and_score_questions_pass_through() {
        let inner = MockClassifier::new("mock/inner");
        let rot = RotationReadout::new(Arc::new(inner), "rot/test");
        let bool_q = DecisionQuestion::Boolean(
            opencodifier_core::BooleanQuestion::new("b1", "is it so").unwrap(),
        );
        let d = rot.decide(&State::from_text("s"), &bool_q).unwrap();
        assert!((d.entries()[0].probability - 0.5).abs() < 1e-9);
    }

    #[test]
    fn single_candidate_passes_through() {
        let inner = MockClassifier::new("mock/inner");
        let rot = RotationReadout::new(Arc::new(inner), "rot/test");
        let q = choice_question(vec![Candidate::new("only", "the one").unwrap()]);
        let d = rot.decide(&State::from_text("s"), &q).unwrap();
        assert!((d.entries()[0].probability - 1.0).abs() < 1e-9);
    }

    #[test]
    fn uniform_inner_stays_uniform() {
        let inner = MockClassifier::new("mock/inner");
        let rot = RotationReadout::new(Arc::new(inner), "rot/test");
        let q = choice_question(three_candidates());
        let d = rot.decide(&State::from_text("s"), &q).unwrap();
        for e in d.entries() {
            assert!((e.probability - 1.0 / 3.0).abs() < 1e-9, "{e:?}");
        }
    }

    #[test]
    fn position_bias_is_corrected() {
        // The biased inner always picks the first-shown candidate. The
        // rotation readout shows every candidate at every position
        // exactly once, so the corrected distribution must be uniform —
        // the bias is divided out by the label prior.
        let rot = RotationReadout::new(Arc::new(FirstBiased), "rot/bias-test");
        let q = choice_question(three_candidates());
        let d = rot.decide(&State::from_text("s"), &q).unwrap();
        for e in d.entries() {
            assert!((e.probability - 1.0 / 3.0).abs() < 1e-6, "bias survived: {e:?}");
        }
    }

    #[test]
    fn deterministic_across_repeats() {
        let rot = RotationReadout::new(Arc::new(FirstBiased), "rot/det");
        let q = choice_question(three_candidates());
        let s = State::from_text("same state");
        let a = rot.decide(&s, &q).unwrap();
        let b = rot.decide(&s, &q).unwrap();
        for (x, y) in a.entries().iter().zip(b.entries()) {
            assert_eq!(x.key, y.key);
            assert_eq!(x.probability, y.probability);
        }
    }

    #[test]
    fn model_id_carries_the_configuration() {
        let rot = RotationReadout::new(Arc::new(MockClassifier::new("mock/i")), "rot/x1");
        assert_eq!(rot.model_id(), "rot/x1");
    }
}

#[cfg(test)]
mod early_exit_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::RotationReadout;
    use crate::classifier::Classifier;
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionQuestion, Distribution, DistributionEntry, State,
    };
    use std::sync::Arc;

    /// Unanimous: every read puts full mass on candidate `pick`.
    #[derive(Debug)]
    struct Unanimous(&'static str);

    impl Classifier for Unanimous {
        fn decide(
            &self,
            _state: &State,
            question: &DecisionQuestion,
        ) -> Result<Distribution, crate::error::EngineError> {
            let DecisionQuestion::Choice(choice) = question else {
                return Err(crate::error::EngineError::InvalidConfig {
                    reason: "unanimous mock only answers choice".into(),
                });
            };
            let entries: Vec<DistributionEntry> = choice
                .candidates()
                .iter()
                .map(|c| DistributionEntry {
                    key: c.id().as_str().to_owned(),
                    probability: if c.id().as_str() == self.0 { 1.0 } else { 0.0 },
                })
                .collect();
            Ok(Distribution::new(entries).unwrap())
        }

        fn model_id(&self) -> &'static str {
            "test/unanimous"
        }
    }

    /// Counts how many times the inner classifier was consulted.
    #[derive(Debug)]
    struct Counting(Arc<dyn Classifier>, std::sync::atomic::AtomicUsize);

    impl Counting {
        fn new(inner: Arc<dyn Classifier>) -> Self {
            Self(inner, std::sync::atomic::AtomicUsize::new(0))
        }
        fn count(&self) -> usize {
            self.1.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Classifier for Counting {
        fn decide(
            &self,
            state: &State,
            question: &DecisionQuestion,
        ) -> Result<Distribution, crate::error::EngineError> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.0.decide(state, question)
        }

        fn model_id(&self) -> &str {
            self.0.model_id()
        }
    }

    fn five_candidates() -> DecisionQuestion {
        DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "q",
                "which",
                vec![
                    Candidate::new("a", "one").unwrap(),
                    Candidate::new("b", "two").unwrap(),
                    Candidate::new("c", "three").unwrap(),
                    Candidate::new("d", "four").unwrap(),
                    Candidate::new("e", "five").unwrap(),
                ],
            )
            .unwrap(),
        )
    }

    #[test]
    fn easy_question_exits_early() {
        // A unanimous winner needs no rotation spread: the certificate
        // should stop at k_min, far below the full K = 5.
        // K = 9, unanimous winner, floor 4. Certification math: at n
        // reads, the winner's CP lower bound alpha^(1/n) must clear the
        // unread share (9-n)/9. n=4: 0.316 > 5/9=0.556? No. n=6:
        // 0.464 > 3/9=0.333? Yes — so it stops at 6, having skipped 3
        // reads. For small K the certificate is conservative (at K=5
        // unanimous it saves exactly one read); the savings grow with
        // roster size, which is where AnyJev deploys it.
        let nine = || {
            DecisionQuestion::Choice(
                ChoiceQuestion::new(
                    "q",
                    "which",
                    ('a'..='i')
                        .map(|c| Candidate::new(c.to_string(), c.to_string()).unwrap())
                        .collect(),
                )
                .unwrap(),
            )
        };
        let counting = Arc::new(Counting::new(Arc::new(Unanimous("a"))));
        let rot =
            RotationReadout::new(counting.clone(), "rot/early").with_early_exit(4, 0.99).unwrap();
        let d = rot.decide(&State::from_text("s"), &nine()).unwrap();
        let winner =
            d.entries().iter().max_by(|x, y| x.probability.total_cmp(&y.probability)).unwrap();
        assert_eq!(winner.key, "a");
        assert_eq!(d.entries().len(), 9, "full support must be reported");
        assert_eq!(counting.count(), 6, "should stop once certified, got {}", counting.count());
    }

    #[test]
    fn without_early_exit_reads_all() {
        let counting = Arc::new(Counting::new(Arc::new(Unanimous("a"))));
        let rot = RotationReadout::new(counting.clone(), "rot/full");
        rot.decide(&State::from_text("s"), &five_candidates()).unwrap();
        assert_eq!(counting.count(), 5);
    }

    #[test]
    fn invalid_params_refused() {
        let build = |k: usize, t: f64| {
            RotationReadout::new(Arc::new(Unanimous("a")), "rot/x")
                .with_early_exit(k, t)
                .map(|_| ())
        };
        assert!(build(0, 0.99).is_err());
        assert!(build(2, 1.0).is_err());
        assert!(build(2, 0.4).is_err());
    }

    #[test]
    fn cp_bound_known_values() {
        use super::cp_lower_bound;
        // w = n: bound is alpha^(1/n); alpha 0.01, n 3 → 0.01^(1/3) ≈ 0.2154
        assert!((cp_lower_bound(3, 3, 0.01) - 0.215_443_469).abs() < 1e-6);
        // scipy.stats.beta.ppf(0.01, 4, 2) = 0.22207228338499818
        assert!((cp_lower_bound(4, 5, 0.01) - 0.222_072_283).abs() < 1e-6);
        // scipy.stats.beta.ppf(0.01, 3, 3) = 0.10563984355077434
        assert!((cp_lower_bound(3, 5, 0.01) - 0.105_639_844).abs() < 1e-6);
        // w = 0: no evidence, bound 0
        assert_eq!(cp_lower_bound(0, 5, 0.01), 0.0);
        // monotone in w
        assert!(cp_lower_bound(4, 5, 0.01) > cp_lower_bound(3, 5, 0.01));
        // bound is below the point estimate
        assert!(cp_lower_bound(4, 5, 0.01) < 0.8);
    }
}
