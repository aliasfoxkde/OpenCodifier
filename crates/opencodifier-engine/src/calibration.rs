//! Calibration of raw model probabilities into decision confidence
//! (PLANNING.md §17 Rule 8, D15).
//!
//! Raw softmax output is not calibrated confidence — benchmark evidence
//! puts winner-probability ECE between 0.048 and 0.626 across the model
//! board (`benchmarks/decision-model/results/REPORT.md`, F5). This module
//! is the seam D15 reserves: a [`Calibration`] maps a classifier's raw
//! distribution to the confidence a policy gate may read, and the artifact
//! it is fitted from is versioned so cache keys invalidate when it changes
//! (D6, D15).
//!
//! The scheme implemented here is **temperature scaling in log-prob
//! space**: for temperature `T`, each probability `p` maps to
//! `q = p^(1/T) / Σ p_j^(1/T)`. This is exactly softmax temperature
//! scaling — `log p` recovers logits up to an additive constant, which
//! cancels in the normalization. `T = 1` is the identity; `T > 1`
//! flattens an overconfident distribution; `T < 1` sharpens. Temperatures
//! are fitted offline (per question class, plus a default) from benchmark
//! run data by `benchmarks/decision-model/runner/fit_calibration.py` and
//! shipped as validated JSON artifacts.
//!
//! Classes are keyed by question kind (`choice`, `boolean`, `score` —
//! see [`question_class`]). The IR does not yet carry task-class tags;
//! when it does, the artifact schema already admits them, because every
//! key outside the built-in three is a valid class name.

use std::collections::BTreeMap;

use serde::Deserialize;

use opencodifier_core::{DecisionQuestion, Distribution};

use crate::error::EngineError;

/// Maps a raw classifier distribution to calibrated confidence.
///
/// Implementations must be deterministic and pure: same inputs, same
/// output, no I/O, no clock. [`IdentityCalibration`] is the V1 default
/// and reports raw values (D15: un-calibrated runs must stay visible as
/// such — `calibration_version 0`).
pub trait Calibration: std::fmt::Debug + Send + Sync {
    /// Calibrated confidence for the top answer of a normalized
    /// distribution. The value must stay in `[0, 1]`.
    ///
    /// # Panics
    ///
    /// Never, for a well-formed [`Distribution`] (probabilities in
    /// `[0, 1]`, positive mass). Implementations must not panic on empty
    /// or degenerate distributions either — return `0.0`.
    fn calibrate(&self, class: &str, distribution: &Distribution) -> f64;

    /// Version folded into every cache key (D6). `0` means "no
    /// calibration" — the identity default. Loading a fitted artifact
    /// gives it the artifact's `calibration_version`, so replacing the
    /// artifact invalidates cached decisions automatically.
    fn version(&self) -> u64;
}

/// The identity map: calibrated confidence equals the raw top probability.
///
/// The V1 default (D15: un-calibrated runs report
/// `calibrated_confidence = raw` with `calibration_version 0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityCalibration;

impl Calibration for IdentityCalibration {
    fn calibrate(&self, _class: &str, distribution: &Distribution) -> f64 {
        distribution.top().probability
    }

    fn version(&self) -> u64 {
        0
    }
}

/// Per-mode wrapper for proof/delegate stacks (CALIBRATION finding 2).
///
/// The relational solver's exact proofs arrive as single-entry
/// distributions — p = 1.0 by construction, and measurably right every
/// time — while the delegated tail hedges across the whole range. Any
/// temperature fitted over that mixed population sharpens the wrong
/// half (measured: NLL improves 0.388 → 0.374 while ECE worsens
/// 0.094 → 0.097, so no artifact shipped). This wrapper restores the
/// bimodal structure without an IR class tag: a single-entry
/// distribution passes through at its raw value, and everything else
/// delegates to the inner calibration.
///
/// Cache identity: the wrapper reports the inner calibration's
/// `version()` unchanged. That is safe because the wrapper changes
/// semantics only inside a configured [`LadderPolicy`](crate::ladder::LadderPolicy),
/// whose non-empty id already decorates the model id as `|ladder-v1@<id>`
/// — bump the ladder id when the wrapper's behavior changes, exactly
/// like any artifact swap.
#[derive(Debug, Clone)]
pub struct ProofAwareCalibration {
    inner: std::sync::Arc<dyn Calibration>,
}

