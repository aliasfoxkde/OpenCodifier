//! Embedding-backed semantic classification — the layer above lexical
//! scoring in the escalation ladder (PLANNING.md §13, DECISIONS.md D4).
//!
//! [`EmbeddingClassifier`] turns any [`EmbeddingBackend`] into an engine
//! [`Classifier`]: it embeds the state text and the question's answer
//! space once per call, scores cosine similarities, and normalizes them
//! with the same stable softmax the lexical layer uses. The *algorithm*
//! is fully deterministic and ours; the *quality* is exactly the quality
//! of the plugged-in backend — a hash mock gives hash-level semantics,
//! a real encoder gives real semantics.
//!
//! This is honestly the weakest useful semantic layer: no cross-encoder,
//! no candidate conditioning, no calibration (the engine computes
//! confidence downstream and the gate decides what to trust). It exists
//! so the pipeline's "embedding similarity" rung is real code today, not
//! a promise.

use opencodifier_core::{
    BooleanQuestion, ChoiceQuestion, DecisionQuestion, Distribution, ScoreQuestion, State,
};
use opencodifier_engine::{Classifier, EngineError, EngineResult, negation_polarity, softmax};
use opencodifier_runtime::EmbeddingBackend;

/// A [`Classifier`] that decides by embedding similarity.
#[derive(Debug)]
pub struct EmbeddingClassifier {
    backend: Box<dyn EmbeddingBackend>,
    temperature: f64,
}

impl EmbeddingClassifier {
    /// Wraps a backend; softmax temperature defaults to `1.0`.
    pub fn new(backend: Box<dyn EmbeddingBackend>) -> Self {
        Self { backend, temperature: 1.0 }
    }

    /// Sharpens (`< 1`) or flattens (`> 1`) the similarity softmax.
    ///
    /// The temperature is a scoring-shape choice, **not** calibration:
    /// raw softmax output is never treated as calibrated confidence
    /// (PLANNING.md §24).
    pub fn with_temperature(mut self, temperature: f64) -> EngineResult<Self> {
        if !temperature.is_finite() || temperature <= 0.0 {
            return Err(EngineError::InvalidConfig {
                reason: format!("temperature must be finite and positive, got {temperature}"),
            });
        }
        self.temperature = temperature;
        Ok(self)
    }

