//! Image-modality rungs (D38): the walk skips an image rung over a
//! text-only state and traces the skip, an image-bearing request without
//! an image-capable rung is refused before any classifier runs, and the
//! rung's modality rides the cache-key decoration.
//!
//! Every test drives the real engine through the public surface — the
//! same surface an interface uses — and a counting or capturing
//! classifier makes the routing observable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::{choice_question, config, five_candidates, request, trace_fact};
use opencode_core::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest, Distribution, FactValue,
    ImageInput, ImageMediaType, RequestMetadata, State,
};
use opencode_engine::{
    Classifier, DecisionEngine, EngineError, EngineHandle, LadderPolicy, MockClassifier, Rung,
    RungModality, SystemClock,
};

/// An image-bearing state: the same research text the other fixtures
/// use, plus one attached PNG.
fn image_state() -> State {
    State::from_text("Summarize research across many sources and compare findings")
        .with_image(ImageInput::new(ImageMediaType::Png, vec![0x89, 0x50, 0x4E, 0x47]))
}

/// An image-bearing one-question choice request.
fn image_request() -> DecisionRequest {
    request(
        image_state(),
        vec![choice_question("model", &five_candidates())],
        RequestMetadata::default(),
    )
}

/// A classifier that counts its decisions — the observable for "the
/// walk never called this rung".
#[derive(Debug, Clone)]
struct Counting {
    calls: Arc<AtomicUsize>,
    delegate: MockClassifier,
    model_id: &'static str,
}

impl Counting {
    fn scripted(
        model_id: &'static str,
        top: (&str, f64),
        other: (&str, f64),
    ) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let delegate =
            MockClassifier::new(model_id).with_script("model", vec![top, other]).unwrap();
        (Self { calls: Arc::clone(&calls), delegate, model_id }, calls)
    }
}

impl Classifier for Counting {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.delegate.decide(state, question)
    }

    fn model_id(&self) -> &str {
        self.model_id
    }
}

/// A classifier that records the state it was handed — the observable
/// for "the image rung actually saw the images".
#[derive(Debug, Clone, Default)]
struct Capturing {
    seen: Arc<Mutex<Vec<(bool, usize)>>>,
}

impl Classifier for Capturing {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        self.seen.lock().unwrap().push((state.has_images(), state.image_count()));
        MockClassifier::new("mock/vision")
            .with_script("model", vec![("local-tiny", 0.9), ("cloud-large", 0.1)])
            .unwrap()
            .decide(state, question)
    }

    fn model_id(&self) -> &'static str {
        "mock/vision"
    }
}

fn engine_with_rungs(primary: Arc<dyn Classifier>, rungs: Vec<Rung>) -> DecisionEngine {
    DecisionEngine::new_with_rungs(config(1), Arc::new(SystemClock), primary, None, rungs).unwrap()
}

/// A handle over the same shape — used where the test reads the engine's
/// composed identity (`EngineHandle::identity`).
fn handle_with_rungs(primary: Arc<dyn Classifier>, rungs: Vec<Rung>) -> EngineHandle {
    EngineHandle::with_rungs(config(1), primary, None, LadderPolicy::default(), rungs).unwrap()
}

fn image_rung(classifier: Arc<dyn Classifier>) -> Rung {
    Rung::new(classifier).with_modality(RungModality::Image)
}

#[test]
fn an_image_request_without_an_image_rung_is_refused_before_any_classifier() {
    // Text primary + text rung: nothing can read the images. The refusal
    // happens at validation, so no classifier is ever charged for a
    // decision it was never equipped to make.
    let (primary, primary_calls) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let (fallback, _) = Counting::scripted("mock/rung", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), vec![Rung::new(Arc::new(fallback))]);

    let error = engine.decide(&image_request()).unwrap_err();
    assert!(matches!(error, EngineError::VisionRequired { images: 1 }), "{error:?}");
    assert_eq!(error.code(), "engine.vision_required");
    assert!(error.to_string().contains("1 image(s)"), "{error}");
    assert_eq!(primary_calls.load(Ordering::SeqCst), 0, "refused before any classifier ran");

    // A zero-rung engine refuses the same way — images are never
    // silently dropped on the floor.
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let bare = engine_with_rungs(Arc::new(primary), Vec::new());
    let error = bare.decide(&image_request()).unwrap_err();
    assert!(matches!(error, EngineError::VisionRequired { images: 1 }), "{error:?}");
}

