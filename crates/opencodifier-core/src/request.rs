//! The canonical request envelope: state + questions + policy (PLANNING.md §6).

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::policy::{DecisionPolicy, RequestMetadata};
use crate::question::DecisionQuestion;
use crate::state::State;

/// One decision request in canonical form.
///
/// Everything — native calls, `OpenAI` structured outputs, Anthropic tool
/// schemas, Jev requests — normalizes into this type before the engine
/// sees it. Vendor formats are adapters; this is the internal truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawDecisionRequest")]
pub struct DecisionRequest {
    state: State,
    questions: Vec<DecisionQuestion>,
    policy: DecisionPolicy,
    metadata: RequestMetadata,
}

/// Deserialization mirror for [`DecisionRequest`]; conversion validates.
///
/// Each field type validates itself on deserialize; [`DecisionRequest::new`]
/// additionally enforces the cross-field resource limits.
#[derive(Debug, Deserialize)]
struct RawDecisionRequest {
    state: State,
    questions: Vec<DecisionQuestion>,
    /// Optional on the wire: omitted means the engine's default posture.
    /// A hand-written quickstart should not have to invent policy fields
    /// it has not formed an opinion about.
    #[serde(default)]
    policy: DecisionPolicy,
    #[serde(default)]
    metadata: RequestMetadata,
}

impl TryFrom<RawDecisionRequest> for DecisionRequest {
    type Error = CoreError;

    fn try_from(raw: RawDecisionRequest) -> CoreResult<Self> {
        Self::new(raw.state, raw.questions, raw.policy, raw.metadata)
    }
}

impl DecisionRequest {
    /// Validates and constructs a request.
    ///
    /// Enforced limits: at least one question, no more than
    /// [`Limits::max_questions`](crate::policy::Limits::max_questions),
    /// state text within
    /// [`Limits::max_input_bytes`](crate::policy::Limits::max_input_bytes),
    /// choice candidate sets within
    /// [`Limits::max_candidates`](crate::policy::Limits::max_candidates),
    /// and attached images within
    /// [`Limits::max_images`](crate::policy::Limits::max_images) and
    /// [`Limits::max_image_bytes`](crate::policy::Limits::max_image_bytes)
    /// each.
    pub fn new(
        state: State,
        questions: Vec<DecisionQuestion>,
        policy: DecisionPolicy,
        metadata: RequestMetadata,
    ) -> CoreResult<Self> {
        let limits = &metadata.limits;
        if questions.is_empty() {
            return Err(CoreError::EmptyQuestions);
        }
        if questions.len() > limits.max_questions {
            return Err(CoreError::InvalidPolicy {
                reason: format!(
                    "request carries {} questions, limit is {}",
                    questions.len(),
                    limits.max_questions
                ),
            });
        }
        if state.text().len() > limits.max_input_bytes {
            return Err(CoreError::InvalidPolicy {
                reason: format!(
                    "state text is {} bytes, limit is {}",
                    state.text().len(),
                    limits.max_input_bytes
                ),
            });
        }
        if state.image_count() > limits.max_images {
            return Err(CoreError::TooManyImages {
                count: state.image_count(),
                limit: limits.max_images,
            });
        }
        for image in state.images() {
            if image.data().len() > limits.max_image_bytes {
                return Err(CoreError::ImageTooLarge {
                    bytes: image.data().len(),
                    limit: limits.max_image_bytes,
                });
            }
        }
        for question in &questions {
            if let DecisionQuestion::Choice(choice) = question
                && choice.candidates().len() > limits.max_candidates
            {
                return Err(CoreError::InvalidPolicy {
                    reason: format!(
                        "question `{}` carries {} candidates, limit is {}",
                        choice.id(),
                        choice.candidates().len(),
                        limits.max_candidates
                    ),
                });
            }
        }
        Ok(Self { state, questions, policy, metadata })
    }

    /// The state this request is evaluated against.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The questions to decide, in order.
    pub fn questions(&self) -> &[DecisionQuestion] {
        &self.questions
    }

    /// The confidence/risk policy governing the outcome cascade.
    pub fn policy(&self) -> &DecisionPolicy {
        &self.policy
    }

    /// Request metadata and limits.
    pub fn metadata(&self) -> &RequestMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::policy::Limits;
    use crate::question::{BooleanQuestion, Candidate, ChoiceQuestion};
    use crate::state::{ImageInput, ImageMediaType};

    /// Small deterministic image of `bytes` decoded bytes.
    fn image(media_type: ImageMediaType, bytes: usize) -> ImageInput {
        ImageInput::new(media_type, vec![0xAB; bytes])
    }