    /// Cosine similarity, with the zero-vector convention: a vector with
    /// no norm (empty text under the mock, or a degenerate encoder output)
    /// is *no evidence*, so the similarity is `0.0`, never `NaN`.
    fn cosine(a: &[f32], b: &[f32]) -> f64 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|y| y * y).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { f64::from(dot / (norm_a * norm_b)) }
    }

    /// The typed failure for a backend that returned fewer embeddings
    /// than texts: never indexed blindly, never guessed around.
    fn no_embeddings_failure(&self) -> EngineError {
        EngineError::ClassifierFailed {
            model_id: self.backend.model_id().to_owned(),
            reason: "backend returned fewer embeddings than texts".to_owned(),
        }
    }

    /// Embeds all texts in one backend call, preserving order.
    fn embed_all(&self, texts: &[&str]) -> EngineResult<Vec<Vec<f32>>> {
        self.backend.embed(texts).map_err(|error| EngineError::ClassifierFailed {
            model_id: self.backend.model_id().to_owned(),
            reason: error.to_string(),
        })
    }

    fn decide_choice(
        &self,
        state: &State,
        question: &ChoiceQuestion,
    ) -> EngineResult<Distribution> {
        let query = format!("{} \n {}", state.text(), question.text());
        let mut texts = vec![query];
        texts.extend(question.candidates().iter().map(|candidate| {
            if candidate.description().is_empty() {
                candidate.id().as_str().to_owned()
            } else {
                candidate.description().to_owned()
            }
        }));
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        let vectors = self.embed_all(&borrowed)?;
        let Some((query_vector, candidate_vectors)) = vectors.split_first() else {
            return Err(self.no_embeddings_failure());
        };

        let scores: Vec<f64> = candidate_vectors
            .iter()
            .map(|vector| Self::cosine(query_vector, vector) / self.temperature)
            .collect();
        let probabilities = softmax(&scores);
        let pairs: Vec<(String, f64)> = question
            .candidates()
            .iter()
            .map(|candidate| candidate.id().as_str().to_owned())
            .zip(probabilities)
            .collect();
        // Defensive-unreachable: softmax output is finite and normalized,
        // candidate ids are unique and non-empty, so `from_pairs` cannot
        // fail here. Kept typed so a future change fails loudly instead
        // of fabricating a distribution.
        Distribution::from_pairs(pairs).map_err(|error| EngineError::ClassifierFailed {
            model_id: self.backend.model_id().to_owned(),
            reason: error.to_string(),
        })
    }

    fn decide_boolean(
        &self,
        state: &State,
        question: &BooleanQuestion,
    ) -> EngineResult<Distribution> {
        let vectors = self.embed_all(&[state.text(), question.text()])?;
        let [state_vector, question_vector, ..] = vectors.as_slice() else {
            return Err(self.no_embeddings_failure());
        };
        let evidence =
            Self::cosine(state_vector, question_vector) * negation_polarity(question.text());
        let probabilities = softmax(&[evidence / self.temperature, 0.0]);
        // Defensive-unreachable, as in `decide_choice`: two distinct
        // literal keys over normalized softmax output.
        Distribution::from_pairs([("true", probabilities[0]), ("false", probabilities[1])]).map_err(
            |error| EngineError::ClassifierFailed {
                model_id: self.backend.model_id().to_owned(),
                reason: error.to_string(),
            },
        )
    }

    fn decide_score(&self, state: &State, question: &ScoreQuestion) -> EngineResult<Distribution> {
        let mut texts = vec![state.text().to_owned()];
        texts.extend(question.levels().iter().map(|level| level.label().to_owned()));
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        let vectors = self.embed_all(&borrowed)?;
        let Some((state_vector, level_vectors)) = vectors.split_first() else {
            return Err(self.no_embeddings_failure());
        };

        let scores: Vec<f64> = level_vectors
            .iter()
            .map(|vector| Self::cosine(state_vector, vector) / self.temperature)
            .collect();
        let probabilities = softmax(&scores);
        let pairs: Vec<(String, f64)> = question
            .levels()
            .iter()
            .map(|level| level.label().to_owned())
            .zip(probabilities)
            .collect();
        // Defensive-unreachable, as in `decide_choice`.
        Distribution::from_pairs(pairs).map_err(|error| EngineError::ClassifierFailed {
            model_id: self.backend.model_id().to_owned(),
            reason: error.to_string(),
        })
    }
}

impl Classifier for EmbeddingClassifier {
    fn decide(&self, state: &State, question: &DecisionQuestion) -> EngineResult<Distribution> {
        match question {
            DecisionQuestion::Choice(question) => self.decide_choice(state, question),
            DecisionQuestion::Boolean(question) => self.decide_boolean(state, question),
            DecisionQuestion::Score(question) => self.decide_score(state, question),
            // `DecisionQuestion` is `#[non_exhaustive]`: a future question
            // kind has no embedding representation until it is designed
            // one, so the honest answer is a typed failure, not a guess.
            _ => Err(EngineError::ClassifierFailed {
                model_id: self.backend.model_id().to_owned(),
                reason: "no embedding representation for this question kind".to_owned(),
            }),
        }
    }

    fn model_id(&self) -> &str {
        self.backend.model_id()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, ScoreLevel, ScoreQuestion,
    };
    use opencodifier_runtime::MockEmbeddingBackend;
    use std::sync::Arc;

    fn backend() -> Box<dyn EmbeddingBackend> {
        Box::new(MockEmbeddingBackend::new("mock-embed-v1", 128).unwrap())
    }