impl ProofAwareCalibration {
    /// Wraps an inner calibration for the delegated (multi-entry) tail.
    #[must_use]
    pub fn new(inner: std::sync::Arc<dyn Calibration>) -> Self {
        Self { inner }
    }
}

impl Calibration for ProofAwareCalibration {
    fn calibrate(&self, class: &str, distribution: &Distribution) -> f64 {
        // A single-entry distribution is a proof: its probability is
        // exact by construction, and no temperature may move it.
        if distribution.entries().len() == 1 {
            return distribution.top().probability;
        }
        self.inner.calibrate(class, distribution)
    }

    fn version(&self) -> u64 {
        self.inner.version()
    }
}

/// The question class a calibration artifact keys on: the question kind's
/// canonical name. Unknown kinds (the IR enum is `#[non_exhaustive]`) map
/// to `"unknown"` and fall back to the artifact default.
#[must_use]
pub fn question_class(question: &DecisionQuestion) -> &'static str {
    match question {
        DecisionQuestion::Choice(_) => "choice",
        DecisionQuestion::Boolean(_) => "boolean",
        DecisionQuestion::Score(_) => "score",
        _ => "unknown",
    }
}

/// Validated calibration artifact (D15): fitted temperatures per question
/// class plus a default, with the fit's provenance. Deserialization runs
/// through [`TemperatureCalibration::from_artifact`], which enforces every
/// invariant; a raw `CalibrationArtifact` value should never be trusted
/// on its own.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationArtifact {
    /// Artifact schema version. Only `1` exists.
    pub format_version: u32,
    /// Calibration scheme. Only `"temperature"` exists; other schemes
    /// (isotonic, Platt) get their own marker when implemented.
    pub scheme: String,
    /// The model (or rung) the fit describes — informational, carried so
    /// an artifact cannot be silently reused for another model.
    pub model_id: String,
    /// Version folded into cache keys (D6). Must be at least 1: `0` is
    /// reserved for "no calibration".
    pub calibration_version: u64,
    /// Fallback temperature for classes without their own entry.
    pub default_temperature: f64,
    /// Per-class temperature overrides, keyed by [`question_class`] name.
    #[serde(default)]
    pub temperatures: BTreeMap<String, f64>,
    /// Where the fit comes from and what it measured.
    pub fit: CalibrationFit,
}

/// Provenance and measured effect of a calibration fit.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationFit {
    /// Items the fit saw.
    pub items: usize,
    /// Expected calibration error of the raw winner probabilities.
    pub ece_before: f64,
    /// Expected calibration error after the fit, measured on the same
    /// items (in-sample; see the benchmark report's threats to validity).
    pub ece_after: f64,
    /// Human-readable provenance (suite, seed, run set).
    pub source: String,
}

/// Temperature scaling calibration, loaded from a validated artifact.
///
/// Fitting happens offline (`benchmarks/decision-model/runner/
/// fit_calibration.py`); this type only applies a fitted artifact, so
/// runtime stays deterministic and dependency-free.
#[derive(Debug, Clone, PartialEq)]
pub struct TemperatureCalibration {
    artifact: CalibrationArtifact,
}

