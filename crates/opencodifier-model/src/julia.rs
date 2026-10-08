//! The Julia-1 ONNX decision contract (task #92).
//!
//! [`JuliaQuestion`] turns a decision request into the inputs the
//! `Julia-1` runtime renders, [`JuliaEncoding`] turns that rendering
//! into the token ids and marker indices its ONNX graph consumes, and
//! [`probabilities`] turns the graph's logits into a probability
//! distribution. Everything here is pure Rust and all decision math is
//! `f64` (DECISIONS.md D7). The softmax is the crate's strict one
//! ([`crate::kai::softmax`]) — both ONNX rungs share the "no garbage
//! fallback" posture, and there is no second implementation to drift.
//!
//! # Contract, extracted and verified
//!
//! The encoder is upstream `julia.data.sequence` over the model
//! directory's `tokenizer.json`:
//!
//! ```text
//! ids  = [<bos>] + head[:max(8, 256 − option_tokens)] + <eos>
//!        then per option: [<mask>] + text_tokens[:48]
//!        then <eos> + state_tokens[:room] + <eos>
//! room = 1024 − len(ids) − 1
//! head = encode("{kind} question: {question}")
//! ```
//!
//! * The graph reads each option at its **marker**: the absolute index
//!   of the option's `<mask>` token — the option's *first* token, not
//!   the last (that is the Kai construction, a different model).
//! * `marker_mask` is all-true for a whole row: every declared option
//!   is scored, and the row width is the declared candidate count.
//! * The graph takes a fifth input, `qtype` (shape `[batch]`, one per
//!   row), naming the primitive: choice 0, score 1, noul 2
//!   ([`JuliaKind::qtype`]). The parity cases confirm the value, not
//!   just the prompt prefix, selects the readout head.
//! * The head is packed to `head_length = 256` tokens **including** the
//!   option budget, but never below 8 tokens; the state fills whatever
//!   room remains up to `max_length = 1024`. Upstream slices silently
//!   when the pieces overrun; this contract refuses with
//!   [`ModelError::PromptTooLong`] past the budget (truncation moves
//!   markers the same way it moves pointers).
//! * An empty state is legal — several upstream rows carry one — but an
//!   empty head or option text is a caller bug and is refused.
//! * Logits are **pre-softmax**, one per option, temperature 1. No
//!   fitted bias table ships for this model: upstream publishes none and
//!   inventing calibration is forbidden (the Kai 5-level bias belongs to
//!   that rung only).
//!
//! Verified on the compute host (fedora) 2026-10-05:
//! `julia1_parity.py` reproduces the export's official parity bar on the
//! full 100-row `parity-cases.json` (100/100 predictions, max logit
//! error 7.8e-5); [`JULIA_FIXTURE`] freezes three of those rows —
//! narrowest, median-width, widest — with their real-tokenizer ids and
//! markers, reproduced in-tree byte for byte by the
//! [`crate::kai::BpeTokenizer`] oracle. The export records no
//! `noul`/`score` cases, so those kinds are exercised here with a stub
//! tokenizer over the same sequence math.

use std::collections::BTreeMap;

use opencodifier_core::{DecisionQuestion, State};
use opencodifier_runtime::{
    ATTENTION_MASK, DenseTensor, INPUT_IDS, MARKER_MASK, MARKER_POS, QTYPE_INPUT,
};

use crate::error::ModelError;
use crate::kai::{KaiTokenizer, MAX_CANDIDATES, MIN_CANDIDATES, softmax};

/// Sequence contract version this module implements.
pub const PROMPT_VERSION: &str = "julia-data-sequence-v1";

/// Total token budget of the Julia-1 contract (`max_length`).
pub const MAX_LENGTH: usize = 1024;

/// Token budget of the question head plus its options (`head_length`).
pub const HEAD_LENGTH: usize = 256;

/// Most tokens an option's text contributes after its marker.
pub const OPTION_TOKEN_LIMIT: usize = 48;

/// Fewest options a question may carry (upstream rows are ≥ 2).
pub const MIN_OPTIONS: usize = MIN_CANDIDATES;

/// Most options a question may carry.
///
/// Upstream publishes no hard cap; this is the OpenCodifier-side guard,
/// matching the Kai rung's candidate ceiling.
pub const MAX_OPTIONS: usize = MAX_CANDIDATES;

/// `<bos>` in the Julia-1 tokenizer (frozen by [`JULIA_FIXTURE`]).
pub const CLS_ID: i64 = 2;

