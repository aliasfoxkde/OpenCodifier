//! The `SigLIP2` zero-shot vision readout — the image-modality rung of
//! the escalation ladder (track V phases E–G; D38).
//!
//! A vision-language dual encoder embeds the request's images and the
//! question's answer space into **one shared space**; the decision is
//! the normalized dot product between them, scaled and softmaxed in
//! Rust f64 (D7). Like `nli`, the module splits:
//!
//! * the **contract** — the [`ImageTextEmbedder`] seam, the
//!   [`Siglip2Config`] head, the [`zero_shot_scores`] math, the
//!   [`PromptTemplate`] verbalizers, and the
//!   [`VisionClassifier`] — always compiles, and its tests run
//!   feature-free over [`MockImageTextEmbedder`] and scripted
//!   embedders;
//! * the **serving transport** — decode, resize, normalize, and the
//!   two ONNX towers — sits behind the crate's `vision` feature
//!   (track G), weights never fetched at build time (D14), with the
//!   compute-host parity runner reproducing the real arm of record
//!   (`onnx-community/siglip2-base-patch16-256-ONNX`) in track I.
//!
//! Honesty rules, carried over from the other rungs: every probability
//! is computed from returned embeddings in Rust (D7) — none is
//! synthesized; a state with no images is a typed refusal, never a
//! silently text-only decision (the same posture the engine enforces
//! at the ladder gate); and the prompt template is part of the model
//! id, because changing the template changes the distribution and must
//! invalidate every cached decision (D6).
//!
//! The arm of record normalizes both towers' outputs at its forward
//! head, so [`zero_shot_scores`] normalizes defensively too: a
//! compliant transport is unchanged, and a backend that forgets still
//! yields cosine semantics rather than scale-dependent garbage.

use std::sync::Arc;

use opencodifier_core::{
    BooleanQuestion, ChoiceQuestion, DecisionQuestion, Distribution, ImageInput, ScoreQuestion,
    State,
};
use opencodifier_engine::error::{EngineError, EngineResult};
use opencodifier_engine::{Classifier, negation_polarity, softmax};

use crate::error::ModelError;

/// The `SigLIP2` family's config init for the logit scale, used when no
/// serving artifact supplies the arm's fitted value. The `vision`
/// transport (track G) reads the real number from the model
/// directory's `config.json`; tests and the contract pin this default.
pub const DEFAULT_LOGIT_SCALE: f64 = 10.0;

/// The head of a `SigLIP2` zero-shot readout: everything about the
/// decision math that is not the embeddings themselves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Siglip2Config {
    /// Multiplies every image·text dot product before the softmax. The
    /// arm of record learns this value; larger scales sharpen the
    /// distribution toward the argmax. Must be finite and positive.
    pub logit_scale: f64,
}

impl Siglip2Config {
    /// The family-default head ([`DEFAULT_LOGIT_SCALE`]).
    #[must_use]
    pub fn new() -> Self {
        Self { logit_scale: DEFAULT_LOGIT_SCALE }
    }

    /// Validates and sets [`Siglip2Config::logit_scale`].
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] for a scale that is not
    /// finite and positive — such a value would fabricate or destroy
    /// evidence in the softmax.
    pub fn with_logit_scale(mut self, logit_scale: f64) -> Result<Self, ModelError> {
        if !logit_scale.is_finite() || logit_scale <= 0.0 {
            return Err(ModelError::ContractViolation {
                reason: format!("logit_scale must be finite and positive, got {logit_scale}"),
            });
        }
        self.logit_scale = logit_scale;
        Ok(self)
    }
}

impl Default for Siglip2Config {
    fn default() -> Self {
        Self::new()
    }
}

/// The dimensionality the arm of record (`siglip2-base-patch16-256`)
/// projects both towers to: the contract module's own fact, checked
/// against every graph output the transport sees, so a wrong-geometry
/// export fails typed instead of producing meaninglessly-shaped
/// embeddings.
pub const SIGLIP2_BASE_DIMS: usize = 768;

/// The square resolution the arm of record covers: `config.json`'s
/// `vision_config.image_size`, which the preprocessing resizes to.
/// Also carried by the config constructor below.
pub const SIGLIP2_BASE_IMAGE_SIZE: usize = 256;

/// Narrows a validated-bounded u64 config value to `usize` for the
/// tensor geometry path — the only conversion `config_from_dir_json`
/// performs above its bounds checks.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] on a target that cannot hold the
/// value (only on 16-bit platforms — the checks above bound by 16384).
fn narrow(value: u64, what: &str) -> Result<usize, ModelError> {
    usize::try_from(value).map_err(|_| ModelError::ContractViolation {
        reason: format!("config.json `{what}` value {value} does not fit usize"),
    })
}

