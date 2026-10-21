//! NLI verbalization verifier as an escalation rung (D39).
//!
//! The [`NliVerifier`](opencodifier_model::NliVerifier) implements the
//! engine's [`Classifier`] trait, so it slots into the escalation ladder
//! through the same [`Rung`](opencodifier_engine::Rung) path every other
//! classifier uses. The serving scorer (`OnnxNliScorer`) requires the `nli`
//! feature, but the contract — `NliVerifier`, `NliScorer`, `Verbalizer`,
//! and the `Classifier` impl — is always compiled. These tests use a scripted
//! scorer that returns canned entailment masses, exactly as the NLI module's
//! own unit tests do.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::sync::Arc;

use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionOutcome, DecisionQuestion, DecisionRequest, FactValue,
    ImageInput, RequestMetadata, State,
};
use opencodifier_engine::{
    Classifier, DecisionPolicy, EngineHandle, LadderPolicy, MockClassifier, Rung, RungModality,
};

use opencodifier_model::{ModelError, NliScorer, NliVerifier, Verbalizer};

// ---------------------------------------------------------------------------
// Scripted scorer

/// A scripted NLI scorer: canned entailment masses per hypothesis, in order.
#[derive(Debug)]
struct ScriptedNli {
    masses: Vec<f64>,
}

impl ScriptedNli {
    fn new(masses: Vec<f64>) -> Self {
        Self { masses }
    }
}

impl NliScorer for ScriptedNli {
    fn entailment_probabilities(
        &self,
        _premise: &str,
        hypotheses: &[String],
    ) -> Result<Vec<f64>, ModelError> {
        if self.masses.len() != hypotheses.len() {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "scripted {} masses for {} hypotheses",
                    self.masses.len(),
                    hypotheses.len()
                ),
            });
        }
        Ok(self.masses.clone())
    }
}

// ---------------------------------------------------------------------------
// Helpers

fn five_candidates() -> Vec<Candidate> {
    vec![
        Candidate::new("cloud-large"),
        Candidate::new("local-tiny"),
        Candidate::new("hybrid"),
        Candidate::new("serverless"),
        Candidate::new("edge"),
    ]
}

fn choice_question(text: &str, candidates: &[Candidate]) -> ChoiceQuestion {
    ChoiceQuestion::new(text, candidates.to_vec()).unwrap()
}

fn state() -> State {
    State::from_text("Summarize research across many sources and compare findings")
}

fn choice_request(q: ChoiceQuestion) -> DecisionRequest {
    DecisionRequest::new(state(), vec![q], DecisionPolicy::default(), RequestMetadata::default())
        .unwrap()
}

fn trace_fact(trace: &opencodifier_core::DecisionTrace, node: &str, fact: &str) -> Option<&str> {
    trace.entries.iter().find(|e| e.node == node)?.detail.get(fact)?.as_str()
}

fn nli_verifier(masses: Vec<f64>) -> Arc<dyn Classifier> {
    Arc::new(NliVerifier::new(
        Arc::new(ScriptedNli::new(masses)),
        "nli-test:model.onnx",
        Verbalizer::BareDescription,
    ))
}

fn nli_rung(masses: Vec<f64>) -> Rung {
    Rung::new(nli_verifier(masses))
}

/// A primary that lands in the verify band: 0.6 top probability,
///
/// 0.4 for the runner-up.
fn verify_primary() -> Arc<dyn Classifier> {
    Arc::new(
        MockClassifier::new("mock/primary")
            .with_script("model", vec![("cloud-large", 0.6), ("local-tiny", 0.4)])
            .unwrap(),
    )
}

/// A config with min_confidence = 1.0 so nothing auto-accepts.
fn engine_config() -> opencodifier_engine::EngineConfig {
    opencodifier_engine::EngineConfig::default().with_min_confidence(1.0)
}

// ---------------------------------------------------------------------------
// Tests

/// The NLI rung fires when the primary's gate does not accept.
///
/// Both candidates receive their entailment masses renormalized: alpha=0.7,
/// beta=0.3. Since alpha is top, the NLI distribution is [0.7, 0.3] which
/// is well above the default accept gate, so this verify-band primary
/// escalates and the NLI rung accepts.
#[test]
fn nli_rung_fires_on_verify_band_primary() {
    let engine = EngineHandle::with_rungs(
        engine_config(),
        verify_primary(),
        None,
        LadderPolicy::default(),
        vec![nli_rung(vec![0.7, 0.3])],
    )
    .unwrap();

    let response =
        engine.decide(&choice_request(choice_question("model", &five_candidates()))).unwrap();
    assert_eq!(
        response.outcome(),
        DecisionOutcome::Accept,
        "NLI rung accepted after renormalization"
    );
    let chain = trace_fact(response.trace(), "threshold", "rung_chain")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(chain.contains("nli-test:model.onnx:bare"), "NLI rung fired: {chain}");
}