/// `<eos>` in the Julia-1 tokenizer (frozen by [`JULIA_FIXTURE`]).
pub const SEP_ID: i64 = 1;

/// `<mask>` in the Julia-1 tokenizer (frozen by [`JULIA_FIXTURE`]).
pub const MASK_ID: i64 = 4;

/// The frozen parity fixture: three recorded choice rows.
///
/// Provenance and the sequence constants are inside the file. Each case
/// carries the real tokenizer's ids and markers for its segments, the
/// export's recorded logits, and the texts the segments were built from.
pub const JULIA_FIXTURE: &str = include_str!("julia/fixtures/julia-reference-trimmed.json");

/// The tokenizer asset that reproduces the frozen cases' ids.
///
/// A derivation closure over the frozen segments, not a vocab subset:
/// the Julia-1 tokenizer is Metaspace (space → `▁`) with byte fallback,
/// so its ids cannot be re-derived from final ids alone. Each real
/// token's spelling carries its real id and a left-to-right merge chain;
/// chains are ordered metaspace runs first, then longest, and any pair
/// that raced ahead of a run's own chain was demoted — the generator
/// verified every segment byte for byte against the
/// [`crate::kai::BpeTokenizer`] algorithm before shipping
/// (`benchmarks/validation/runner/julia_fixture_gen2.py`). Serving uses
/// the full tokenizer behind the `tokenizers` feature; here the oracle
/// proves the contract's ids.
pub const JULIA_FIXTURE_TOKENIZER: &str =
    include_str!("julia/fixtures/julia-fixture-tokenizer.json");

/// Which decision primitive a Julia-1 question asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JuliaKind {
    /// Select one option — upstream `choice`.
    Choice,
    /// Ordered levels — upstream `score`, keys `"0"`…
    Score,
    /// Yes/no with probability — upstream `noul`, keys `false`/`true`.
    Noul,
}

impl JuliaKind {
    /// The task-type string the head text carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }

    /// The `qtype` tensor value the graph reads for this kind:
    /// choice 0, score 1, noul 2 — the upstream `QTYPES` mapping, and
    /// not derivable from the prompt text alone.
    #[must_use]
    pub fn qtype(self) -> i64 {
        match self {
            Self::Choice => 0,
            Self::Score => 1,
            Self::Noul => 2,
        }
    }
}

/// One option: the key the distribution reports and the text the
/// prompt embeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JuliaOption {
    /// The answer key reported in the distribution (for a choice
    /// question: the candidate id, so a prediction maps back to a
    /// candidate; upstream rows carry only texts and use them as keys).
    pub key: String,
    /// The option text embedded after the option's marker.
    pub text: String,
}

/// A Julia-1 decision question: the exact input the contract renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JuliaQuestion {
    /// Identifier carried into error messages.
    pub id: String,
    /// The state text (may be empty — upstream rows carry empty states).
    pub state: String,
    /// The question text embedded after the task-type prefix.
    pub question: String,
    /// Which primitive this is.
    pub kind: JuliaKind,
    /// The options, in distribution order.
    pub options: Vec<JuliaOption>,
}