    fn sample_question() -> DecisionQuestion {
        DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model?",
                vec![Candidate::new("qwen", "general").expect("valid")],
            )
            .expect("valid"),
        )
    }

    #[test]
    fn accepts_valid_request() {
        let request = DecisionRequest::new(
            State::from_text("refactor this parser"),
            vec![sample_question()],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .expect("valid");
        assert_eq!(request.questions().len(), 1);
        assert_eq!(request.state().text(), "refactor this parser");
    }

    #[test]
    fn exposes_the_policy_and_metadata_it_was_built_with() {
        let policy = DecisionPolicy::default();
        let metadata =
            RequestMetadata { request_id: Some("req-0001".to_owned()), limits: Limits::default() };
        let request = DecisionRequest::new(
            State::from_text("route this request"),
            vec![sample_question()],
            policy.clone(),
            metadata.clone(),
        )
        .expect("valid");
        assert_eq!(request.policy(), &policy);
        assert_eq!(request.metadata(), &metadata);
    }

    #[test]
    fn rejects_empty_questions() {
        assert!(matches!(
            DecisionRequest::new(
                State::from_text("x"),
                vec![],
                DecisionPolicy::default(),
                RequestMetadata::default(),
            ),
            Err(CoreError::EmptyQuestions)
        ));
    }

    #[test]
    fn rejects_oversized_state() {
        let metadata = RequestMetadata {
            request_id: None,
            limits: Limits { max_input_bytes: 8, ..Limits::default() },
        };
        assert!(matches!(
            DecisionRequest::new(
                State::from_text("this text is longer than eight bytes"),
                vec![sample_question()],
                DecisionPolicy::default(),
                metadata,
            ),
            Err(CoreError::InvalidPolicy { .. })
        ));
    }

    #[test]
    fn rejects_oversized_image() {
        let metadata = RequestMetadata {
            request_id: None,
            limits: Limits { max_image_bytes: 8, ..Limits::default() },
        };
        let state = State::from_text("what is pictured").with_image(image(ImageMediaType::Png, 16));
        assert!(matches!(
            DecisionRequest::new(
                state,
                vec![sample_question()],
                DecisionPolicy::default(),
                metadata,
            ),
            Err(CoreError::ImageTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_too_many_images() {
        let metadata = RequestMetadata {
            request_id: None,
            limits: Limits { max_images: 1, ..Limits::default() },
        };
        let state = State::from_text("two pictures")
            .with_image(image(ImageMediaType::Png, 4))
            .with_image(image(ImageMediaType::Jpeg, 4));
        assert!(matches!(
            DecisionRequest::new(
                state,
                vec![sample_question()],
                DecisionPolicy::default(),
                metadata,
            ),
            Err(CoreError::TooManyImages { .. })
        ));
    }

    #[test]
    fn accepts_images_within_limits() {
        let state = State::from_text("what is pictured")
            .with_image(image(ImageMediaType::Png, 8))
            .with_image(image(ImageMediaType::Jpeg, 8));
        let request = DecisionRequest::new(
            state,
            vec![sample_question()],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .expect("within default limits");
        assert_eq!(request.state().image_count(), 2);
    }

    #[test]
    fn rejects_too_many_candidates() {
        let metadata = RequestMetadata {
            request_id: None,
            limits: Limits { max_candidates: 2, ..Limits::default() },
        };
        let candidates: Vec<Candidate> =
            ["a", "b", "c"].iter().map(|id| Candidate::new(*id, "d").expect("valid")).collect();
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new("model", "Which?", candidates).expect("valid"),
        );
        assert!(matches!(
            DecisionRequest::new(
                State::from_text("x"),
                vec![question],
                DecisionPolicy::default(),
                metadata,
            ),
            Err(CoreError::InvalidPolicy { .. })
        ));
    }

    #[test]
    fn round_trips_through_json() {
        let request = DecisionRequest::new(
            State::from_text("hello").with_fact("k", crate::state::FactValue::Integer(1)),
            vec![
                sample_question(),
                DecisionQuestion::Boolean(
                    BooleanQuestion::new("tools", "Needs tools?").expect("valid"),
                ),
            ],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .expect("valid");
        let json = serde_json::to_string(&request).expect("serialize");
        let back: DecisionRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, request);
    }

    #[test]
    fn policy_and_metadata_are_optional_on_the_wire() {
        // The minimal hand-written payload — the shape a quickstart
        // documents — decodes with the engine's default policy and empty
        // metadata; a caller never invents housekeeping fields to ask one
        // question.
        let minimal = serde_json::from_str::<DecisionRequest>(
            r#"{
                "state": {"text": "deploy is erroring, decide"},
                "questions": [{"type": "boolean", "id": "rollback", "text": "Roll back now?"}]
            }"#,
        )
        .expect("minimal payload decodes");
        assert_eq!(minimal.policy(), &DecisionPolicy::default());
        assert_eq!(minimal.metadata(), &RequestMetadata::default());
    }
}