/// The NLI rung abstains when all entailment mass is zero.
#[test]
fn nli_rung_refuses_degenerate_entailment() {
    let engine = EngineHandle::with_rungs(
        engine_config(),
        verify_primary(),
        None,
        LadderPolicy::default(),
        vec![nli_rung(vec![0.0, 0.0])],
    )
    .unwrap();

    let response =
        engine.decide(&choice_request(choice_question("model", &five_candidates()))).unwrap();
    // A refused distribution is surfaced as Verify on that question.
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
}

/// The NLI rung with mismatched masses produces the correct winner.
#[test]
fn nli_rung_orders_by_entailment_mass() {
    // beta gets higher entailment mass even though the primary ranked alpha first.
    // NLI renormalized: beta=0.75, alpha=0.25.
    let engine = EngineHandle::with_rungs(
        engine_config(),
        verify_primary(),
        None,
        LadderPolicy::default(),
        vec![nli_rung(vec![0.25, 0.75])], // [alpha, beta] entailment
    )
    .unwrap();

    let response =
        engine.decide(&choice_request(choice_question("model", &five_candidates()))).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    let answer = match response.answers().first().unwrap() {
        opencodifier_core::DecisionAnswer::Choice { choice, .. } => choice.as_str(),
        other => panic!("unexpected answer shape: {other:?}"),
    };
    // beta has the higher entailment mass after renormalization
    assert_eq!(answer, "local-tiny", "NLI reorder flipped the primary's preference");
}

/// An accepting primary never fires the NLI rung (D27 — exactly one classifier).
#[test]
fn accepting_primary_skips_nli_rung() {
    let accepting_primary = Arc::new(
        MockClassifier::new("mock/primary")
            .with_script("model", vec![("cloud-large", 0.95), ("local-tiny", 0.05)])
            .unwrap(),
    );
    let engine = EngineHandle::with_rungs(
        engine_config(),
        accepting_primary,
        None,
        LadderPolicy::default(),
        vec![nli_rung(vec![0.25, 0.75])],
    )
    .unwrap();

    let response =
        engine.decide(&choice_request(choice_question("model", &five_candidates()))).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    let chain = trace_fact(response.trace(), "threshold", "rung_chain")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(chain.is_empty(), "no rung fired on accepting primary: {chain}");
}

/// The NLI verifier's model id encodes the template tag, so the cache key
/// discriminates between `bare` and `answer-is` verbalizers (D6).
#[test]
fn nli_model_id_encodes_template_tag() {
    let bare = NliVerifier::new(
        Arc::new(ScriptedNli::new(vec![0.5, 0.5])),
        "nli-zeroshot:model.onnx",
        Verbalizer::BareDescription,
    );
    let answer_is = NliVerifier::new(
        Arc::new(ScriptedNli::new(vec![0.5, 0.5])),
        "nli-zeroshot:model.onnx",
        Verbalizer::TheAnswerIs,
    );
    assert_eq!(bare.model_id(), "nli-zeroshot:model.onnx:bare");
    assert_eq!(answer_is.model_id(), "nli-zeroshot:model.onnx:answer-is");
    assert_ne!(bare.model_id(), answer_is.model_id());
}

/// The NLI rung is skipped for an image-bearing state when it is text-only
/// (D38 — same modality routing as every other rung).
#[test]
fn nli_rung_is_modality_skipped_for_image_state() {
    let nli = nli_rung(vec![0.7, 0.3]).with_modality(RungModality::Text);
    let engine = EngineHandle::with_rungs(
        engine_config(),
        verify_primary(),
        None,
        LadderPolicy::default(),
        vec![nli],
    )
    .unwrap();

    // Image-bearing state via State::with_fact
    let img_state = State::from_text("describe this")
        .with_fact("image", FactValue::Image(ImageInput::Png(b"fake".to_vec())));
    let request = DecisionRequest::new(
        img_state,
        vec![choice_question("model", &five_candidates())],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();

    let response = engine.decide(&request).unwrap();
    // Image state with a text-only NLI rung skips it (routing, not abstention)
    // and the verify-band primary remains verify.
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    let skipped = trace_fact(response.trace(), "threshold", "rungs_skipped")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(skipped.contains("nli-test"), "NLI rung was not skipped: {skipped}");
}