impl JuliaQuestion {
    /// Builds a question, enforcing the contract's cardinality and key
    /// rules.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when the option count is outside
    /// `2..=255`, an option key or text is empty, the question text is
    /// empty, or a `noul` question's keys are not exactly `false` and
    /// `true` in that order.
    pub fn new(
        id: impl Into<String>,
        state: impl Into<String>,
        question: impl Into<String>,
        kind: JuliaKind,
        options: Vec<JuliaOption>,
    ) -> Result<Self, ModelError> {
        let id = id.into();
        let question = question.into();
        if options.len() < MIN_OPTIONS || options.len() > MAX_OPTIONS {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "`{id}`: {} carries {} options, outside {MIN_OPTIONS}..={MAX_OPTIONS}",
                    kind.as_str(),
                    options.len()
                ),
            });
        }
        if question.trim().is_empty() {
            return Err(ModelError::ContractViolation {
                reason: format!("`{id}`: the question text must be nonempty"),
            });
        }
        for (index, option) in options.iter().enumerate() {
            if option.key.trim().is_empty() {
                return Err(ModelError::ContractViolation {
                    reason: format!("`{id}`: option {index} has an empty key"),
                });
            }
            if option.text.trim().is_empty() {
                return Err(ModelError::ContractViolation {
                    reason: format!("`{id}`: option {index} has empty text"),
                });
            }
        }
        if kind == JuliaKind::Noul {
            let expected = ["false", "true"];
            let actual: Vec<&str> = options.iter().map(|option| option.key.as_str()).collect();
            if actual != expected {
                return Err(ModelError::ContractViolation {
                    reason: format!(
                        "`{id}`: noul requires keys false,true in order, got {actual:?}"
                    ),
                });
            }
        }
        Ok(Self { id, state: state.into(), question, kind, options })
    }

    /// Maps an `OpenCodifier` request onto the Julia-1 contract.
    ///
    /// Choice candidates keep their ids as keys and carry their
    /// descriptions as option text (falling back to the id when the
    /// description is empty, matching how the upstream rows look);
    /// boolean questions become `noul` with `false`/`true`; score levels
    /// become keys `"0"`… with their labels as text.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when the state carries typed
    /// facts (no upstream-trained rendering exists for them — project
    /// them into the state text first) or the question breaks the rules
    /// above.
    pub fn from_decision(state: &State, question: &DecisionQuestion) -> Result<Self, ModelError> {
        if state.facts().next().is_some() {
            return Err(ModelError::ContractViolation {
                reason: "the Julia-1 rung renders text states only: project typed facts into \
                         the state text before rendering"
                    .to_owned(),
            });
        }
        let state = state.text().to_owned();
        match question {
            DecisionQuestion::Choice(choice) => {
                let options = choice
                    .candidates()
                    .iter()
                    .map(|candidate| {
                        let description = candidate.description();
                        let text = if description.trim().is_empty() {
                            candidate.id().as_str()
                        } else {
                            description
                        };
                        JuliaOption {
                            key: candidate.id().as_str().to_owned(),
                            text: text.to_owned(),
                        }
                    })
                    .collect();
                Self::new(choice.id().as_str(), state, choice.text(), JuliaKind::Choice, options)
            }
            DecisionQuestion::Boolean(boolean) => Self::new(
                boolean.id().as_str(),
                state,
                boolean.text(),
                JuliaKind::Noul,
                vec![
                    JuliaOption { key: "false".to_owned(), text: "false".to_owned() },
                    JuliaOption { key: "true".to_owned(), text: "true".to_owned() },
                ],
            ),
            DecisionQuestion::Score(score) => {
                let options = score
                    .levels()
                    .iter()
                    .enumerate()
                    .map(|(index, level)| JuliaOption {
                        key: index.to_string(),
                        text: level.label().to_owned(),
                    })
                    .collect();
                Self::new(score.id().as_str(), state, score.text(), JuliaKind::Score, options)
            }
            _ => Err(ModelError::ContractViolation {
                reason: "unrecognized decision question shape; the Julia-1 rung has no \
                         rendering for it"
                    .to_owned(),
            }),
        }
    }

    /// The head text: the task-type prefix the upstream `sequence`
    /// prepends to the question.
    #[must_use]
    pub fn head_text(&self) -> String {
        format!("{} question: {}", self.kind.as_str(), self.question)
    }

    /// The option segment text: a leading space, the way upstream
    /// encodes each option.
    #[must_use]
    pub fn option_segment(&self, index: usize) -> String {
        format!(" {}", self.options[index].text)
    }

    /// The answer keys, in distribution order.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.options.iter().map(|option| option.key.clone()).collect()
    }
}

/// The tokenizer a Julia-1 encoding needs: text → ids, no special
/// tokens (the contract adds `<bos>`/`<eos>`/`<mask>` itself).
pub trait JuliaTokenizer: std::fmt::Debug {
    /// Encodes `text` without special tokens.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when the text cannot be encoded.
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError>;
}

impl<T: KaiTokenizer> JuliaTokenizer for T {
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError> {
        KaiTokenizer::encode(self, text)
    }
}

/// One encoded question: the ids and marker indices the graph consumes,
/// and the invariants that make its marker readout safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JuliaEncoding {
    /// Prompt token ids, special tokens included.
    pub ids: Vec<i64>,
    /// Absolute token index of each option's `<mask>` token.
    pub marker_pos: Vec<usize>,
    /// Answer keys, in marker order.
    pub keys: Vec<String>,
    /// Which primitive this encodes.
    pub kind: JuliaKind,
}

