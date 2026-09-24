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
pub struct DecisionRequest {
    state: State,
    questions: Vec<DecisionQuestion>,
    policy: DecisionPolicy,
    metadata: RequestMetadata,
}

impl DecisionRequest {
    /// Validates and constructs a request.
    ///
    /// Enforced limits: at least one question, no more than
    /// [`Limits::max_questions`], state text within
    /// [`Limits::max_input_bytes`], choice candidate sets within
    /// [`Limits::max_candidates`].
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
}