    fn choice_question() -> ChoiceQuestion {
        ChoiceQuestion::new(
            "model",
            "Which model should run deep reasoning work?",
            vec![
                Candidate::new("local-qwen", "fast general coding").unwrap(),
                Candidate::new("local-glm", "deep reasoning specialist").unwrap(),
            ],
        )
        .unwrap()
    }

    fn state(text: &str) -> State {
        State::from_text(text)
    }

    #[test]
    fn choice_similarity_ranks_the_matching_candidate_first() {
        let classifier = EmbeddingClassifier::new(backend());
        assert_eq!(classifier.model_id(), "mock-embed-v1");

        let distribution = classifier
            .decide(
                &state("we need deep reasoning for this proof"),
                &DecisionQuestion::Choice(choice_question()),
            )
            .unwrap();
        assert!(distribution.probability_of("local-glm").unwrap() > 0.5, "{distribution:?}");
        assert!(distribution.probability_of("local-qwen").unwrap() < 0.5);
        assert!(
            (distribution.probability_of("local-glm").unwrap()
                + distribution.probability_of("local-qwen").unwrap()
                - 1.0)
                .abs()
                < 1e-9,
            "distribution must be normalized"
        );
    }

    #[test]
    fn empty_candidate_descriptions_fall_back_to_the_id() {
        let question = ChoiceQuestion::new(
            "model",
            "Which model?",
            vec![Candidate::new("alpha", "").unwrap(), Candidate::new("beta", "").unwrap()],
        )
        .unwrap();
        let classifier = EmbeddingClassifier::new(backend());
        let distribution =
            classifier.decide(&state("pick alpha"), &DecisionQuestion::Choice(question)).unwrap();
        // The id participates as the embedded text, so "alpha" is scored
        // against itself and wins over the unrelated "beta".
        assert!(distribution.probability_of("alpha").unwrap() > 0.5, "{distribution:?}");
    }

    #[test]
    fn boolean_affirms_on_support_and_flips_on_negation() {
        let classifier = EmbeddingClassifier::new(backend());
        let supported =
            BooleanQuestion::new("needs_reasoning", "deep reasoning required here").unwrap();
        let distribution = classifier
            .decide(
                &state("this task needs deep reasoning required here"),
                &DecisionQuestion::Boolean(supported),
            )
            .unwrap();
        assert!(distribution.probability_of("true").unwrap() > 0.5, "{distribution:?}");

        let negated =
            BooleanQuestion::new("needs_reasoning", "deep reasoning is not required").unwrap();
        let flipped = classifier
            .decide(&state("deep reasoning is not required"), &DecisionQuestion::Boolean(negated))
            .unwrap();
        assert!(flipped.probability_of("false").unwrap() > 0.5, "{flipped:?}");
    }

    #[test]
    fn score_similarity_prefers_the_matching_level() {
        let question = ScoreQuestion::new(
            "difficulty",
            "How hard is this?",
            ["trivial", "moderate", "expert"]
                .iter()
                .map(|label| ScoreLevel::new(*label).unwrap())
                .collect(),
        )
        .unwrap();
        let classifier = EmbeddingClassifier::new(backend());
        let distribution = classifier
            .decide(&state("an expert level task"), &DecisionQuestion::Score(question))
            .unwrap();
        assert!(distribution.probability_of("expert").unwrap() > 0.4, "{distribution:?}");
    }