impl JuliaEncoding {
    /// Encodes `question` under the contract.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] for an encoding failure and
    /// [`ModelError::PromptTooLong`] when the assembled prompt exceeds
    /// `max_length` — the contract refuses, never truncates (upstream
    /// slices silently; the delta is deliberate and documented at the
    /// module level).
    pub fn encode(
        question: &JuliaQuestion,
        tokenizer: &dyn JuliaTokenizer,
        max_length: usize,
    ) -> Result<Self, ModelError> {
        let head = tokenizer.encode(&question.head_text())?;
        if head.is_empty() {
            return Err(ModelError::Tokenizer {
                reason: format!("`{}`: the question head encoded to no tokens", question.id),
            });
        }
        let mut options = Vec::with_capacity(question.options.len());
        for index in 0..question.options.len() {
            let text = tokenizer.encode(&question.option_segment(index))?;
            if text.is_empty() {
                return Err(ModelError::Tokenizer {
                    reason: format!("`{}`: option {index} encoded to no tokens", question.id),
                });
            }
            let mut segment = Vec::with_capacity(1 + OPTION_TOKEN_LIMIT);
            segment.push(MASK_ID);
            segment.extend(text.into_iter().take(OPTION_TOKEN_LIMIT));
            options.push(segment);
        }
        let option_tokens: usize = options.iter().map(Vec::len).sum();
        let budget = HEAD_LENGTH.saturating_sub(option_tokens);
        let keep = head.len().min(budget.max(8));

        let mut ids = Vec::with_capacity(max_length.min(1 + keep + 1 + option_tokens + 1));
        ids.push(CLS_ID);
        ids.extend(head[..keep].iter().copied());
        ids.push(SEP_ID);
        let mut marker_pos = Vec::with_capacity(options.len());
        for segment in &options {
            marker_pos.push(ids.len());
            ids.extend(segment.iter().copied());
        }
        ids.push(SEP_ID);
        let room = max_length.saturating_sub(ids.len() + 1);
        if room > 0 {
            ids.extend(tokenizer.encode(&question.state)?.into_iter().take(room));
        }
        ids.push(SEP_ID);

        let encoding = Self { ids, marker_pos, keys: question.keys(), kind: question.kind };
        if encoding.ids.len() > max_length {
            return Err(ModelError::PromptTooLong {
                id: question.id.clone(),
                tokens: encoding.ids.len(),
                max_tokens: max_length,
            });
        }
        encoding.validate()?;
        Ok(encoding)
    }

    /// Asserts the marker invariants the pointer-discipline posture
    /// demands.
    ///
    /// One marker per option, strictly increasing, every marker inside
    /// the prompt and landing on a `<mask>`. A violation is a typed
    /// error: a marker readout fed a mis-anchored index produces
    /// confidently wrong scores, not a failure.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] describing which invariant broke.
    pub fn validate(&self) -> Result<(), ModelError> {
        let count = self.keys.len();
        if self.marker_pos.len() != count {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "marker count {} must equal the option count {count}",
                    self.marker_pos.len()
                ),
            });
        }
        if !self.marker_pos.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(ModelError::ContractViolation {
                reason: "markers must be strictly increasing".to_owned(),
            });
        }
        if let Some(position) = self.marker_pos.iter().find(|position| **position >= self.ids.len())
        {
            return Err(ModelError::ContractViolation {
                reason: format!("marker {position} is outside the {}-token prompt", self.ids.len()),
            });
        }
        if let Some(position) =
            self.marker_pos.iter().find(|position| self.ids[**position] != MASK_ID)
        {
            return Err(ModelError::ContractViolation {
                reason: format!("marker {position} does not land on a `<mask>` token"),
            });
        }
        Ok(())
    }

    /// Builds the named tensors the backend consumes, one row.
    ///
    /// Batched padding belongs to the upstream collator and is not
    /// fixture-pinned here, so the contract produces exactly one row per
    /// encoding; callers loop. The marker lanes cross as integral-valued
    /// `f32` like every other index tensor, and the all-true
    /// `marker_mask` crosses the same way (0.0/1.0 → bool on the
    /// transport). The row's `qtype` carries [`JuliaKind::qtype`].
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when a tensor shape is rejected.
    pub fn to_tensors(&self) -> Result<BTreeMap<String, DenseTensor>, ModelError> {
        let build =
            |name: &str, shape: Vec<usize>, data: &[i64]| -> Result<DenseTensor, ModelError> {
                #[allow(clippy::cast_precision_loss)] // ids and positions stay far below 2^24
                let floats: Vec<f32> = data.iter().map(|value| *value as f32).collect();
                DenseTensor::new(shape, floats).map_err(|error| ModelError::ContractViolation {
                    reason: format!("`{name}`: {error}"),
                })
            };
        let length = self.ids.len();
        let width = self.keys.len();
        let mask = vec![1; length];
        let marker_mask = vec![1; width];
        // Token positions come from `Vec<usize>` index math and stay far
        // below `i64::MAX`; the cast cannot wrap.
        #[allow(clippy::cast_possible_wrap)]
        let markers: Vec<i64> = self.marker_pos.iter().map(|position| *position as i64).collect();
        Ok(BTreeMap::from([
            (INPUT_IDS.to_owned(), build(INPUT_IDS, vec![1, length], &self.ids)?),
            (ATTENTION_MASK.to_owned(), build(ATTENTION_MASK, vec![1, length], &mask)?),
            (MARKER_POS.to_owned(), build(MARKER_POS, vec![1, width], &markers)?),
            (MARKER_MASK.to_owned(), build(MARKER_MASK, vec![1, width], &marker_mask)?),
            (QTYPE_INPUT.to_owned(), build(QTYPE_INPUT, vec![1], &[self.kind.qtype()])?),
        ]))
    }
}