#[test]
fn a_text_only_request_is_unchanged_by_the_vision_gate() {
    // The gate reads `image_count() > 0`: every pre-D38 payload (images
    // default to empty) walks the same path as before.
    let (primary, primary_calls) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), Vec::new());
    let response = engine.decide(&common::choice_request(&five_candidates())).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn an_accepting_primary_owns_the_answer_even_with_an_image_rung_waiting() {
    // The text primary still decides first (C11): a confident primary
    // means the image rung is never charged — the escalation tail fires
    // only on a non-accepting gate.
    let (primary, primary_calls) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let (vision, vision_calls) =
        Counting::scripted("mock/vision", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), vec![image_rung(Arc::new(vision))]);

    let response = engine.decide(&image_request()).unwrap();
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(vision_calls.load(Ordering::SeqCst), 0, "an accepted question ends the walk");
    assert_eq!(trace_fact(&response, "threshold", "rungs_skipped"), None);
    assert_eq!(trace_fact(&response, "threshold", "rungs_fired"), None);
    let answer = match response.answers().first().unwrap() {
        DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
        other => panic!("unexpected answer shape: {other:?}"),
    };
    assert_eq!(answer, "cloud-large");
}

#[test]
fn a_text_only_walk_skips_the_image_rung_and_traces_the_skip() {
    // No images, the primary lands in the verify band, and the only
    // rung is image-only: the walk skips it without error — skipping is
    // routing, not abstention — and the outcome stays the primary's
    // verdict (Verify, unverified: no verifier configured).
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.3), ("local-tiny", 0.7));
    let (vision, vision_calls) =
        Counting::scripted("mock/vision", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), vec![image_rung(Arc::new(vision))]);

    let response = engine.decide(&common::choice_request(&five_candidates())).unwrap();
    assert_eq!(vision_calls.load(Ordering::SeqCst), 0, "the image rung never saw a text state");
    assert_eq!(trace_fact(&response, "threshold", "rungs_fired"), None);
    let skipped = trace_fact(&response, "threshold", "rungs_skipped")
        .and_then(FactValue::as_str)
        .unwrap_or_default();
    assert_eq!(skipped, "mock/vision", "the trace names the rung the walk routed past");
    assert_eq!(response.outcome(), DecisionOutcome::Verify, "the primary's verdict stands");
}

#[test]
fn an_image_request_reaches_the_image_rung() {
    // Images present, primary gates low: the image rung fires, answers
    // decisively, and owns the shipped answer.
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.3), ("local-tiny", 0.7));
    let (vision, vision_calls) =
        Counting::scripted("mock/vision", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), vec![image_rung(Arc::new(vision))]);

    let response = engine.decide(&image_request()).unwrap();
    assert_eq!(vision_calls.load(Ordering::SeqCst), 1);
    assert_eq!(trace_fact(&response, "threshold", "rungs_fired"), Some(&FactValue::Integer(1)));
    assert_eq!(trace_fact(&response, "threshold", "rungs_skipped"), None);
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    let answer = match response.answers().first().unwrap() {
        DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
        other => panic!("unexpected answer shape: {other:?}"),
    };
    assert_eq!(answer, "local-tiny", "the rung's answer ships");
}

#[test]
fn the_image_rung_receives_the_image_bearing_state() {
    // The classifier contract is unchanged — the rung sees the full
    // state, images included, and reports what it saw.
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.3), ("local-tiny", 0.7));
    let capturing = Arc::new(Capturing::default());
    let seen = Arc::clone(&capturing.seen);
    let engine = engine_with_rungs(Arc::new(primary), vec![image_rung(capturing)]);

    engine.decide(&image_request()).unwrap();
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[(true, 1)],
        "the image rung saw one image-bearing state"
    );
}

#[test]
fn modality_is_part_of_the_rungs_identity() {
    // `:image` marks image-capable rungs in the rungs-v1 signature, so
    // flipping a rung's modality — which changes which requests the
    // walk runs it on — re-keys cached decisions mechanically.
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let (text_rung, _) = Counting::scripted("mock/rung", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let (image, _) = Counting::scripted("mock/rung", ("local-tiny", 0.9), ("cloud-large", 0.1));

    let plain = handle_with_rungs(Arc::new(primary.clone()), Vec::new());
    let text = handle_with_rungs(Arc::new(primary.clone()), vec![Rung::new(Arc::new(text_rung))]);
    let vision = handle_with_rungs(Arc::new(primary), vec![image_rung(Arc::new(image))]);

    let base = plain.identity().model_id;
    assert!(!base.contains("rungs-v1"), "{base}");
    assert_eq!(text.identity().model_id, format!("{base}|rungs-v1@mock/rung@0"));
    assert_eq!(vision.identity().model_id, format!("{base}|rungs-v1@mock/rung:image@0"));
    assert_ne!(text.identity().model_id, vision.identity().model_id);
}