/// Reads a serving [`Siglip2Config`] plus the geometry facts an
/// embedder needs out of the arm-of-record's `config.json`.
///
/// The document the published `SigLIP2` repositories ship nests the two
/// towers, so both lookups are explicit: `vision_config.image_size`
/// (the square the preprocessing must produce) and
/// `text_config.hidden_size` (the width of every returned vector,
/// defaulting to the base arm's [`SIGLIP2_BASE_DIMS`], with a
/// smaller/foreign width refused).
///
/// The head scale stays at [`DEFAULT_LOGIT_SCALE`]: some published
/// configs omit the learned scale; the calibration pass (track J)
/// supplies the fitted value.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the JSON is unparsable, the
/// image size is missing or smaller than a single patch, or the
/// hidden size is present but outside 1..=4096 — all are geometry the
/// exact tower could never decide with anyway.
pub fn config_from_dir_json(json: &str) -> Result<(Siglip2Config, usize, usize), ModelError> {
    let document: serde_json::Value = serde_json::from_str(json).map_err(|error| {
        ModelError::ContractViolation { reason: format!("config.json: {error}") }
    })?;
    let vision = document.get("vision_config").unwrap_or(&document);
    let image_size =
        vision.get("image_size").and_then(serde_json::Value::as_u64).ok_or_else(|| {
            ModelError::ContractViolation {
                reason: "config.json carries no integer `vision_config.image_size`".to_owned(),
            }
        })?;
    if image_size == 0 {
        return Err(ModelError::ContractViolation {
            reason: "config.json image_size must be positive".to_owned(),
        });
    }
    // Every published SigLIP2 arm is 32..=1024; anything above this
    // bound is not an export this transport believes it can session.
    if image_size > 16384 {
        return Err(ModelError::ContractViolation {
            reason: format!("config.json image_size {image_size} is above the 16384 binding"),
        });
    }
    let dims = match document.get("text_config").and_then(|text| text.get("hidden_size")) {
        Some(serde_json::Value::Number(number)) => {
            let hidden = number.as_u64().ok_or_else(|| ModelError::ContractViolation {
                reason: format!("config.json hidden_size {number} is not a positive integer"),
            })?;
            if hidden == 0 || hidden > 4096 {
                return Err(ModelError::ContractViolation {
                    reason: format!("config.json hidden_size {hidden} is outside 1..=4096"),
                });
            }
            hidden
        }
        _ => SIGLIP2_BASE_DIMS as u64,
    };
    Ok((Siglip2Config::new(), narrow(image_size, "image_size")?, narrow(dims, "hidden_size")?))
}

/// How an answer's subject becomes the text an image is scored
/// against. The template is a measured lever, not a style choice (the
/// NLI probe moved its arm from below chance to usable by changing the
/// scaffold), so each variant is named, tagged, and cached: the tag is
/// part of the classifier's model id (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptTemplate {
    /// `"<question>\n<subject>"` — the question asked, then the answer
    /// subject. Keeps the readout anchored to what was asked, the same
    /// composition the embedding rung uses for its query side.
    #[default]
    QuestionSubject,
    /// `"a photo of <subject>."` — the `SigLIP` zero-shot convention.
    PhotoOf,
}

impl PromptTemplate {
    /// The prompt an answer subject renders into under this template.
    ///
    /// `question` is the question's text and `subject` the answer
    /// space member (a candidate's description, a score level's
    /// label). Booleans do not pass through templates: their evidence
    /// prompt is the question text itself, mirroring the embedding
    /// rung's evidence readout.
    #[must_use]
    pub fn prompt(&self, question: &str, subject: &str) -> String {
        match self {
            Self::QuestionSubject => format!("{question}\n{subject}"),
            Self::PhotoOf => format!("a photo of {subject}."),
        }
    }

    /// The template's cache-key tag: part of [`Classifier::model_id`].
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::QuestionSubject => "question-subject",
            Self::PhotoOf => "photo-of",
        }
    }

    /// Parses a template tag (the inverse of [`PromptTemplate::tag`]).
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] for a tag this module does
    /// not define.
    pub fn from_tag(tag: &str) -> Result<Self, ModelError> {
        match tag {
            "question-subject" => Ok(Self::QuestionSubject),
            "photo-of" => Ok(Self::PhotoOf),
            other => Err(ModelError::ContractViolation {
                reason: format!("unknown prompt template tag {other:?}"),
            }),
        }
    }
}