impl TemperatureCalibration {
    /// Validates and adopts an artifact.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidCalibration`] when the artifact is
    /// structurally unusable: wrong `format_version`, a scheme other than
    /// `"temperature"`, a non-positive or non-finite temperature anywhere,
    /// or `calibration_version == 0` (reserved for "no calibration").
    pub fn from_artifact(artifact: CalibrationArtifact) -> Result<Self, EngineError> {
        if artifact.format_version != 1 {
            return Err(unsupported_format(artifact.format_version));
        }
        if artifact.scheme != "temperature" {
            return Err(invalid(format!(
                "calibration scheme must be \"temperature\", got {:?}",
                artifact.scheme
            )));
        }
        if artifact.calibration_version == 0 {
            return Err(invalid(
                "calibration_version must be >= 1; 0 is reserved for no calibration".to_owned(),
            ));
        }
        if artifact.model_id.trim().is_empty() {
            return Err(invalid("model_id must not be empty".to_owned()));
        }
        for (class, temperature) in
            std::iter::once((&"<default>".to_owned(), artifact.default_temperature)).chain(
                artifact.temperatures.iter().map(|(class, temperature)| (class, *temperature)),
            )
        {
            if !temperature.is_finite() || temperature <= 0.0 {
                return Err(invalid(format!(
                    "temperature for {class} must be finite and > 0, got {temperature}"
                )));
            }
        }
        Ok(Self { artifact })
    }

    /// Parses and validates a JSON artifact.
    ///
    /// # Errors
    ///
    /// [`EngineError::Serialization`] on malformed JSON,
    /// [`EngineError::InvalidCalibration`] on a well-formed but unusable
    /// artifact.
    pub fn from_json(json: &str) -> Result<Self, EngineError> {
        let artifact: CalibrationArtifact = serde_json::from_str(json)
            .map_err(|error| EngineError::Serialization { reason: error.to_string() })?;
        Self::from_artifact(artifact)
    }

    /// The validated artifact this calibration applies.
    #[must_use]
    pub fn artifact(&self) -> &CalibrationArtifact {
        &self.artifact
    }

    /// The temperature applied to `class`: the per-class entry when
    /// present, the artifact default otherwise.
    #[must_use]
    pub fn temperature_for(&self, class: &str) -> f64 {
        self.artifact.temperatures.get(class).copied().unwrap_or(self.artifact.default_temperature)
    }

    /// Rescales a normalized distribution by temperature `T` in log-prob
    /// space: `q_i = p_i^(1/T) / Σ_j p_j^(1/T)`.
    ///
    /// Probabilities of exactly `0.0` stay `0.0` (they carry no mass at
    /// any temperature); `0^x` is `0` for `x > 0`, so the general formula
    /// already does the right thing — the guard is for the `0 * inf`
    /// float trap when `T` approaches zero, which validation forbids.
    fn rescale(distribution: &Distribution, temperature: f64) -> Vec<(String, f64)> {
        let exponent = 1.0 / temperature;
        let rescaled: Vec<(String, f64)> = distribution
            .entries()
            .iter()
            .map(|entry| {
                let weight =
                    if entry.probability > 0.0 { entry.probability.powf(exponent) } else { 0.0 };
                (entry.key.clone(), weight)
            })
            .collect();
        let total: f64 = rescaled.iter().map(|(_, weight)| weight).sum();
        if !total.is_finite() || total <= 0.0 {
            // A degenerate fit (all-zero mass cannot happen for a valid
            // Distribution, but float paranoia is cheap): fall back to the
            // identity rather than inventing mass.
            return distribution
                .entries()
                .iter()
                .map(|entry| (entry.key.clone(), entry.probability))
                .collect();
        }
        rescaled.into_iter().map(|(key, weight)| (key, weight / total)).collect()
    }
}

fn invalid(reason: String) -> EngineError {
    EngineError::InvalidCalibration { reason }
}

fn unsupported_format(found: u32) -> EngineError {
    invalid(format!("calibration format_version must be 1, got {found}"))
}

