//! Question rendering helpers shared by the serving adapters.
//!
//! Lifted out of `llamacpp` so the tree-mode adapter and the NLI
//! verbalization verifier (`nli`, PLAN 24j) declare the answer keys and
//! the question text from one implementation — a per-adapter copy of
//! the declared answer order is exactly how a wire adapter and a
//! decision trace drift apart.

use opencodifier_core::DecisionQuestion;

/// The answer keys `question`'s distribution must use, in declared
/// order (`"true"`/`"false"` for booleans, level labels for score
/// questions, candidate ids for choices). `None` for a kind this
/// engine does not know — `DecisionQuestion` is `#[non_exhaustive]`.
pub(crate) fn answers_of(question: &DecisionQuestion) -> Option<Vec<String>> {
    match question {
        DecisionQuestion::Choice(choice) => {
            Some(choice.candidates().iter().map(|candidate| candidate.id().to_string()).collect())
        }
        DecisionQuestion::Boolean(_) => Some(vec!["true".to_owned(), "false".to_owned()]),
        DecisionQuestion::Score(score) => {
            Some(score.levels().iter().map(|level| level.label().to_owned()).collect())
        }
        _ => None,
    }
}

/// The text a serving adapter names as the question.
pub(crate) fn question_text(question: &DecisionQuestion) -> &str {
    match question {
        DecisionQuestion::Choice(choice) => choice.text(),
        DecisionQuestion::Boolean(boolean) => boolean.text(),
        DecisionQuestion::Score(score) => score.text(),
        _ => "",
    }
}