/// The shared embedding space of a vision-language dual encoder.
///
/// One model, two encoders, one space: [`embed_images`](Self::embed_images)
/// and [`embed_texts`](Self::embed_texts) must be answered by the same
/// artifact, or the dot products are meaningless. Vectors are `f32`
/// and exactly [`dims`](Self::dims) wide; compliant implementations
/// return L2-normalized vectors (the `SigLIP2` forward head does, and
/// the mock does), and [`zero_shot_scores`] normalizes defensively
/// regardless.
///
/// Implementations must be deterministic for identical inputs given
/// the same [`model_id`](Self::model_id) — the decision cache is keyed
/// on exactly these outputs.
pub trait ImageTextEmbedder: std::fmt::Debug + Send + Sync {
    /// Stable identifier of the model this embedder serves. Feeds cache
    /// keys exactly like an inference model id.
    fn model_id(&self) -> &str;

    /// The fixed width of every vector this embedder returns.
    fn dims(&self) -> usize;

    /// Embeds each image, preserving order.
    ///
    /// # Errors
    ///
    /// Implementation-defined ([`ModelError`]); an embedder must never
    /// panic on any input it is handed.
    fn embed_images(&self, images: &[ImageInput]) -> Result<Vec<Vec<f32>>, ModelError>;

    /// Embeds each text, preserving order.
    ///
    /// # Errors
    ///
    /// Implementation-defined ([`ModelError`]); an embedder must never
    /// panic on any input it is handed.
    fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError>;
}

/// The zero-shot head math: normalized dot products scaled by
/// `logit_scale`, in the D7 order — f32 accumulation exactly as the
/// graph's f32 outputs arrive, promotion to f64 only at the scale.
///
/// Both sides are L2-normalized first (the zero-vector convention: a
/// vector with no norm is *no evidence*, so its score is `0.0`, never
/// `NaN`). `texts` must carry one vector per answer; length agreement
/// is the caller's invariant.
#[must_use]
pub fn zero_shot_scores(image: &[f32], texts: &[&[f32]], logit_scale: f64) -> Vec<f64> {
    let image = normalized(image);
    texts
        .iter()
        .map(|text| {
            let text = normalized(text);
            let dot: f32 = image.iter().zip(text.iter()).map(|(x, y)| x * y).sum();
            f64::from(dot) * logit_scale
        })
        .collect()
}

/// L2-normalized copy of `vector`; a zero-norm vector is returned
/// unchanged (the no-evidence convention).
fn normalized(vector: &[f32]) -> Vec<f32> {
    let norm: f32 = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 { vector.iter().map(|value| value / norm).collect() } else { vector.to_vec() }
}