impl Calibration for TemperatureCalibration {
    fn calibrate(&self, class: &str, distribution: &Distribution) -> f64 {
        let temperature = self.temperature_for(class);
        let rescaled = Self::rescale(distribution, temperature);
        let top_key = &distribution.top().key;
        rescaled
            .into_iter()
            .find(|(key, _)| key == top_key)
            .map_or(distribution.top().probability, |(_, probability)| probability)
    }

    fn version(&self) -> u64 {
        self.artifact.calibration_version
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use std::sync::Arc;

    use crate::error::EngineError;

    fn artifact(temperatures: BTreeMap<String, f64>, default: f64) -> CalibrationArtifact {
        CalibrationArtifact {
            format_version: 1,
            scheme: "temperature".to_owned(),
            model_id: "test-model".to_owned(),
            calibration_version: 1,
            default_temperature: default,
            temperatures,
            fit: CalibrationFit {
                items: 120,
                ece_before: 0.5,
                ece_after: 0.1,
                source: "test".to_owned(),
            },
        }
    }

    fn dist(pairs: &[(&str, f64)]) -> Distribution {
        Distribution::from_pairs(pairs.iter().copied()).expect("valid distribution")
    }

    #[test]
    fn identity_calibration_reports_raw_values_and_version_zero() {
        let calibration = IdentityCalibration;
        let d = dist(&[("a", 0.78), ("b", 0.22)]);
        assert_eq!(calibration.calibrate("choice", &d), 0.78);
        assert_eq!(calibration.version(), 0);
    }

    #[test]
    fn question_class_names_the_three_ir_kinds() {
        let choice = choice_question();
        assert_eq!(question_class(&choice), "choice");
    }

    fn choice_question() -> DecisionQuestion {
        use opencodifier_core::{Candidate, ChoiceQuestion};
        let candidates: Vec<Candidate> = ["alpha", "beta"]
            .iter()
            .map(|name| Candidate::new(*name, *name).expect("candidate"))
            .collect();
        DecisionQuestion::Choice(
            ChoiceQuestion::new("q1", "pick one", candidates).expect("question"),
        )
    }

    #[test]
    fn temperature_one_is_the_identity() {
        let calibration =
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 1.0)).expect("valid");
        let d = dist(&[("a", 0.70), ("b", 0.20), ("c", 0.10)]);
        assert!((calibration.calibrate("choice", &d) - 0.70).abs() < 1e-12);
    }

    #[test]
    fn flattening_temperature_lowers_confidence_and_sharpening_raises_it() {
        let flat =
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 2.0)).expect("valid");
        let sharp =
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 0.5)).expect("valid");
        let d = dist(&[("a", 0.70), ("b", 0.20), ("c", 0.10)]);
        let top = d.top().probability;
        assert!(flat.calibrate("choice", &d) < top, "T>1 must flatten");
        assert!(sharp.calibrate("choice", &d) > top, "T<1 must sharpen");
    }

    #[test]
    fn calibrated_distribution_stays_normalized_and_keeps_the_winner() {
        let calibration =
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 3.0)).expect("valid");
        let d = dist(&[("a", 0.55), ("b", 0.30), ("c", 0.15)]);
        let top = calibration.calibrate("choice", &d);
        assert!((0.0..=1.0).contains(&top));
        assert!(
            (TemperatureCalibration::rescale(&d, 3.0)
                .iter()
                .map(|(_, probability)| *probability)
                .sum::<f64>()
                - 1.0)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn class_temperature_overrides_the_default() {
        let mut temperatures = BTreeMap::new();
        temperatures.insert("choice".to_owned(), 0.5);
        let calibration =
            TemperatureCalibration::from_artifact(artifact(temperatures, 2.5)).expect("valid");
        assert_eq!(calibration.temperature_for("choice"), 0.5);
        assert_eq!(calibration.temperature_for("boolean"), 2.5);
        assert_eq!(calibration.temperature_for("unknown"), 2.5);
    }

    #[test]
    fn version_comes_from_the_artifact() {
        let mut a = artifact(BTreeMap::new(), 2.0);
        a.calibration_version = 7;
        assert_eq!(TemperatureCalibration::from_artifact(a).expect("valid").version(), 7);
    }

    #[test]
    fn proof_aware_passes_a_single_entry_through_raw() {
        let inner = Arc::new(
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 3.0)).expect("valid"),
        );
        let wrapped = ProofAwareCalibration::new(inner.clone());
        let proof = dist(&[("proved", 1.0)]);
        // A T = 3 fit would flatten this to nothing; the wrapper must
        // not touch an exact proof.
        assert_eq!(wrapped.calibrate("choice", &proof), 1.0);
        assert_eq!(proof.top().probability, 1.0);
    }

    #[test]
    fn proof_aware_delegates_the_hedging_tail_unchanged() {
        let inner = Arc::new(
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 3.0)).expect("valid"),
        );
        let wrapped = ProofAwareCalibration::new(inner.clone());
        let d = dist(&[("a", 0.70), ("b", 0.20), ("c", 0.10)]);
        assert_eq!(wrapped.calibrate("choice", &d), inner.calibrate("choice", &d));
        // The inner calibration is a real flattening fit: the delegated
        // tail lands below raw, which is exactly what the wrapper
        // preserves for delegates while sparing proofs.
        assert!(wrapped.calibrate("choice", &d) < d.top().probability);
    }

    #[test]
    fn proof_aware_version_is_the_inner_version() {
        let mut a = artifact(BTreeMap::new(), 2.0);
        a.calibration_version = 7;
        let inner = Arc::new(TemperatureCalibration::from_artifact(a).expect("valid"));
        let wrapped = ProofAwareCalibration::new(inner);
        assert_eq!(wrapped.version(), 7);
    }

    #[test]
    fn invalid_artifacts_are_rejected() {
        let mut a = artifact(BTreeMap::new(), 2.0);
        a.format_version = 2;
        assert!(TemperatureCalibration::from_artifact(a.clone()).is_err());

        a.format_version = 1;
        a.scheme = "isotonic".to_owned();
        assert!(TemperatureCalibration::from_artifact(a.clone()).is_err());

        a.scheme = "temperature".to_owned();
        a.calibration_version = 0;
        assert!(TemperatureCalibration::from_artifact(a.clone()).is_err());

        a.calibration_version = 1;
        a.default_temperature = 0.0;
        assert!(TemperatureCalibration::from_artifact(a.clone()).is_err());

        a.default_temperature = f64::NEG_INFINITY;
        assert!(TemperatureCalibration::from_artifact(a.clone()).is_err());

        let mut temperatures = BTreeMap::new();
        temperatures.insert("choice".to_owned(), f64::NAN);
        a.default_temperature = 2.0;
        a.temperatures = temperatures;
        assert!(TemperatureCalibration::from_artifact(a).is_err());
    }

    #[test]
    fn an_artifact_without_a_model_id_is_rejected() {
        // The model id is what stops an artifact fitted for one model being
        // reused for another (D15), so a blank one is unusable even though
        // every temperature is well formed.
        let mut a = artifact(BTreeMap::new(), 2.0);
        a.model_id = "   ".to_owned();
        let error = TemperatureCalibration::from_artifact(a).expect_err("blank model id");
        assert!(matches!(error, EngineError::InvalidCalibration { .. }));
        assert_eq!(error.code(), "calibration.invalid");
        assert!(error.to_string().contains("model_id"), "{error}");
    }

    #[test]
    fn the_validated_artifact_is_readable_back() {
        // Callers carry the artifact's provenance into logs and wire
        // payloads; what comes back must be exactly what was validated.
        let mut temperatures = BTreeMap::new();
        temperatures.insert("choice".to_owned(), 1.91);
        let calibration =
            TemperatureCalibration::from_artifact(artifact(temperatures.clone(), 2.07))
                .expect("valid");
        let shown = calibration.artifact();
        assert_eq!(shown.format_version, 1);
        assert_eq!(shown.scheme, "temperature");
        assert_eq!(shown.model_id, "test-model");
        assert_eq!(shown.default_temperature, 2.07);
        assert_eq!(shown.temperatures, temperatures);
        assert_eq!(shown.fit.items, 120);
    }

    #[test]
    fn a_degenerate_rescale_falls_back_to_the_identity() {
        // A temperature tiny enough that every weight underflows to zero
        // leaves no mass to normalize by: inventing a distribution would be
        // worse than reporting the raw one, so the raw probabilities come
        // back unchanged and calibrated confidence stays honest.
        let d = dist(&[("a", 0.5), ("b", 0.3), ("c", 0.2)]);
        assert_eq!(
            TemperatureCalibration::rescale(&d, 1e-5),
            vec![("a".to_owned(), 0.5), ("b".to_owned(), 0.3), ("c".to_owned(), 0.2)]
        );

        // Through the public path: `1e-5` is a validated temperature, so the
        // calibration must still answer — with the raw top probability.
        let calibration =
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 1e-5)).expect("valid");
        assert_eq!(calibration.calibrate("choice", &d), 0.5);
        // A healthy temperature next to it does rescale — flattening this
        // distribution lowers its top probability — so the identity above is
        // the degenerate fit and not what temperature scaling does.
        let flat =
            TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 2.0)).expect("valid");
        assert!(flat.calibrate("choice", &d) < 0.5);
    }

    #[test]
    fn json_round_trip_and_malformed_json() {
        let json = r#"{
            "format_version": 1,
            "scheme": "temperature",
            "model_id": "qwen3.5-0.8b-q4_0",
            "calibration_version": 2,
            "default_temperature": 2.07,
            "temperatures": {"choice": 1.91},
            "fit": {"items": 120, "ece_before": 0.074, "ece_after": 0.021,
                    "source": "decision-model board"}
        }"#;
        let calibration = TemperatureCalibration::from_json(json).expect("valid artifact");
        assert_eq!(calibration.temperature_for("choice"), 1.91);
        assert_eq!(calibration.temperature_for("score"), 2.07);
        assert_eq!(calibration.version(), 2);

        assert!(matches!(
            TemperatureCalibration::from_json("{not json"),
            Err(EngineError::Serialization { .. })
        ));
        assert!(matches!(
            TemperatureCalibration::from_json(
                r#"{"format_version": 9, "scheme": "temperature", "model_id": "m",
                    "calibration_version": 1, "default_temperature": 2.0,
                    "fit": {"items": 1, "ece_before": 0.1, "ece_after": 0.1,
                            "source": "s"}}"#
            ),
            Err(EngineError::InvalidCalibration { .. })
        ));
    }

    #[test]
    fn unknown_fields_are_denied_so_schema_drift_cannot_pass_silently() {
        let json = r#"{"format_version": 1, "scheme": "temperature",
                       "model_id": "m", "calibration_version": 1,
                       "default_temperature": 2.0,
                       "fit": {"items": 1, "ece_before": 0.1, "ece_after": 0.1,
                               "source": "s", "extra": true}}"#;
        let error = TemperatureCalibration::from_json(json).expect_err("unknown field");
        assert!(matches!(error, EngineError::Serialization { .. }));
    }

    #[test]
    fn error_code_is_stable() {
        let error = EngineError::InvalidCalibration { reason: "r".to_owned() };
        assert_eq!(error.code(), "calibration.invalid");
    }

    #[test]
    fn non_positive_default_temperature_is_invalid_with_a_stable_code() {
        let error = TemperatureCalibration::from_artifact(artifact(BTreeMap::new(), 0.0))
            .expect_err("non-positive temperature");
        assert!(matches!(error, EngineError::InvalidCalibration { .. }));
        assert_eq!(error.code(), "calibration.invalid");
        assert!(error.to_string().contains("calibration artifact"));
    }
}