    #[test]
    fn temperature_sharpens_and_validates() {
        let query = "deep reasoning";
        let question = DecisionQuestion::Choice(choice_question());
        let sharp = EmbeddingClassifier::new(backend())
            .with_temperature(0.1)
            .unwrap()
            .decide(&state(query), &question)
            .unwrap();
        let flat = EmbeddingClassifier::new(backend())
            .with_temperature(10.0)
            .unwrap()
            .decide(&state(query), &question)
            .unwrap();
        assert!(
            sharp.probability_of("local-glm").unwrap() > flat.probability_of("local-glm").unwrap()
        );

        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = EmbeddingClassifier::new(backend()).with_temperature(bad).unwrap_err();
            assert_eq!(error.code(), "engine.invalid_config", "temperature {bad}");
        }
    }

    #[test]
    fn backend_failures_surface_as_classifier_failed() {
        #[derive(Debug)]
        struct Failing;
        impl EmbeddingBackend for Failing {
            fn model_id(&self) -> &'static str {
                "failing-embed"
            }
            fn dims(&self) -> usize {
                8
            }
            fn embed(
                &self,
                _texts: &[&str],
            ) -> Result<Vec<Vec<f32>>, opencodifier_runtime::RuntimeError> {
                Err(opencodifier_runtime::RuntimeError::BackendFailed {
                    model_id: "failing-embed".into(),
                    message: "session closed".into(),
                })
            }
        }
        let classifier = EmbeddingClassifier::new(Box::new(Failing));
        assert_eq!(EmbeddingBackend::dims(&Failing), 8, "dims is part of the trait contract");
        let error = classifier
            .decide(&state("s"), &DecisionQuestion::Choice(choice_question()))
            .unwrap_err();
        assert_eq!(error.code(), "engine.classifier_failed");
        assert!(error.to_string().contains("session closed"), "{error}");
    }

    #[test]
    fn a_backend_returning_no_embeddings_is_a_typed_failure() {
        // A backend that answers fewer embeddings than texts is broken;
        // the classifier must refuse every question kind rather than
        // index blindly or guess around the gap.
        #[derive(Debug)]
        struct Empty;
        impl EmbeddingBackend for Empty {
            fn model_id(&self) -> &'static str {
                "empty-embed"
            }
            fn dims(&self) -> usize {
                4
            }
            fn embed(
                &self,
                texts: &[&str],
            ) -> Result<Vec<Vec<f32>>, opencodifier_runtime::RuntimeError> {
                debug_assert!(texts.len() > 1, "drives the split_first failure arms");
                Ok(Vec::new())
            }
        }
        let classifier = EmbeddingClassifier::new(Box::new(Empty));
        assert_eq!(EmbeddingBackend::dims(&Empty), 4, "dims is part of the trait contract");

        let choice = classifier
            .decide(&state("s"), &DecisionQuestion::Choice(choice_question()))
            .unwrap_err();
        assert!(choice.to_string().contains("fewer embeddings"), "{choice}");

        let boolean = classifier
            .decide(
                &state("s"),
                &DecisionQuestion::Boolean(BooleanQuestion::new("gate", "is this on?").unwrap()),
            )
            .unwrap_err();
        assert_eq!(boolean.code(), "engine.classifier_failed");

        let score = classifier
            .decide(
                &state("s"),
                &DecisionQuestion::Score(
                    ScoreQuestion::new(
                        "level",
                        "how bad?",
                        ["low", "high"]
                            .iter()
                            .map(|label| ScoreLevel::new(*label).unwrap())
                            .collect(),
                    )
                    .unwrap(),
                ),
            )
            .unwrap_err();
        assert!(score.to_string().contains("empty-embed"), "{score}");
    }

    #[test]
    fn runs_through_a_real_decision_engine_end_to_end() {
        let config = opencodifier_engine::EngineConfig::with_default_pipeline().unwrap();
        let engine = opencodifier_engine::DecisionEngine::new(
            config,
            Arc::new(opencodifier_engine::SystemClock),
            Arc::new(EmbeddingClassifier::new(backend())),
            None,
        )
        .unwrap();

        let request = opencodifier_core::DecisionRequest::new(
            state("route this: deep reasoning over a large proof"),
            vec![DecisionQuestion::Choice(choice_question())],
            opencodifier_core::DecisionPolicy::default(),
            opencodifier_core::RequestMetadata::default(),
        )
        .unwrap();
        let response = engine.decide(&request).unwrap();
        assert_eq!(response.answers().len(), 1);
        let trace = response.trace();
        let ran_choice = trace.entries().iter().any(|entry| entry.node == "choice");
        assert!(ran_choice, "the choice node must have run: {trace:?}");
    }
}