/// The elementwise mean of image embeddings, renormalized.
///
/// A multi-image state is one piece of evidence pooled, not several
/// independent decisions; the mean keeps a single shared-space vector.
/// A single image is pooled to itself, normalized — so the common case
/// is identical to scoring it directly. An empty slice is `None` — the
/// caller refuses, it never fabricates evidence.
fn pooled_image(images: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = images.first()?;
    let mut pooled = vec![0.0_f32; first.len()];
    for image in images {
        for (slot, value) in pooled.iter_mut().zip(image.iter()) {
            *slot += value;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let count = images.len() as f32;
    for value in &mut pooled {
        *value /= count;
    }
    Some(normalized(&pooled))
}

/// The `SigLIP2` zero-shot vision rung as an engine [`Classifier`].
///
/// Construct it around any [`ImageTextEmbedder`] (the ONNX transport
/// behind the `vision` feature, or the mock and scripted embedders in
/// tests). The classifier refuses a text-only state: the engine's
/// ladder gate keeps image rungs off text-only decisions, and a direct
/// call with no images is a caller bug worth naming, not an empty
/// answer to paper over.
#[derive(Debug)]
pub struct VisionClassifier {
    embedder: Arc<dyn ImageTextEmbedder>,
    config: Siglip2Config,
    template: PromptTemplate,
    model_id: String,
}

impl VisionClassifier {
    /// Builds a vision classifier over `embedder`.
    ///
    /// The model id folds the embedder's identity and the template
    /// tag: either changing the artifact or the prompt changes the
    /// distribution, so either must invalidate cached decisions (D6).
    #[must_use]
    pub fn new(
        embedder: Arc<dyn ImageTextEmbedder>,
        config: Siglip2Config,
        template: PromptTemplate,
    ) -> Self {
        Self {
            model_id: format!("{}:siglip2-zero-shot:{}", embedder.model_id(), template.tag()),
            embedder,
            config,
            template,
        }
    }

    /// The typed refusal for a state with no images.
    fn no_images_failure(&self) -> EngineError {
        EngineError::ClassifierFailed {
            model_id: self.model_id.clone(),
            reason: "state carries no images; the vision readout decides over \
                     images and never falls back to text-only evidence"
                .to_owned(),
        }
    }

    /// The typed failure for an embedder that answered fewer vectors
    /// than requests: never indexed blindly, never guessed around.
    fn short_return_failure(&self, what: &str) -> EngineError {
        EngineError::ClassifierFailed {
            model_id: self.model_id.clone(),
            reason: format!("embedder returned fewer {what} embeddings than requested"),
        }
    }

    fn embed_failure(&self, error: &ModelError) -> EngineError {
        EngineError::ClassifierFailed { model_id: self.model_id.clone(), reason: error.to_string() }
    }

    /// The one pooled image embedding for `state`, or the typed
    /// no-images refusal.
    fn image_embedding(&self, state: &State) -> EngineResult<Vec<f32>> {
        let images = state.images();
        if images.is_empty() {
            return Err(self.no_images_failure());
        }
        let vectors =
            self.embedder.embed_images(images).map_err(|error| self.embed_failure(&error))?;
        if vectors.len() < images.len() {
            return Err(self.short_return_failure("image"));
        }
        pooled_image(&vectors).ok_or_else(|| self.no_images_failure())
    }

    /// The subject text one choice answer renders into: the
    /// candidate's description when it has one, else its id (the
    /// embedding rung's convention).
    fn choice_subject(candidate: &opencodifier_core::Candidate) -> String {
        if candidate.description().is_empty() {
            candidate.id().as_str().to_owned()
        } else {
            candidate.description().to_owned()
        }
    }

    fn decide_choice(
        &self,
        state: &State,
        question: &ChoiceQuestion,
    ) -> EngineResult<Distribution> {
        let image = self.image_embedding(state)?;
        let prompts: Vec<String> = question
            .candidates()
            .iter()
            .map(|candidate| {
                self.template.prompt(question.text(), &Self::choice_subject(candidate))
            })
            .collect();
        let scores = self.score_prompts(&image, &prompts)?;
        self.distribution_over(
            question.candidates().iter().map(|candidate| candidate.id().as_str().to_owned()),
            &scores,
        )
    }

    fn decide_boolean(
        &self,
        state: &State,
        question: &BooleanQuestion,
    ) -> EngineResult<Distribution> {
        let image = self.image_embedding(state)?;
        // The evidence prompt is the question text itself, polarity
        // flipped by its negators — the embedding rung's readout, over
        // the image embedding instead of a text query embedding.
        let vectors = self
            .embedder
            .embed_texts(&[question.text()])
            .map_err(|error| self.embed_failure(&error))?;
        let Some(text) = vectors.first() else {
            return Err(self.short_return_failure("text"));
        };
        let text = normalized(text);
        let dot: f32 = image.iter().zip(text.iter()).map(|(x, y)| x * y).sum();
        let evidence =
            f64::from(dot) * self.config.logit_scale * negation_polarity(question.text());
        let probabilities = softmax(&[evidence, 0.0]);
        // Defensive-unreachable, as in the embedding rung: two distinct
        // literal keys over normalized softmax output.
        Distribution::from_pairs([("true", probabilities[0]), ("false", probabilities[1])]).map_err(
            |error| EngineError::ClassifierFailed {
                model_id: self.model_id.clone(),
                reason: error.to_string(),
            },
        )
    }

    fn decide_score(&self, state: &State, question: &ScoreQuestion) -> EngineResult<Distribution> {
        let image = self.image_embedding(state)?;
        let prompts: Vec<String> = question
            .levels()
            .iter()
            .map(|level| self.template.prompt(question.text(), level.label()))
            .collect();
        let scores = self.score_prompts(&image, &prompts)?;
        self.distribution_over(
            question.levels().iter().map(|level| level.label().to_owned()),
            &scores,
        )
    }

    /// Embeds `prompts` and applies the zero-shot head.
    fn score_prompts(&self, image: &[f32], prompts: &[String]) -> EngineResult<Vec<f64>> {
        let borrowed: Vec<&str> = prompts.iter().map(String::as_str).collect();
        let vectors =
            self.embedder.embed_texts(&borrowed).map_err(|error| self.embed_failure(&error))?;
        if vectors.len() < prompts.len() {
            return Err(self.short_return_failure("text"));
        }
        let refs: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
        Ok(zero_shot_scores(image, &refs, self.config.logit_scale))
    }

    /// Normalizes head scores into the distribution over `keys`.
    fn distribution_over(
        &self,
        keys: impl Iterator<Item = String>,
        scores: &[f64],
    ) -> EngineResult<Distribution> {
        let probabilities = softmax(scores);
        // Defensive-unreachable: keys are unique and non-empty by
        // construction and softmax output is finite and normalized.
        Distribution::from_pairs(keys.zip(probabilities)).map_err(|error| {
            EngineError::ClassifierFailed {
                model_id: self.model_id.clone(),
                reason: error.to_string(),
            }
        })
    }
}

impl Classifier for VisionClassifier {
    fn decide(&self, state: &State, question: &DecisionQuestion) -> EngineResult<Distribution> {
        match question {
            DecisionQuestion::Choice(question) => self.decide_choice(state, question),
            DecisionQuestion::Boolean(question) => self.decide_boolean(state, question),
            DecisionQuestion::Score(question) => self.decide_score(state, question),
            // `DecisionQuestion` is `#[non_exhaustive]`: a future kind
            // has no image readout until it is designed one, so the
            // honest answer is a typed failure, not a guess.
            _ => Err(EngineError::ClassifierFailed {
                model_id: self.model_id.clone(),
                reason: "no image readout for this question kind".to_owned(),
            }),
        }
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// The serving transport: preprocessing, encoding, and the ONNX
/// split-tower graphs behind the [`ImageTextEmbedder`] seam. Compiled
/// only under the `vision` feature (phase G).
#[cfg(feature = "vision")]
pub mod serve;

/// A deterministic [`ImageTextEmbedder`] with no model file.
///
/// A test double, not a simulated model: it makes **no claim** about
/// semantic quality. Images embed as an L2-normalized bag of byte
/// chunks hashed into `dims` buckets with FNV-1a (the mock text
/// backend's fixed hash); texts embed as the same hash over the same
/// tokenizer-free word buckets `MockEmbeddingBackend` uses, so the two
/// sides at least share a bucket space. Identical inputs always
/// produce identical outputs on every platform.
#[derive(Debug, Clone)]
pub struct MockImageTextEmbedder {
    model_id: String,
    dims: usize,
}

impl MockImageTextEmbedder {
    /// Creates a mock dual embedder with `dims` buckets (1..=4096).
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] outside the supported range —
    /// the mock exists for narrow test spaces, not to imitate a large
    /// embedding model.
    pub fn new(model_id: impl Into<String>, dims: usize) -> Result<Self, ModelError> {
        if dims == 0 || dims > 4096 {
            return Err(ModelError::ContractViolation {
                reason: format!("mock embedding dims must be in 1..=4096, got {dims}"),
            });
        }
        Ok(Self { model_id: model_id.into(), dims })
    }

    /// FNV-1a over bytes; stable across platforms and processes.
    fn fnv1a(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// The L2-normalized bucket bag over arbitrary byte chunks.
    fn bag(&self, chunks: impl Iterator<Item = Vec<u8>>) -> Vec<f32> {
        let mut vector = vec![0.0_f32; self.dims];
        for chunk in chunks {
            if chunk.is_empty() {
                continue;
            }
            // `dims` is capped at 4096, so this narrowing cannot
            // truncate on any supported target.
            #[allow(clippy::cast_possible_truncation)]
            let slot = (Self::fnv1a(&chunk) % self.dims as u64) as usize;
            vector[slot] += 1.0;
        }
        normalized(&vector)
    }
}

impl ImageTextEmbedder for MockImageTextEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dims(&self) -> usize {
        self.dims
    }

    fn embed_images(&self, images: &[ImageInput]) -> Result<Vec<Vec<f32>>, ModelError> {
        Ok(images
            .iter()
            .map(|image| {
                // Hash the media type and payload in 8-byte chunks: the
                // same bytes embed identically wherever they appear.
                let mut chunks = Vec::new();
                chunks.extend(image.media_type().as_str().as_bytes().chunks(8).map(<[u8]>::to_vec));
                chunks.extend(image.data().chunks(8).map(<[u8]>::to_vec));
                self.bag(chunks.into_iter())
            })
            .collect())
    }

    fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError> {
        Ok(texts
            .iter()
            .map(|text| {
                let chunks = text
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .filter(|token| !token.is_empty())
                    .map(|token| token.to_ascii_lowercase().into_bytes());
                self.bag(chunks)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{Candidate, ImageMediaType, ScoreLevel};
    use std::collections::BTreeMap;

    fn png_image(bytes: &[u8]) -> ImageInput {
        ImageInput::new(ImageMediaType::Png, bytes.to_vec())
    }

    fn image_state(text: &str, images: &[ImageInput]) -> State {
        let mut state = State::from_text(text);
        for image in images {
            state = state.with_image(image.clone());
        }
        state
    }

    #[test]
    fn head_scores_are_pinned_for_aligned_orthogonal_and_opposite_texts() {
        // The image sits on e0; the three texts are aligned with it,
        // orthogonal to it, and opposite to it: the expected
        // probabilities are softmax([10, 0, -10]).
        let image = [1.0, 0.0, 0.0, 0.0];
        let texts: Vec<&[f32]> =
            vec![&[1.0, 0.0, 0.0, 0.0], &[0.0, 1.0, 0.0, 0.0], &[-1.0, 0.0, 0.0, 0.0]];
        let scores = zero_shot_scores(&image, &texts, 10.0);
        let probabilities = softmax(&scores);
        let e10 = f64::exp(10.0);
        let e0 = 1.0;
        let neg10 = f64::exp(-10.0);
        let total = e10 + e0 + neg10;
        assert!((probabilities[0] - e10 / total).abs() < 1e-12, "{probabilities:?}");
        assert!((probabilities[1] - e0 / total).abs() < 1e-12, "{probabilities:?}");
        assert!((probabilities[2] - neg10 / total).abs() < 1e-12, "{probabilities:?}");
        assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn head_normalizes_defensively_and_treats_zero_vectors_as_no_evidence() {
        // An unnormalized text vector must not change the outcome.
        let image = [3.0, 0.0];
        let scaled = zero_shot_scores(&image, &[&[9.0, 0.0], &[0.0, 5.0]], 4.0);
        let unit = zero_shot_scores(&image, &[&[1.0, 0.0], &[0.0, 1.0]], 4.0);
        assert_eq!(scaled, unit, "scaling an input must not move a normalized score");

        // A zero-norm side is no evidence: every score is exactly 0.0,
        // so the softmax is uniform — never NaN.
        let blind = zero_shot_scores(&[0.0, 0.0], &[&[1.0, 0.0], &[0.0, 1.0]], 4.0);
        assert!(blind.iter().all(|score| *score == 0.0), "{blind:?}");
        let uniform = softmax(&blind);
        assert!(uniform.iter().all(|probability| (probability - 0.5).abs() < 1e-12));
    }

    #[test]
    fn logit_scale_is_validated() {
        assert_eq!(Siglip2Config::new().logit_scale, DEFAULT_LOGIT_SCALE);
        assert_eq!(Siglip2Config::default().logit_scale, DEFAULT_LOGIT_SCALE);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = Siglip2Config::new().with_logit_scale(bad).unwrap_err();
            assert_eq!(error.code(), "model.contract_violation", "scale {bad}");
        }
        assert_eq!(Siglip2Config::new().with_logit_scale(57.5).unwrap().logit_scale, 57.5);
    }

    #[test]
    fn templates_render_and_round_trip_through_tags() {
        assert_eq!(
            PromptTemplate::QuestionSubject.prompt("How hard?", "expert"),
            "How hard?\nexpert"
        );
        assert_eq!(PromptTemplate::PhotoOf.prompt("How hard?", "expert"), "a photo of expert.");
        assert_eq!(PromptTemplate::default(), PromptTemplate::QuestionSubject);
        assert_eq!(PromptTemplate::QuestionSubject.tag(), "question-subject");
        assert_eq!(PromptTemplate::PhotoOf.tag(), "photo-of");
        for template in [PromptTemplate::QuestionSubject, PromptTemplate::PhotoOf] {
            assert_eq!(PromptTemplate::from_tag(template.tag()).unwrap(), template);
        }
        let error = PromptTemplate::from_tag("nope").unwrap_err();
        assert_eq!(error.code(), "model.contract_violation");
        assert!(error.to_string().contains("nope"), "{error}");
    }

    #[test]
    fn mock_embedder_is_deterministic_normalized_and_ordered() {
        let embedder = MockImageTextEmbedder::new("mock-vision-v1", 64).unwrap();
        assert_eq!(embedder.model_id(), "mock-vision-v1");
        assert_eq!(embedder.dims(), 64);

        let images = [png_image(b"chart bytes"), png_image(b"chart bytes"), png_image(b"other")];
        let vectors = embedder.embed_images(&images).unwrap();
        assert_eq!(vectors.len(), images.len());
        assert_eq!(vectors[0], vectors[1], "identical images embed identically");
        assert_ne!(vectors[0], vectors[2], "different images differ");
        let norm: f32 = vectors[0].iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "images must embed to unit length, got {norm}");

        let texts = ["route this request", "route this request", "different words entirely"];
        let text_vectors = embedder.embed_texts(&texts).unwrap();
        assert_eq!(text_vectors[0], text_vectors[1]);
        assert_ne!(text_vectors[0], text_vectors[2]);

        // Repeatability across separate instances.
        let again = MockImageTextEmbedder::new("mock-vision-v1", 64).unwrap();
        assert_eq!(again.embed_images(&images[..1]).unwrap()[0], vectors[0]);
    }

    #[test]
    fn mock_embedder_rejects_out_of_range_dims() {
        for dims in [0, 4097] {
            let error = MockImageTextEmbedder::new("m", dims).unwrap_err();
            assert_eq!(error.code(), "model.contract_violation", "dims {dims}");
            assert!(error.to_string().contains("1..=4096"), "{error}");
        }
        assert!(MockImageTextEmbedder::new("m", 1).is_ok());
        assert!(MockImageTextEmbedder::new("m", 4096).is_ok());
    }

    /// A scripted embedder: one fixed image answer per request and
    /// fixed vectors for known prompts, so readout semantics can be
    /// asserted exactly.
    #[derive(Debug)]
    struct Scripted {
        image: Vec<f32>,
        texts: BTreeMap<String, Vec<f32>>,
        fail_images: bool,
    }

    impl ImageTextEmbedder for Scripted {
        fn model_id(&self) -> &'static str {
            "scripted-vision"
        }

        fn dims(&self) -> usize {
            4
        }

        fn embed_images(&self, images: &[ImageInput]) -> Result<Vec<Vec<f32>>, ModelError> {
            if self.fail_images {
                return Err(ModelError::ContractViolation { reason: "session closed".to_owned() });
            }
            Ok(vec![self.image.clone(); images.len()])
        }

        fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError> {
            Ok(texts
                .iter()
                .map(|text| self.texts.get(*text).cloned().unwrap_or_else(|| vec![0.0; 4]))
                .collect())
        }
    }

    fn choice_question() -> ChoiceQuestion {
        ChoiceQuestion::new(
            "chart",
            "Which chart is shown?",
            vec![
                Candidate::new("revenue", "revenue over time").unwrap(),
                Candidate::new("headcount", "headcount by team").unwrap(),
            ],
        )
        .unwrap()
    }

    fn scripted(texts: BTreeMap<String, Vec<f32>>) -> VisionClassifier {
        VisionClassifier::new(
            Arc::new(Scripted { image: vec![1.0, 0.0, 0.0, 0.0], texts, fail_images: false }),
            Siglip2Config::new().with_logit_scale(10.0).unwrap(),
            PromptTemplate::QuestionSubject,
        )
    }

    #[test]
    fn choice_readout_ranks_the_aligned_candidate_first() {
        let classifier = scripted(BTreeMap::from([
            ("Which chart is shown?\nrevenue over time".to_owned(), vec![1.0, 0.0, 0.0, 0.0]),
            ("Which chart is shown?\nheadcount by team".to_owned(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));
        assert_eq!(
            classifier.model_id(),
            "scripted-vision:siglip2-zero-shot:question-subject",
            "the model id folds the embedder and the template (D6)"
        );

        let state = image_state("quarterly numbers", &[png_image(b"chart")]);
        let distribution =
            classifier.decide(&state, &DecisionQuestion::Choice(choice_question())).unwrap();
        assert!(distribution.probability_of("revenue").unwrap() > 0.99, "{distribution:?}");
        assert!(
            (distribution.probability_of("revenue").unwrap()
                + distribution.probability_of("headcount").unwrap()
                - 1.0)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn empty_candidate_descriptions_fall_back_to_the_id() {
        let question = ChoiceQuestion::new(
            "chart",
            "Which chart is shown?",
            vec![Candidate::new("revenue", "").unwrap(), Candidate::new("headcount", "").unwrap()],
        )
        .unwrap();
        // The id is the subject, so the same scripted space aligns.
        let classifier = scripted(BTreeMap::from([
            ("Which chart is shown?\nrevenue".to_owned(), vec![1.0, 0.0, 0.0, 0.0]),
            ("Which chart is shown?\nheadcount".to_owned(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));
        let state = image_state("s", &[png_image(b"chart")]);
        let distribution = classifier.decide(&state, &DecisionQuestion::Choice(question)).unwrap();
        assert!(distribution.probability_of("revenue").unwrap() > 0.99, "{distribution:?}");
    }

    #[test]
    fn boolean_readout_affirms_on_support_and_flips_on_negation() {
        // Both prompts share one aligned text vector; only the
        // negator in the question text flips the evidence sign.
        let aligned = vec![1.0, 0.0, 0.0, 0.0];
        let classifier = scripted(BTreeMap::from([
            ("the chart shows growth".to_owned(), aligned.clone()),
            ("the chart does not show growth".to_owned(), aligned),
        ]));
        let state = image_state("s", &[png_image(b"chart")]);

        let supported = BooleanQuestion::new("growing", "the chart shows growth").unwrap();
        let distribution =
            classifier.decide(&state, &DecisionQuestion::Boolean(supported)).unwrap();
        assert!(distribution.probability_of("true").unwrap() > 0.99, "{distribution:?}");

        let negated = BooleanQuestion::new("growing", "the chart does not show growth").unwrap();
        let flipped = classifier.decide(&state, &DecisionQuestion::Boolean(negated)).unwrap();
        assert!(flipped.probability_of("false").unwrap() > 0.99, "{flipped:?}");
    }

    #[test]
    fn score_readout_prefers_the_matching_level() {
        let question = ScoreQuestion::new(
            "trend",
            "How is the trend shaped?",
            ["flat", "rising"].iter().map(|label| ScoreLevel::new(*label).unwrap()).collect(),
        )
        .unwrap();
        let classifier = scripted(BTreeMap::from([
            ("How is the trend shaped?\nflat".to_owned(), vec![0.0, 1.0, 0.0, 0.0]),
            ("How is the trend shaped?\nrising".to_owned(), vec![1.0, 0.0, 0.0, 0.0]),
        ]));
        let state = image_state("s", &[png_image(b"chart")]);
        let distribution = classifier.decide(&state, &DecisionQuestion::Score(question)).unwrap();
        assert!(distribution.probability_of("rising").unwrap() > 0.99, "{distribution:?}");
    }

    #[test]
    fn multiple_images_pool_into_one_piece_of_evidence() {
        // Two copies of the same image must decide exactly as one.
        let one = image_state("s", &[png_image(b"chart")]);
        let two = image_state("s", &[png_image(b"chart"), png_image(b"chart")]);
        let question = DecisionQuestion::Choice(choice_question());
        let expected = scripted(BTreeMap::from([
            ("Which chart is shown?\nrevenue over time".to_owned(), vec![1.0, 0.0, 0.0, 0.0]),
            ("Which chart is shown?\nheadcount by team".to_owned(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));
        assert_eq!(
            expected.decide(&one, &question).unwrap(),
            expected.decide(&two, &question).unwrap()
        );
    }

    #[test]
    fn a_text_only_state_is_a_typed_refusal() {
        let classifier = scripted(BTreeMap::new());
        let error = classifier
            .decide(
                &State::from_text("no images here"),
                &DecisionQuestion::Choice(choice_question()),
            )
            .unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("no images"), "{error}");
    }

    #[test]
    fn embedder_failures_surface_as_classifier_failed() {
        let classifier = VisionClassifier::new(
            Arc::new(Scripted { image: Vec::new(), texts: BTreeMap::new(), fail_images: true }),
            Siglip2Config::new(),
            PromptTemplate::QuestionSubject,
        );
        let state = image_state("s", &[png_image(b"chart")]);
        let error =
            classifier.decide(&state, &DecisionQuestion::Choice(choice_question())).unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("session closed"), "{error}");
    }

    #[test]
    fn an_embedder_returning_no_embeddings_is_a_typed_failure() {
        #[derive(Debug)]
        struct Empty;
        impl ImageTextEmbedder for Empty {
            fn model_id(&self) -> &'static str {
                "empty-vision"
            }
            fn dims(&self) -> usize {
                4
            }
            fn embed_images(&self, _images: &[ImageInput]) -> Result<Vec<Vec<f32>>, ModelError> {
                Ok(Vec::new())
            }
            fn embed_texts(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, ModelError> {
                Ok(Vec::new())
            }
        }
        let classifier = VisionClassifier::new(
            Arc::new(Empty),
            Siglip2Config::new(),
            PromptTemplate::QuestionSubject,
        );
        let state = image_state("s", &[png_image(b"chart")]);
        let error =
            classifier.decide(&state, &DecisionQuestion::Choice(choice_question())).unwrap_err();
        assert!(error.to_string().contains("fewer image embeddings"), "{error}");
    }

    #[test]
    fn runs_through_a_real_decision_engine_end_to_end() {
        // The vision classifier as a fallback rung: the default pipeline's
        // graph fires the image-capable rung when the request carries
        // images. Construct the engine with the vision classifier wired
        // as the only fallback (modality Image, so it fires on images
        // and is skipped on text-only requests).
        let config = opencodifier_engine::EngineConfig::with_default_pipeline().unwrap();
        let classifier = scripted(BTreeMap::from([
            ("Which chart is shown?\nrevenue over time".to_owned(), vec![1.0, 0.0, 0.0, 0.0]),
            ("Which chart is shown?\nheadcount by team".to_owned(), vec![0.0, 1.0, 0.0, 0.0]),
        ]));
        let vision_rung = opencodifier_engine::ladder::Rung::new(Arc::new(classifier))
            .with_modality(opencodifier_engine::ladder::RungModality::Image);
        let engine = opencodifier_engine::DecisionEngine::new_with_rungs(
            config,
            Arc::new(opencodifier_engine::SystemClock),
            Arc::new(opencodifier_engine::LexicalClassifier::new()),
            None,
            vec![vision_rung],
        )
        .unwrap();

        let request = opencodifier_core::DecisionRequest::new(
            image_state("quarterly numbers", &[png_image(b"chart")]),
            vec![DecisionQuestion::Choice(choice_question())],
            opencodifier_core::DecisionPolicy::default(),
            opencodifier_core::RequestMetadata::default(),
        )
        .unwrap();
        let response = engine.decide(&request).unwrap();
        assert_eq!(response.answers().len(), 1);
        let trace = response.trace();
        assert!(
            trace.entries().iter().any(|entry| entry.node == "choice"),
            "the choice node must have run: {trace:?}"
        );
    }
}