/// Turns graph logits into the distribution the runtime reports.
///
/// `f64` softmax at temperature 1, in key order — there is no fitted
/// bias table for this model, so the logits pass through unmodified.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the logits are empty, carry a
/// non-finite value, or disagree with the key count.
pub fn probabilities(keys: &[String], logits: &[f64]) -> Result<Vec<f64>, ModelError> {
    if logits.len() != keys.len() {
        return Err(ModelError::ContractViolation {
            reason: format!("{} logits must match {} keys", logits.len(), keys.len()),
        });
    }
    softmax(logits)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]

    use std::collections::HashMap;

    use super::*;
    use crate::kai::BpeTokenizer;

    /// The trimmed fixture, parsed.
    fn fixture_cases() -> Vec<FixtureCase> {
        use serde_json::Value;

        let parsed: Value = serde_json::from_str(JULIA_FIXTURE).expect("fixture is valid JSON");
        let specials = &parsed["special_tokens"];
        assert_eq!(specials["cls"].as_i64(), Some(CLS_ID));
        assert_eq!(specials["sep"].as_i64(), Some(SEP_ID));
        assert_eq!(specials["mask"].as_i64(), Some(MASK_ID));
        parsed["cases"]
            .as_array()
            .expect("fixture carries cases")
            .iter()
            .map(|case| FixtureCase {
                id: case["id"].as_str().unwrap_or_default().to_owned(),
                kind: match case["kind"].as_str() {
                    Some("score") => JuliaKind::Score,
                    Some("noul") => JuliaKind::Noul,
                    _ => JuliaKind::Choice,
                },
                head_text: case["head_text"].as_str().unwrap_or_default().to_owned(),
                option_texts: case["option_texts"]
                    .as_array()
                    .map(|items| {
                        items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect()
                    })
                    .unwrap_or_default(),
                state_text: case["state_text"].as_str().unwrap_or_default().to_owned(),
                ids: case["ids"]
                    .as_array()
                    .map(|items| items.iter().filter_map(Value::as_i64).collect())
                    .unwrap_or_default(),
                markers: case["markers"]
                    .as_array()
                    .map(|items| {
                        items.iter().filter_map(Value::as_i64).map(|value| value as usize).collect()
                    })
                    .unwrap_or_default(),
                logits: case["logits"]
                    .as_array()
                    .map(|items| items.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// One frozen fixture row.
    #[derive(Debug)]
    struct FixtureCase {
        /// The case id (`fixture-choice-N`).
        id: String,
        /// Which primitive the row encodes.
        kind: JuliaKind,
        /// The upstream head text (`{kind} question: {question}`).
        head_text: String,
        /// The option texts, in order.
        option_texts: Vec<String>,
        /// The state text.
        state_text: String,
        /// The real tokenizer's ids for the assembled prompt.
        ids: Vec<i64>,
        /// The real tokenizer's option markers.
        markers: Vec<usize>,
        /// The export's recorded logits.
        logits: Vec<f64>,
    }

    /// A whitespace oracle: one id per known word, so sequence-math
    /// tests are hand-checkable. Unknown words are errors, like any
    /// honest tokenizer over a closed vocabulary.
    #[derive(Debug)]
    struct StubTokenizer {
        words: HashMap<String, i64>,
    }

    impl StubTokenizer {
        fn new(pairs: &[(&str, i64)]) -> Self {
            Self { words: pairs.iter().map(|(word, id)| ((*word).to_owned(), *id)).collect() }
        }
    }

    impl JuliaTokenizer for StubTokenizer {
        fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError> {
            text.split_whitespace()
                .map(|word| {
                    self.words.get(word).copied().ok_or_else(|| ModelError::Tokenizer {
                        reason: format!("unknown word `{word}`"),
                    })
                })
                .collect()
        }
    }

    fn stub() -> StubTokenizer {
        StubTokenizer::new(&[
            ("choice", 10),
            ("score", 11),
            ("noul", 12),
            ("question:", 13),
            ("pick", 14),
            ("one", 15),
            ("alpha", 20),
            ("beta", 21),
            ("gamma", 22),
            ("two", 16),
            ("three", 17),
            ("four", 18),
            ("five", 19),
            ("six", 50),
            ("seven", 51),
            ("false", 30),
            ("true", 31),
            ("state", 40),
            ("text", 41),
        ])
    }

    /// The stub is an honest tokenizer: a closed vocabulary refuses
    /// unknown words instead of guessing an id, so a sequence-math test
    /// can never silently pass over missing vocabulary.
    #[test]
    fn the_stub_tokenizer_refuses_unknown_words() {
        let error = stub().encode("unvocabularied").unwrap_err();
        assert!(matches!(error, ModelError::Tokenizer { .. }), "{error}");
        assert_eq!(error.code(), "model.tokenizer");
        assert!(error.to_string().contains("unknown word `unvocabularied`"), "{error}");
    }

    fn choice_question() -> JuliaQuestion {
        JuliaQuestion::new(
            "q1",
            "state text",
            "pick one",
            JuliaKind::Choice,
            vec![
                JuliaOption { key: "alpha".to_owned(), text: "alpha".to_owned() },
                JuliaOption { key: "beta".to_owned(), text: "beta".to_owned() },
            ],
        )
        .unwrap()
    }

    #[test]
    fn wire_names_match_the_published_graph() {
        assert_eq!(PROMPT_VERSION, "julia-data-sequence-v1");
        assert_eq!(MAX_LENGTH, 1024);
        assert_eq!(HEAD_LENGTH, 256);
        assert_eq!(OPTION_TOKEN_LIMIT, 48);
        assert_eq!((CLS_ID, SEP_ID, MASK_ID), (2, 1, 4));
    }

    #[test]
    fn the_sequence_matches_the_upstream_layout() {
        let encoding = JuliaEncoding::encode(&choice_question(), &stub(), MAX_LENGTH).unwrap();
        // head: [CLS] "choice question: pick one" [SEP] = 1 + 4 + 1
        // options: [MASK] alpha, [MASK] beta
        // tail: [SEP] "state text" [SEP]
        assert_eq!(encoding.ids, vec![2, 10, 13, 14, 15, 1, 4, 20, 4, 21, 1, 40, 41, 1]);
        assert_eq!(encoding.marker_pos, vec![6, 8]);
        assert_eq!(encoding.keys, vec!["alpha", "beta"]);
        encoding.validate().unwrap();
    }

    #[test]
    fn an_empty_state_is_legal_and_option_tokens_are_capped() {
        let question = JuliaQuestion::new(
            "q2",
            "",
            "pick one",
            JuliaKind::Choice,
            vec![
                JuliaOption { key: "a".to_owned(), text: "alpha alpha alpha".to_owned() },
                JuliaOption { key: "b".to_owned(), text: "beta".to_owned() },
            ],
        )
        .unwrap();
        let encoding = JuliaEncoding::encode(&question, &stub(), MAX_LENGTH).unwrap();
        // state contributes nothing; the tail is still SEP, (nothing), SEP.
        assert_eq!(*encoding.ids.last().unwrap(), SEP_ID);
        assert_eq!(*encoding.ids.get(encoding.ids.len() - 2).unwrap(), SEP_ID);
        assert_eq!(encoding.ids.len(), 1 + 4 + 1 + (4 + 2) + 1 + 1);
        encoding.validate().unwrap();
    }

    #[test]
    fn the_head_never_drops_below_eight_tokens() {
        // A wide option set squeezes the budget below 8; an 11-token head
        // keeps 8 tokens anyway, upstream's exact rule.
        let options: Vec<JuliaOption> = (0..40)
            .map(|index| JuliaOption {
                key: format!("k{index}"),
                text: "alpha alpha alpha alpha alpha alpha alpha alpha alpha alpha".to_owned(),
            })
            .collect();
        let question = JuliaQuestion::new(
            "q3",
            "state",
            "pick one two three four five six seven",
            JuliaKind::Choice,
            options,
        )
        .unwrap();
        let encoding = JuliaEncoding::encode(&question, &stub(), 1024).unwrap();
        // budget = 256 - (40 * 11) saturates to 0 -> keep = min(11, 8) = 8
        assert_eq!(
            encoding.ids[1..9],
            [10, 13, 14, 15, 16, 17, 18, 19],
            "the head survives its first eight tokens"
        );
        assert_eq!(encoding.marker_pos.len(), 40);
        assert_eq!(encoding.marker_pos[0], 1 + 8 + 1, "the first mask follows the kept head");
        encoding.validate().unwrap();
    }

    #[test]
    fn an_over_budget_prompt_is_refused_never_truncated() {
        let stub = stub();
        let question = choice_question();
        let error = JuliaEncoding::encode(&question, &stub, 8).unwrap_err();
        assert_eq!(error.code(), "model.prompt_too_long");
        assert!(error.to_string().contains("above the 8 token budget"), "{error}");
        // The same prompt fits under the real budget.
        let encoding = JuliaEncoding::encode(&question, &stub, MAX_LENGTH).unwrap();
        assert_eq!(encoding.ids.first(), Some(&CLS_ID));
    }

    #[test]
    fn noul_requires_false_true_in_order() {
        let error = JuliaQuestion::new(
            "q4",
            "state",
            "yes or no",
            JuliaKind::Noul,
            vec![
                JuliaOption { key: "yes".to_owned(), text: "true".to_owned() },
                JuliaOption { key: "no".to_owned(), text: "false".to_owned() },
            ],
        )
        .unwrap_err();
        assert_eq!(error.code(), "model.contract_violation");
        assert!(error.to_string().contains("false,true"), "{error}");

        JuliaQuestion::new(
            "q5",
            "state",
            "yes or no",
            JuliaKind::Noul,
            vec![
                JuliaOption { key: "false".to_owned(), text: "false".to_owned() },
                JuliaOption { key: "true".to_owned(), text: "true".to_owned() },
            ],
        )
        .unwrap();
    }

    #[test]
    fn from_decision_maps_the_ir_shapes() {
        use opencodifier_core::{
            BooleanQuestion, Candidate, ChoiceQuestion, ScoreLevel, ScoreQuestion,
        };

        let state = State::from_text("the hub is down");
        let choice = ChoiceQuestion::new(
            "c1",
            "which node?",
            vec![
                Candidate::new("gateway", "the dependency hub").unwrap(),
                Candidate::new("catalog", "").unwrap(),
            ],
        )
        .unwrap();
        let question =
            JuliaQuestion::from_decision(&state, &DecisionQuestion::Choice(choice)).unwrap();
        assert_eq!(question.kind, JuliaKind::Choice);
        assert_eq!(question.options[0].key, "gateway");
        assert_eq!(question.options[0].text, "the dependency hub");
        // An empty description falls back to the id as option text.
        assert_eq!(question.options[1].text, "catalog");

        let boolean = BooleanQuestion::new("b1", "is it urgent?").unwrap();
        let question =
            JuliaQuestion::from_decision(&state, &DecisionQuestion::Boolean(boolean)).unwrap();
        assert_eq!(question.kind, JuliaKind::Noul);
        assert_eq!(question.keys(), vec!["false", "true"]);

        let score = ScoreQuestion::new(
            "s1",
            "how severe?",
            vec![
                ScoreLevel::new("calm").unwrap(),
                ScoreLevel::new("frustrated").unwrap(),
                ScoreLevel::new("furious").unwrap(),
            ],
        )
        .unwrap();
        let question =
            JuliaQuestion::from_decision(&state, &DecisionQuestion::Score(score)).unwrap();
        assert_eq!(question.kind, JuliaKind::Score);
        assert_eq!(question.keys(), vec!["0", "1", "2"]);
        assert_eq!(question.options[2].text, "furious");
    }

    #[test]
    fn probabilities_need_matching_widths_and_stay_normalized() {
        let keys = vec!["a".to_owned(), "b".to_owned()];
        let error = probabilities(&keys, &[1.0]).unwrap_err();
        assert!(error.to_string().contains("must match"), "{error}");

        let distribution = probabilities(&keys, &[0.0, std::f64::consts::LN_2]).unwrap();
        let sum: f64 = distribution.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);
        assert!(distribution[1] > distribution[0]);
    }

    #[test]
    fn tensors_carry_the_whole_row_with_an_all_true_marker_mask() {
        let encoding = JuliaEncoding::encode(&choice_question(), &stub(), MAX_LENGTH).unwrap();
        let tensors = encoding.to_tensors().unwrap();
        assert_eq!(tensors[INPUT_IDS].shape(), &[1, encoding.ids.len()]);
        assert_eq!(tensors[ATTENTION_MASK].shape(), &[1, encoding.ids.len()]);
        assert_eq!(tensors[MARKER_POS].shape(), &[1, 2]);
        assert_eq!(tensors[MARKER_MASK].shape(), &[1, 2]);
        assert_eq!(tensors[QTYPE_INPUT].shape(), &[1]);
        assert_eq!(tensors[QTYPE_INPUT].data(), &[0.0], "choice is qtype 0");
        assert!(tensors[MARKER_MASK].data().iter().all(|value| *value == 1.0));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let first_marker = tensors[MARKER_POS].data()[0] as usize;
        assert_eq!(first_marker, encoding.marker_pos[0]);
    }

    #[test]
    fn qtype_names_the_primitive_the_graph_reads() {
        assert_eq!(JuliaKind::Choice.qtype(), 0);
        assert_eq!(JuliaKind::Score.qtype(), 1);
        assert_eq!(JuliaKind::Noul.qtype(), 2);
        let noul = JuliaQuestion::new(
            "q6",
            "state",
            "pick one",
            JuliaKind::Noul,
            vec![
                JuliaOption { key: "false".to_owned(), text: "false".to_owned() },
                JuliaOption { key: "true".to_owned(), text: "true".to_owned() },
            ],
        )
        .unwrap();
        let encoding = JuliaEncoding::encode(&noul, &stub(), MAX_LENGTH).unwrap();
        let tensors = encoding.to_tensors().unwrap();
        assert_eq!(tensors[QTYPE_INPUT].data(), &[2.0]);
    }

    // -- frozen fixture: the real tokenizer's ids, reproduced in-tree --

    fn fixture_question(case: &FixtureCase) -> JuliaQuestion {
        let question =
            case.head_text.strip_prefix("choice question: ").unwrap_or(&case.head_text).to_owned();
        JuliaQuestion::new(
            case.id.clone(),
            case.state_text.clone(),
            question,
            case.kind,
            case.option_texts
                .iter()
                .map(|text| JuliaOption { key: text.clone(), text: text.clone() })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn fixture_cases_carry_choice_rows_with_recorded_logits() {
        let cases = fixture_cases();
        assert_eq!(cases.len(), 3, "narrowest, median-width, widest");
        for case in &cases {
            assert_eq!(case.kind, JuliaKind::Choice);
            assert_eq!(case.logits.len(), case.option_texts.len());
            assert_eq!(case.markers.len(), case.option_texts.len());
        }
    }

    #[test]
    fn the_bpe_oracle_reproduces_the_real_tokenizer_ids_and_markers() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(JULIA_FIXTURE_TOKENIZER).unwrap();
        for case in fixture_cases() {
            let question = fixture_question(&case);
            let encoding = JuliaEncoding::encode(&question, &tokenizer, MAX_LENGTH).unwrap();
            assert_eq!(encoding.ids, case.ids, "ids for {}", case.id);
            assert_eq!(encoding.marker_pos, case.markers, "markers for {}", case.id);
        }
    }

    #[test]
    fn softmax_of_the_recorded_logits_is_normalized_per_case() {
        for case in fixture_cases() {
            let distribution = probabilities(&case.option_texts, &case.logits).unwrap();
            let sum: f64 = distribution.iter().sum();
            assert!((sum - 1.0).abs() < 1e-9, "case {}", case.id);
        }
    }

    #[test]
    fn the_whole_row_tensors_carry_integral_f32_only() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(JULIA_FIXTURE_TOKENIZER).unwrap();
        let case = &fixture_cases()[0];
        let encoding =
            JuliaEncoding::encode(&fixture_question(case), &tokenizer, MAX_LENGTH).unwrap();
        let tensors = encoding.to_tensors().unwrap();
        for (name, tensor) in &tensors {
            assert!(
                tensor.data().iter().all(|value| value.fract() == 0.0),
                "`{name}` carries a fractional index value"
            );
        }
    }
}
