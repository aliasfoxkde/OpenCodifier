//! The Kai-0.6B-ONNX decision contract (RESEARCH.md §10.5, task #92).
//!
//! [`KaiQuestion`] turns a decision request into the exact prompt the
//! `vllm-sr/Decision-2.0-Kai-0.6B` runtime renders, [`KaiEncoding`] turns
//! that prompt into the token ids and pointer indices its fused ONNX
//! graph consumes, and [`probabilities`] turns the graph's logits into a
//! probability distribution the way the upstream runtime does. Everything
//! here is pure Rust: rendering, pointer math, and score bias are
//! feature-free, and all decision math is `f64` (DECISIONS.md D7).
//!
//! # Contract, extracted and verified
//!
//! `prompt_version` is `decision2-segmented-options-global-query-v1`
//! (`decision_config.json`). The encoder is upstream
//! `decision2/_vendor/dev2model/decision_model.py::encode` over
//! `segments`:
//!
//! ```text
//! Context:\n{state}\n\nTask type: {kind}\nQuestion:\n{instructions}\nOptions:
//! \n<option>\n{"description":…,"key":…}\n</option>     (once per option)
//! \n\nSelect the single option best supported by the context and instructions.\nDecision:
//! ```
//!
//! * The option line's payload is **canonical JSON**: keys sorted
//!   (`description` before `key`), compact separators, non-ASCII kept
//!   literal, non-finite floats refused.
//! * Prefix, every option line, and the suffix are tokenized
//!   **separately**; an option's pointer is the index of that option
//!   segment's **last** token, so pointers are absolute token indices in
//!   the whole prompt — there is no `len(state_ids)` base offset (that
//!   construction belongs to the Strands packer, RESEARCH §6.5, a
//!   different model).
//! * `answer_pos` is the last token of the prompt: the query vector is
//!   global, reading the `Decision:` colon rather than any option.
//! * **No truncation.** A prompt above the token budget is refused with
//!   `model.prompt_too_long`, matching upstream's `max_length_exceeded`;
//!   truncating would silently move every pointer.
//! * The head scores every candidate in one forward pass; logits are
//!   **pre-softmax**, temperature 1. Only the 5-level score rung carries
//!   a fitted additive bias (`score_bias.json`, Newton-fitted,
//!   mean-zero), applied to the logits *before* softmax.
//!
//! Verified against `conversion/fixtures/reference-Kai-0.6B.json`
//! (fetched 2026-10-05): all 11 recorded cases reproduce their `ids`,
//! `option_pos`, and `answer_pos` exactly. Three of them are frozen in
//! [`FIXTURE`] and asserted on every test run.
//!
//! # Pointer discipline (RESEARCH §6.5)
//!
//! Pointer readouts fail **non-gracefully**: a mis-anchored index yields
//! confidently wrong scores, not errors — measured at 0.2251 accuracy
//! (below chance) with mean top-probability 0.56. [`KaiEncoding::validate`]
//! therefore asserts the pointer invariants on every encoding: one
//! pointer per candidate, strictly increasing, inside the prompt and
//! strictly below the query position. A violation is a typed error.

use std::collections::{BTreeMap, HashMap};

use opencodifier_core::{DecisionQuestion, State};
use opencodifier_runtime::{
    ANSWER_POS, ATTENTION_MASK, DenseTensor, INPUT_IDS, LOGITS_OUTPUT, OPTION_POS,
};
use serde_json::Value;

use crate::error::ModelError;

/// Prompt contract version this module implements.
pub const PROMPT_VERSION: &str = "decision2-segmented-options-global-query-v1";

/// Input-token budget of the published Kai-0.6B-ONNX export
/// (`config.json` → `decision2.max_input_tokens`).
pub const MAX_INPUT_TOKENS: usize = 8192;

/// Fewest candidates a question may carry (upstream `data.py`).
pub const MIN_CANDIDATES: usize = 2;

/// Most candidates a question may carry (upstream `MAX_OPTIONS`).
pub const MAX_CANDIDATES: usize = 255;

/// Most ordered levels a score question may carry (upstream `infer.py`).
pub const MAX_SCORE_LEVELS: usize = 10;

/// Pad id used for right padding, per the upstream ONNX `parity.py`
/// convention (mask 0, so the padded lane is never read).
pub const PAD_ID: i64 = 0;

/// The frozen parity fixture: 3 of the 11 upstream recorded cases.
///
/// Provenance, the score-bias table, and the token budget are inside the
/// file; the trim covers `kind = choice` (with a null description),
/// `kind = noul`, and the 5-level score rung the fitted bias targets.
pub const FIXTURE: &str = include_str!("kai/fixtures/kai-reference-trimmed.json");

/// The tokenizer asset that reproduces the frozen cases' ids.
///
/// This is the merge closure of the three frozen prompts over the
/// upstream Qwen `tokenizer.json`, not a usable general tokenizer: it
/// knows only the vocabulary those prompts exercise. Serving uses the
/// full model directory's tokenizer behind the `tokenizers` feature.
pub const FIXTURE_TOKENIZER: &str = include_str!("kai/fixtures/kai-fixture-tokenizer.json");

/// Which decision primitive a Kai question asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KaiKind {
    /// Select one candidate — upstream `choice`.
    Choice,
    /// Yes/no with probability — upstream `noul`, keys `false` then `true`.
    Noul,
    /// Ordered levels with expected value — upstream `score`, keys `"0"`…
    Score,
}

impl KaiKind {
    /// The task-type string the prompt carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Noul => "noul",
            Self::Score => "score",
        }
    }

    /// Whether the fitted score bias may be applied to this kind's logits.
    #[must_use]
    pub fn takes_score_bias(self) -> bool {
        matches!(self, Self::Score)
    }
}

/// One option's rendering payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaiOption {
    /// The answer key the distribution is reported under.
    pub key: String,
    /// The option's description; `None` renders as JSON `null`, which is
    /// what upstream records when a choice criterion carries no text.
    pub description: Option<String>,
}

impl KaiOption {
    /// Builds an option with a description.
    #[must_use]
    pub fn described(key: impl Into<String>, description: impl Into<String>) -> Self {
        Self { key: key.into(), description: Some(description.into()) }
    }

    /// Builds an option whose description renders as JSON `null`.
    #[must_use]
    pub fn bare(key: impl Into<String>) -> Self {
        Self { key: key.into(), description: None }
    }
}

/// A prompt slot that may carry free text or structured data.
///
/// Upstream `_payload` renders strings verbatim and everything else as
/// canonical JSON. Only [`KaiQuestion::new`] callers reach the structured
/// arm; [`KaiQuestion::from_decision`] stays on plain text, because the
/// `OpenCodifier` IR's typed facts have no upstream-trained rendering and
/// inventing one would feed the model an out-of-distribution prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum KaiPayload {
    /// Rendered verbatim.
    Text(String),
    /// Rendered as canonical JSON (sorted keys, compact separators).
    Structured(Value),
}

impl KaiPayload {
    /// Renders the slot the way the contract does.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when structured data carries a
    /// non-finite float: upstream canonical JSON refuses them, and
    /// silently writing `null` would change the prompt.
    pub fn render(&self) -> Result<String, ModelError> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Structured(value) => canonical_json(value),
        }
    }
}

/// A rendered Kai prompt, split the way the encoder tokenizes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaiSegments {
    /// Everything before the first option line.
    pub prefix: String,
    /// One line per option, in option order.
    pub options: Vec<String>,
    /// The query tail after the last option line.
    pub suffix: String,
}

impl KaiSegments {
    /// The full prompt, for hashing and logging.
    #[must_use]
    pub fn prompt(&self) -> String {
        let mut prompt = String::with_capacity(
            self.prefix.len()
                + self.suffix.len()
                + self.options.iter().map(String::len).sum::<usize>(),
        );
        prompt.push_str(&self.prefix);
        for option in &self.options {
            prompt.push_str(option);
        }
        prompt.push_str(&self.suffix);
        prompt
    }
}

/// A Kai decision question: the exact input the contract renders.
#[derive(Debug, Clone, PartialEq)]
pub struct KaiQuestion {
    /// Identifier carried into error messages.
    pub id: String,
    /// The state rendered under `Context:`.
    pub state: KaiPayload,
    /// The question text rendered under `Question:`.
    pub instructions: KaiPayload,
    /// Which primitive this is.
    pub kind: KaiKind,
    /// The candidates, in answer order.
    pub options: Vec<KaiOption>,
}

impl KaiQuestion {
    /// Builds a question, enforcing the upstream cardinality and key rules.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when the option count is outside
    /// `2..=255` (score: `2..=10`), a key is empty, `state` or
    /// `instructions` render empty, or a `noul` question's keys are not
    /// exactly `false` and `true` in that order.
    pub fn new(
        id: impl Into<String>,
        state: KaiPayload,
        instructions: KaiPayload,
        kind: KaiKind,
        options: Vec<KaiOption>,
    ) -> Result<Self, ModelError> {
        let id = id.into();
        let render_nonempty = |payload: &KaiPayload, slot: &str| -> Result<(), ModelError> {
            if payload.render()?.trim().is_empty() {
                return Err(ModelError::ContractViolation {
                    reason: format!("`{id}`: {slot} must be nonempty text or structured data"),
                });
            }
            Ok(())
        };
        render_nonempty(&state, "state")?;
        render_nonempty(&instructions, "instructions")?;

        let (minimum, maximum) = match kind {
            KaiKind::Score => (MIN_CANDIDATES, MAX_SCORE_LEVELS),
            KaiKind::Choice | KaiKind::Noul => (MIN_CANDIDATES, MAX_CANDIDATES),
        };
        if options.len() < minimum || options.len() > maximum {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "`{id}`: {} carries {} options, outside {minimum}..={maximum}",
                    kind.as_str(),
                    options.len()
                ),
            });
        }
        for (index, option) in options.iter().enumerate() {
            if option.key.trim().is_empty() {
                return Err(ModelError::ContractViolation {
                    reason: format!("`{id}`: option {index} has an empty key"),
                });
            }
        }
        if kind == KaiKind::Noul {
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
        Ok(Self { id, state, instructions, kind, options })
    }

    /// Builds the `noul` question a boolean request maps to.
    ///
    /// `false` and `true` are ordered; their descriptions are the ones the
    /// upstream runtime substitutes when a request carries none.
    #[must_use]
    pub fn noul(id: impl Into<String>, state: KaiPayload, instructions: KaiPayload) -> Self {
        Self {
            id: id.into(),
            state,
            instructions,
            kind: KaiKind::Noul,
            options: vec![KaiOption::described("false", "No"), KaiOption::described("true", "Yes")],
        }
    }

    /// Maps an `OpenCodifier` request onto the Kai contract.
    ///
    /// Choice candidates keep their ids and descriptions (an empty
    /// description renders as `null`, matching the upstream fixtures);
    /// boolean questions become `noul` with the default `No`/`Yes`
    /// descriptions; score levels become keys `"0"`… with their labels.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when the state carries typed
    /// facts (no upstream-trained rendering exists for them — project
    /// them into the state text first) or the question breaks the
    /// cardinality rules above.
    pub fn from_decision(state: &State, question: &DecisionQuestion) -> Result<Self, ModelError> {
        if state.facts().next().is_some() {
            return Err(ModelError::ContractViolation {
                reason: "the Kai rung renders text states only: project typed facts into the \
                         state text before rendering"
                    .to_owned(),
            });
        }
        let state = KaiPayload::Text(state.text().to_owned());
        let instructions = KaiPayload::Text(question_text(question)?.to_owned());
        match question {
            DecisionQuestion::Choice(choice) => Self::new(
                choice.id().as_str(),
                state,
                instructions,
                KaiKind::Choice,
                choice
                    .candidates()
                    .iter()
                    .map(|candidate| {
                        let description = candidate.description();
                        if description.trim().is_empty() {
                            KaiOption::bare(candidate.id().as_str())
                        } else {
                            KaiOption::described(candidate.id().as_str(), description)
                        }
                    })
                    .collect(),
            ),
            DecisionQuestion::Boolean(boolean) => {
                Ok(Self::noul(boolean.id().as_str(), state, instructions))
            }
            DecisionQuestion::Score(score) => Self::new(
                score.id().as_str(),
                state,
                instructions,
                KaiKind::Score,
                score
                    .levels()
                    .iter()
                    .enumerate()
                    .map(|(index, level)| KaiOption::described(index.to_string(), level.label()))
                    .collect(),
            ),
            _ => Err(ModelError::ContractViolation {
                reason:
                    "unrecognized decision question shape; the Kai rung has no rendering for it"
                        .to_owned(),
            }),
        }
    }

    /// Renders the prompt segments.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when a payload cannot be rendered
    /// (non-finite float in structured data).
    pub fn segments(&self) -> Result<KaiSegments, ModelError> {
        let prefix = format!(
            "Context:\n{}\n\nTask type: {}\nQuestion:\n{}\nOptions:",
            self.state.render()?,
            self.kind.as_str(),
            self.instructions.render()?
        );
        let options = self
            .options
            .iter()
            .map(|option| {
                Ok(format!(
                    "\n<option>\n{}\n</option>",
                    canonical_json(&serde_json::json!({
                        "key": option.key,
                        "description": option.description,
                    }))?
                ))
            })
            .collect::<Result<Vec<String>, ModelError>>()?;
        Ok(KaiSegments {
            prefix,
            options,
            suffix: "\n\nSelect the single option best supported by the context and \
                     instructions.\nDecision:"
                .to_owned(),
        })
    }

    /// The answer keys, in distribution order.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.options.iter().map(|option| option.key.clone()).collect()
    }
}

/// The question text each IR shape carries.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] for a question shape this crate does
/// not know: the IR enum is `#[non_exhaustive]`, and inventing a rendering
/// for an unrecognized shape would feed the rung an out-of-distribution
/// prompt.
fn question_text(question: &DecisionQuestion) -> Result<&str, ModelError> {
    match question {
        DecisionQuestion::Choice(choice) => Ok(choice.text()),
        DecisionQuestion::Boolean(boolean) => Ok(boolean.text()),
        DecisionQuestion::Score(score) => Ok(score.text()),
        _ => Err(ModelError::ContractViolation {
            reason: "unrecognized decision question shape; the Kai rung has no rendering for it"
                .to_owned(),
        }),
    }
}

/// Canonical JSON: sorted keys, compact separators, non-ASCII literal.
///
/// `serde_json`'s default map is order-sorted, which is exactly the
/// upstream `json.dumps(..., sort_keys=True, separators=(",", ":"))`.
///
/// Finiteness is enforced twice, on purpose. `serde_json::Number` cannot
/// represent a non-finite float at all — `Number::from_f64(NaN)` is
/// `None` — so a `Value` is finite by construction, and the recursive
/// check below re-asserts the upstream `allow_nan=False` invariant for any
/// future representation that could carry one.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the value carries a non-finite
/// number.
pub fn canonical_json(value: &Value) -> Result<String, ModelError> {
    fn reject_non_finite(value: &Value) -> Result<(), ModelError> {
        match value {
            Value::Number(number) => {
                if number.as_f64().is_some_and(f64::is_finite) {
                    Ok(())
                } else {
                    Err(ModelError::ContractViolation {
                        reason: "canonical JSON refuses non-finite numbers".to_owned(),
                    })
                }
            }
            Value::Array(items) => items.iter().try_for_each(reject_non_finite),
            Value::Object(map) => map.values().try_for_each(reject_non_finite),
            Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        }
    }
    reject_non_finite(value)?;
    serde_json::to_string(value).map_err(|error| ModelError::ContractViolation {
        reason: format!("canonical JSON failed: {error}"),
    })
}

/// Text → token ids, the only model-specific operation the contract needs.
pub trait KaiTokenizer: std::fmt::Debug {
    /// Encodes `text` without adding special tokens (the contract adds
    /// none: upstream encodes with `add_special_tokens=False`).
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when the text cannot be encoded.
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError>;
}

/// Byte-level BPE encoder over a `tokenizer.json` vocabulary.
///
/// This is the contract's *oracle*: it loads a `model.vocab` + `merges`
/// pair and reproduces the reference encoder's ids for the frozen fixture
/// set byte for byte (56 of 56 segments, 11 of 11 upstream cases — see
/// `FIXTURE`). It applies no regex pre-tokenization, because the GPT-2
/// split never changes a merge decision for a vocabulary trained under
/// that split; the `tokenizers` feature carries the reference encoder for
/// serving, and its agreement test keeps the two honest.
#[derive(Debug, Clone)]
pub struct BpeTokenizer {
    vocabulary: HashMap<String, i64>,
    ranks: HashMap<(String, String), usize>,
}

impl BpeTokenizer {
    /// Loads a `tokenizer.json` subset: only `model.vocab` and
    /// `model.merges` are read.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when the JSON is not a BPE tokenizer.
    pub fn from_tokenizer_json(json: &str) -> Result<Self, ModelError> {
        let parsed: Value = serde_json::from_str(json)
            .map_err(|error| ModelError::Tokenizer { reason: format!("not JSON: {error}") })?;
        let model = parsed
            .get("model")
            .ok_or_else(|| ModelError::Tokenizer { reason: "no `model` object".to_owned() })?;
        let vocabulary = model
            .get("vocab")
            .ok_or_else(|| ModelError::Tokenizer { reason: "no `model.vocab`".to_owned() })?;
        let vocabulary: HashMap<String, i64> =
            serde_json::from_value(vocabulary.clone()).map_err(|error| ModelError::Tokenizer {
                reason: format!("`model.vocab` is not a token→id map: {error}"),
            })?;
        let merges = model
            .get("merges")
            .ok_or_else(|| ModelError::Tokenizer { reason: "no `model.merges`".to_owned() })?;
        let merges: Vec<Value> = serde_json::from_value(merges.clone()).map_err(|error| {
            ModelError::Tokenizer { reason: format!("`model.merges` is not a list: {error}") }
        })?;
        let mut ranks = HashMap::with_capacity(merges.len());
        for merge in merges {
            let (left, right) = match merge {
                Value::String(pair) => match pair.split_once(' ') {
                    Some((left, right)) => (left.to_owned(), right.to_owned()),
                    None => {
                        return Err(ModelError::Tokenizer {
                            reason: format!("merge `{pair}` is not a space-separated pair"),
                        });
                    }
                },
                Value::Array(pair) if pair.len() == 2 => match (pair[0].as_str(), pair[1].as_str())
                {
                    (Some(left), Some(right)) => (left.to_owned(), right.to_owned()),
                    _ => {
                        return Err(ModelError::Tokenizer {
                            reason: "merge pair is not two strings".to_owned(),
                        });
                    }
                },
                _ => {
                    return Err(ModelError::Tokenizer { reason: "merge is not a pair".to_owned() });
                }
            };
            ranks.insert((left, right), ranks.len());
        }
        Ok(Self { vocabulary, ranks })
    }

    /// The id of one symbol, or a typed error when the vocabulary lacks it.
    fn id_of(&self, symbol: &str) -> Result<i64, ModelError> {
        self.vocabulary.get(symbol).copied().ok_or_else(|| ModelError::Tokenizer {
            reason: format!("vocabulary has no id for {symbol:?}"),
        })
    }
}

/// The byte→unicode alphabet byte-level BPE tokenizes through.
fn byte_alphabet() -> [char; 256] {
    let mut alphabet = ['\0'; 256];
    let mut next = 256u32;
    for byte in 0..=255u8 {
        let printable = (33..=126).contains(&byte)
            || (161..=172).contains(&byte)
            || (174..=255).contains(&byte);
        alphabet[byte as usize] = if printable {
            byte as char
        } else {
            let mapped = next;
            next += 1;
            char::from_u32(mapped).unwrap_or('\u{fffd}')
        };
    }
    alphabet
}

impl KaiTokenizer for BpeTokenizer {
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError> {
        let alphabet = byte_alphabet();
        let mut symbols: Vec<String> =
            text.bytes().map(|byte| alphabet[byte as usize].to_string()).collect();
        if symbols.is_empty() {
            return Ok(Vec::new());
        }

        // Best-first BPE by merge rank. A binary heap of
        // `(rank, position, version)` keeps this O(n log n): `version`
        // invalidates entries whose position has since merged away, and a
        // re-read rank on pop discards entries whose *pair* has changed
        // (the merge that changed it re-pushed the new pair). Both guards
        // are required — a stale rank would apply a merge out of order.
        let mut versions = vec![0u64; symbols.len()];
        let mut alive = vec![true; symbols.len()];
        let mut queue = std::collections::BinaryHeap::new();
        let rank_of = |symbols: &[String], left: usize, right: usize| -> Option<usize> {
            self.ranks.get(&(symbols[left].clone(), symbols[right].clone())).copied()
        };
        for (position, version) in versions.iter().enumerate().take(symbols.len() - 1) {
            if let Some(rank) = rank_of(&symbols, position, position + 1) {
                queue.push(std::cmp::Reverse((rank, position, *version)));
            }
        }
        while let Some(std::cmp::Reverse((rank, position, version))) = queue.pop() {
            if !alive[position] || versions[position] != version {
                continue;
            }
            let mut next = position + 1;
            while next < symbols.len() && !alive[next] {
                next += 1;
            }
            if next >= symbols.len() {
                continue;
            }
            // The pair may have changed since this entry was pushed; only
            // the merge that created the current pair holds its real rank.
            if rank_of(&symbols, position, next) != Some(rank) {
                continue;
            }
            let joined = format!("{}{}", symbols[position], symbols[next]);
            self.id_of(&joined)?;
            symbols[position] = joined;
            alive[next] = false;
            versions[position] += 1;
            // Nearest surviving symbol to the left: the merge changed that
            // pair, so its (possibly new) merge must be re-queued.
            let mut previous = position;
            while previous > 0 {
                previous -= 1;
                if alive[previous] {
                    break;
                }
            }
            if previous < position
                && let Some(neighbour_rank) = rank_of(&symbols, previous, position)
            {
                queue.push(std::cmp::Reverse((neighbour_rank, previous, versions[previous])));
            }
            let mut following = next + 1;
            while following < symbols.len() && !alive[following] {
                following += 1;
            }
            if following < symbols.len()
                && let Some(neighbour_rank) = rank_of(&symbols, position, following)
            {
                queue.push(std::cmp::Reverse((neighbour_rank, position, versions[position])));
            }
        }

        let mut ids = Vec::with_capacity(symbols.len());
        for (position, symbol) in symbols.iter().enumerate() {
            if alive[position] {
                ids.push(self.id_of(symbol)?);
            }
        }
        Ok(ids)
    }
}

/// One encoded question: the tensors the graph consumes, and the
/// invariants that make its pointer readout safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaiEncoding {
    /// Prompt token ids, no special tokens added.
    pub ids: Vec<i64>,
    /// Absolute token index of each option's last token.
    pub option_pos: Vec<usize>,
    /// Absolute token index of the query token (`Decision:`'s colon).
    pub answer_pos: usize,
    /// Answer keys, in pointer order.
    pub keys: Vec<String>,
    /// Which primitive this encodes.
    pub kind: KaiKind,
}

impl KaiEncoding {
    /// Encodes `question` under the contract.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] for an encoding failure, and
    /// [`ModelError::PromptTooLong`] when the prompt exceeds
    /// `max_input_tokens` — the contract refuses, never truncates.
    pub fn encode(
        question: &KaiQuestion,
        tokenizer: &dyn KaiTokenizer,
        max_input_tokens: usize,
    ) -> Result<Self, ModelError> {
        let segments = question.segments()?;
        let mut ids = tokenizer.encode(&segments.prefix)?;
        if ids.is_empty() {
            return Err(ModelError::Tokenizer {
                reason: format!("`{}`: the prefix encoded to no tokens", question.id),
            });
        }
        let mut option_pos = Vec::with_capacity(segments.options.len());
        for (index, option) in segments.options.iter().enumerate() {
            let part = tokenizer.encode(option)?;
            if part.is_empty() {
                return Err(ModelError::Tokenizer {
                    reason: format!("`{}`: option {index} encoded to no tokens", question.id),
                });
            }
            ids.extend(part);
            option_pos.push(ids.len() - 1);
        }
        let tail = tokenizer.encode(&segments.suffix)?;
        if tail.is_empty() {
            return Err(ModelError::Tokenizer {
                reason: format!("`{}`: the query suffix encoded to no tokens", question.id),
            });
        }
        ids.extend(tail);
        let encoding = Self {
            answer_pos: ids.len() - 1,
            ids,
            option_pos,
            keys: question.keys(),
            kind: question.kind,
        };
        if encoding.ids.len() > max_input_tokens {
            return Err(ModelError::PromptTooLong {
                id: question.id.clone(),
                tokens: encoding.ids.len(),
                max_tokens: max_input_tokens,
            });
        }
        encoding.validate()?;
        Ok(encoding)
    }

    /// Asserts the pointer invariants the §6.5 incident is made of.
    ///
    /// One pointer per candidate, distinct and strictly increasing, every
    /// pointer inside the prompt and strictly below the query position,
    /// and the query position last. A violation is an error: a pointer
    /// readout fed a mis-anchored index does not degrade gracefully, it
    /// produces confidently wrong scores.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] describing which invariant broke.
    pub fn validate(&self) -> Result<(), ModelError> {
        let count = self.keys.len();
        if self.option_pos.len() != count {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "pointer count {} must equal the candidate count {count}",
                    self.option_pos.len()
                ),
            });
        }
        if !(MIN_CANDIDATES..=MAX_CANDIDATES).contains(&count) {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "candidate count {count} outside {MIN_CANDIDATES}..={MAX_CANDIDATES}"
                ),
            });
        }
        let mut distinct = self.option_pos.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() != count {
            return Err(ModelError::ContractViolation {
                reason: "option pointers must be distinct".to_owned(),
            });
        }
        if !self.option_pos.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(ModelError::ContractViolation {
                reason: "option pointers must be strictly increasing".to_owned(),
            });
        }
        if self.answer_pos != self.ids.len().saturating_sub(1) {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "answer_pos {} must be the last token index {}",
                    self.answer_pos,
                    self.ids.len() - 1
                ),
            });
        }
        if let Some(position) =
            self.option_pos.iter().find(|position| **position >= self.answer_pos)
        {
            return Err(ModelError::ContractViolation {
                reason: format!(
                    "option pointer {position} must be below answer_pos {}",
                    self.answer_pos
                ),
            });
        }
        Ok(())
    }
}

/// Widens a validated token position to the graph's index element type.
///
/// Token positions come from `Vec<usize>` index math and stay far below
/// `i64::MAX`; the cast cannot wrap.
#[must_use]
fn cast_position(position: usize) -> i64 {
    #[allow(clippy::cast_possible_wrap)]
    {
        position as i64
    }
}

/// A right-padded batch, in the shape the fused graph consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaiBatch {
    /// `[B, L]` token ids, [`PAD_ID`] on the right.
    pub input_ids: Vec<i64>,
    /// `[B, L]` mask, 1 for real tokens.
    pub attention_mask: Vec<i64>,
    /// `[B]` query positions.
    pub answer_pos: Vec<i64>,
    /// `[B, K]` option endpoints, zero-filled past a row's candidate count.
    pub option_pos: Vec<i64>,
    /// Rows in the batch.
    pub batch: usize,
    /// Padded sequence length.
    pub length: usize,
    /// Padded candidate width.
    pub width: usize,
}

impl KaiBatch {
    /// Pads `encodings` into one batch.
    ///
    /// Upstream's `collate` rounds the length up to a multiple of 8 and its
    /// `parity.py` pads with id 0; the fused graph derives position ids
    /// from the length and never reads a masked lane, so this batch uses
    /// the parity convention without the rounding.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] for an empty batch or rows whose
    /// candidate counts disagree with their key lists.
    pub fn collate(encodings: &[&KaiEncoding]) -> Result<Self, ModelError> {
        let first = encodings.first().ok_or_else(|| ModelError::ContractViolation {
            reason: "cannot collate an empty batch".to_owned(),
        })?;
        let batch = encodings.len();
        let length =
            encodings.iter().map(|encoding| encoding.ids.len()).max().unwrap_or(first.ids.len());
        let width =
            encodings.iter().map(|encoding| encoding.keys.len()).max().unwrap_or(first.keys.len());
        let mut input_ids = vec![PAD_ID; batch * length];
        let mut attention_mask = vec![0; batch * length];
        let mut answer_pos = vec![0; batch];
        let mut option_pos = vec![0; batch * width];
        for (row, encoding) in encodings.iter().enumerate() {
            if encoding.keys.len() != encoding.option_pos.len() {
                return Err(ModelError::ContractViolation {
                    reason: format!(
                        "row {row}: {} keys but {} option pointers",
                        encoding.keys.len(),
                        encoding.option_pos.len()
                    ),
                });
            }
            for (column, id) in encoding.ids.iter().enumerate() {
                input_ids[row * length + column] = *id;
                attention_mask[row * length + column] = 1;
            }
            answer_pos[row] = cast_position(encoding.answer_pos);
            for (column, position) in encoding.option_pos.iter().enumerate() {
                option_pos[row * width + column] = cast_position(*position);
            }
        }
        Ok(Self { input_ids, attention_mask, answer_pos, option_pos, batch, length, width })
    }

    /// Builds the named tensors the backend consumes.
    ///
    /// Index tensors cross as integral-valued `f32`, the transport
    /// convention `opencodifier_runtime::kai` documents.
    ///
    /// # Errors
    ///
    /// [`ModelError::ContractViolation`] when a tensor shape is rejected.
    pub fn to_tensors(&self) -> Result<BTreeMap<String, DenseTensor>, ModelError> {
        let build =
            |name: &str, shape: Vec<usize>, data: &[i64]| -> Result<DenseTensor, ModelError> {
                #[allow(clippy::cast_precision_loss)] // token ids and positions stay far below 2^24
                let floats: Vec<f32> = data.iter().map(|value| *value as f32).collect();
                DenseTensor::new(shape, floats).map_err(|error| ModelError::ContractViolation {
                    reason: format!("`{name}`: {error}"),
                })
            };
        Ok(BTreeMap::from([
            (
                INPUT_IDS.to_owned(),
                build(INPUT_IDS, vec![self.batch, self.length], &self.input_ids)?,
            ),
            (
                ATTENTION_MASK.to_owned(),
                build(ATTENTION_MASK, vec![self.batch, self.length], &self.attention_mask)?,
            ),
            (ANSWER_POS.to_owned(), build(ANSWER_POS, vec![self.batch], &self.answer_pos)?),
            (
                OPTION_POS.to_owned(),
                build(OPTION_POS, vec![self.batch, self.width], &self.option_pos)?,
            ),
        ]))
    }
}

/// Additive logit offsets per score-level count, from upstream
/// `score_bias.json` (`format: dev2-score-bias-v1`).
///
/// Only the 5-level rung is fitted: a float64 Newton fit of
/// `−log softmax(z + b)` over 400 ledger rows, stratum-weighted by
/// `n^−0.5`, shifted to mean zero and rounded to 6 decimals. `b_0` is
/// pinned to 0 by the objective, which the stored table reproduces.
pub const SCORE_BIAS: &[(usize, &[f64])] =
    &[(5, &[0.039_188, 0.203_049, 0.079_362, -0.151_62, -0.169_979])];

/// The fitted offsets for a level count, if the table carries one.
#[must_use]
pub fn score_bias(levels: usize) -> Option<&'static [f64]> {
    SCORE_BIAS.iter().find(|(count, _)| *count == levels).map(|(_, offsets)| *offsets)
}

/// Applies the fitted score bias to logits, in place.
///
/// Returns whether an offset vector was found and applied. Called only
/// for score questions — the bias belongs to that rung's calibration,
/// never to choice or noul logits.
pub fn apply_score_bias(logits: &mut [f64]) -> bool {
    match score_bias(logits.len()) {
        Some(offsets) if offsets.len() == logits.len() => {
            for (logit, offset) in logits.iter_mut().zip(offsets.iter()) {
                *logit += offset;
            }
            true
        }
        _ => false,
    }
}

/// Softmax in `f64`, max-subtracted, with **no** garbage fallback.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] for empty input or a non-finite
/// logit. This is deliberately stricter than
/// `opencodifier_engine::softmax`, which degrades non-finite input to a
/// uniform distribution: a calibration path must never quietly invent a
/// confident-looking distribution out of garbage logits.
pub fn softmax(logits: &[f64]) -> Result<Vec<f64>, ModelError> {
    if logits.is_empty() {
        return Err(ModelError::ContractViolation {
            reason: "softmax needs at least one logit".to_owned(),
        });
    }
    if let Some(bad) = logits.iter().position(|logit| !logit.is_finite()) {
        return Err(ModelError::ContractViolation {
            reason: format!("softmax logit {bad} is not finite"),
        });
    }
    let maximum = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exponentials: Vec<f64> = logits.iter().map(|logit| (logit - maximum).exp()).collect();
    let sum: f64 = exponentials.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(ModelError::ContractViolation {
            reason: "softmax exponentials do not sum".to_owned(),
        });
    }
    Ok(exponentials.iter().map(|value| value / sum).collect())
}

/// Turns graph logits into the distribution the runtime reports.
///
/// Order of operations is the upstream one: score bias first (score
/// questions, fitted level counts only), then `f64` softmax at
/// temperature 1. Output is in key order, so a `noul` distribution reads
/// `[P(false), P(true)]`.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when the logits are empty, carry a
/// non-finite value, or disagree with the key count.
pub fn probabilities(
    kind: KaiKind,
    keys: &[String],
    logits: &[f64],
) -> Result<Vec<f64>, ModelError> {
    if logits.len() != keys.len() {
        return Err(ModelError::ContractViolation {
            reason: format!("{} logits must match {} keys", logits.len(), keys.len()),
        });
    }
    let mut scores = logits.to_vec();
    if kind.takes_score_bias() {
        apply_score_bias(&mut scores);
    }
    softmax(&scores)
}

/// The Hugging Face `tokenizers` reference encoder, behind the
/// `tokenizers` feature.
///
/// This is the encoder a serving deployment loads from the model
/// directory's own `tokenizer.json`. It exists here so the built-in
/// [`BpeTokenizer`] oracle can be held against it: [`FIXTURE`]'s
/// agreement test asserts both produce the same ids for every frozen
/// segment, so the feature-free contract never drifts from the reference.
#[cfg(feature = "tokenizers")]
#[derive(Debug)]
pub struct HfTokenizer {
    inner: tokenizers::Tokenizer,
}

#[cfg(feature = "tokenizers")]
impl HfTokenizer {
    /// Loads a `tokenizer.json`.
    ///
    /// # Errors
    ///
    /// [`ModelError::Tokenizer`] when the file is not a loadable tokenizer.
    pub fn from_tokenizer_json(json: &str) -> Result<Self, ModelError> {
        use std::str::FromStr;
        let inner = tokenizers::Tokenizer::from_str(json)
            .map_err(|error| ModelError::Tokenizer { reason: format!("load failed: {error}") })?;
        Ok(Self { inner })
    }
}

#[cfg(feature = "tokenizers")]
impl KaiTokenizer for HfTokenizer {
    fn encode(&self, text: &str) -> Result<Vec<i64>, ModelError> {
        let encoding = self
            .inner
            .encode(text, false)
            .map_err(|error| ModelError::Tokenizer { reason: format!("encode failed: {error}") })?;
        Ok(encoding.get_ids().iter().map(|id| i64::from(*id)).collect())
    }
}

/// Scores a batch through any backend and reports one distribution per row.
///
/// This is the rung's serving entry point: render → encode → collate is
/// the caller's business, everything from tensors to probabilities is
/// here. Softmax happens in Rust (D7); the backend returns raw logits.
///
/// # Errors
///
/// [`ModelError::ContractViolation`] when a tensor shape breaks the
/// contract, and a flattened backend error message when the backend
/// itself fails — [`ModelError`] never wraps a backend error opaquely.
pub fn score_batch(
    backend: &dyn opencodifier_runtime::InferenceBackend,
    encodings: &[&KaiEncoding],
) -> Result<Vec<Vec<f64>>, ModelError> {
    let batch = KaiBatch::collate(encodings)?;
    let outputs = backend.infer(&batch.to_tensors()?).map_err(|error| {
        ModelError::ContractViolation { reason: format!("backend failed: {error}") }
    })?;
    let logits = outputs.get(LOGITS_OUTPUT).ok_or_else(|| ModelError::ContractViolation {
        reason: format!("backend did not return `{LOGITS_OUTPUT}`"),
    })?;
    if logits.shape() != [batch.batch, batch.width] {
        return Err(ModelError::ContractViolation {
            reason: format!(
                "`{LOGITS_OUTPUT}` must have shape [{}, {}], got {:?}",
                batch.batch,
                batch.width,
                logits.shape()
            ),
        });
    }
    let mut distributions = Vec::with_capacity(batch.batch);
    for (row, encoding) in encodings.iter().enumerate() {
        let row_logits: Vec<f64> = logits
            .row(row)
            .ok_or_else(|| ModelError::ContractViolation { reason: format!("row {row} missing") })?
            .iter()
            .map(|value| f64::from(*value))
            .take(encoding.keys.len())
            .collect();
        distributions.push(probabilities(encoding.kind, &encoding.keys, &row_logits)?);
    }
    Ok(distributions)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]

    use super::*;
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, ScoreLevel, ScoreQuestion,
    };
    use opencodifier_runtime::MockInferenceBackend;

    const UPSTREAM_SUFFIX: &str =
        "\n\nSelect the single option best supported by the context and instructions.\nDecision:";

    #[test]
    fn renders_the_upstream_template_byte_for_byte() {
        let question = KaiQuestion::new(
            "payouts/is_urgent",
            KaiPayload::Text("Help! My payouts have been failing for 3 days! ".to_owned()),
            KaiPayload::Text("Does this convey urgency?".to_owned()),
            KaiKind::Noul,
            vec![KaiOption::described("false", "No"), KaiOption::described("true", "Yes")],
        )
        .unwrap();
        let segments = question.segments().unwrap();
        assert_eq!(
            segments.prefix,
            "Context:\nHelp! My payouts have been failing for 3 days! \n\nTask type: noul\n\
             Question:\nDoes this convey urgency?\nOptions:"
        );
        assert_eq!(
            segments.options,
            vec![
                "\n<option>\n{\"description\":\"No\",\"key\":\"false\"}\n</option>",
                "\n<option>\n{\"description\":\"Yes\",\"key\":\"true\"}\n</option>",
            ]
        );
        assert_eq!(segments.suffix, UPSTREAM_SUFFIX);
        assert_eq!(question.keys(), vec!["false", "true"]);
    }

    #[test]
    fn choice_descriptions_render_as_null_when_absent() {
        let question = KaiQuestion::new(
            "payouts/team",
            KaiPayload::Text("state".to_owned()),
            KaiPayload::Text("Which team?".to_owned()),
            KaiKind::Choice,
            vec![KaiOption::bare("billing"), KaiOption::described("sales", "Sells things")],
        )
        .unwrap();
        let segments = question.segments().unwrap();
        assert_eq!(
            segments.options[0],
            "\n<option>\n{\"description\":null,\"key\":\"billing\"}\n</option>"
        );
        assert_eq!(
            segments.options[1],
            "\n<option>\n{\"description\":\"Sells things\",\"key\":\"sales\"}\n</option>"
        );
    }

    #[test]
    fn canonical_json_sorts_keys_and_refuses_non_finite_numbers() {
        let value = serde_json::json!({"key": "b", "description": "d", "nested": {"z": 1, "a": 2}});
        assert_eq!(
            canonical_json(&value).unwrap(),
            r#"{"description":"d","key":"b","nested":{"a":2,"z":1}}"#
        );

        let literal = canonical_json(&serde_json::json!({"description": "café"})).unwrap();
        assert!(literal.contains("café"), "{literal}");

        // The recursive refusal is defensive by construction: serde_json
        // rejects overflowing literals outright ("number out of range")
        // and `Number::from_f64` refuses non-finites, so no `Value` can
        // reach the check carrying one. Proven empirically 2026-10-07;
        // the arm stays as the re-assertion of the upstream invariant.

        // Upstream refuses `allow_nan` output; serde_json cannot even
        // represent one, which is the structural half of the guarantee.
        assert!(
            serde_json::Number::from_f64(f64::NAN).is_none()
                && serde_json::Number::from_f64(f64::INFINITY).is_none(),
            "a serde_json Number must never carry a non-finite float"
        );
    }

    #[test]
    fn cardinality_and_key_rules_are_enforced() {
        let state = KaiPayload::Text("state".to_owned());
        let instructions = KaiPayload::Text("question".to_owned());
        let one = vec![KaiOption::bare("only")];
        let error =
            KaiQuestion::new("q", state.clone(), instructions.clone(), KaiKind::Choice, one)
                .unwrap_err();
        assert!(error.to_string().contains("outside 2..=255"), "{error}");

        let eleven =
            (0..11).map(|index| KaiOption::described(index.to_string(), "level")).collect();
        let error =
            KaiQuestion::new("q", state.clone(), instructions.clone(), KaiKind::Score, eleven)
                .unwrap_err();
        assert!(error.to_string().contains("outside 2..=10"), "{error}");

        let swapped =
            vec![KaiOption::described("true", "Yes"), KaiOption::described("false", "No")];
        let error =
            KaiQuestion::new("q", state.clone(), instructions.clone(), KaiKind::Noul, swapped)
                .unwrap_err();
        assert!(error.to_string().contains("false,true"), "{error}");

        let empty_key = vec![KaiOption::bare("  "), KaiOption::bare("b")];
        let error =
            KaiQuestion::new("q", state, instructions, KaiKind::Choice, empty_key).unwrap_err();
        assert!(error.to_string().contains("empty key"), "{error}");
    }

    /// The fixture: ids, pointers, and probabilities, in one place.
    struct FixtureCase {
        kind: KaiKind,
        state: String,
        instructions: String,
        criteria: Value,
        keys: Vec<String>,
        ids: Vec<i64>,
        option_pos: Vec<usize>,
        answer_pos: usize,
        probabilities: Vec<f64>,
        max_input_tokens: usize,
    }

    fn fixture_cases() -> Vec<FixtureCase> {
        let parsed: Value = serde_json::from_str(FIXTURE).unwrap();
        let max_input_tokens = parsed["max_input_tokens"].as_u64().unwrap() as usize;
        let cases = parsed["cases"].as_array().unwrap();
        cases
            .iter()
            .map(|case| FixtureCase {
                kind: match case["kind"].as_str().unwrap() {
                    "choice" => KaiKind::Choice,
                    "noul" => KaiKind::Noul,
                    _ => KaiKind::Score,
                },
                state: case["state"].as_str().unwrap().to_owned(),
                instructions: case["instructions"].as_str().unwrap().to_owned(),
                criteria: case["criteria"].clone(),
                keys: case["keys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|key| key.as_str().unwrap().to_owned())
                    .collect(),
                ids: case["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_i64().unwrap())
                    .collect(),
                option_pos: case["option_pos"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|position| position.as_u64().unwrap() as usize)
                    .collect(),
                answer_pos: case["answer_pos"].as_u64().unwrap() as usize,
                probabilities: case["probabilities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|probability| probability.as_f64().unwrap())
                    .collect(),
                max_input_tokens,
            })
            .collect()
    }

    fn fixture_question(case: &FixtureCase) -> KaiQuestion {
        // Options are built in recorded `keys` order: a JSON object's
        // iteration order is sorted, and the fixture's choice case proves
        // that is not the answer order.
        let options: Vec<KaiOption> = case
            .keys
            .iter()
            .map(|key| {
                if case.kind == KaiKind::Score {
                    let index: usize = key.parse().unwrap();
                    KaiOption::described(
                        key.clone(),
                        case.criteria.as_array().unwrap()[index].as_str().unwrap(),
                    )
                } else {
                    KaiOption {
                        key: key.clone(),
                        description: case.criteria[key].as_str().map(str::to_owned),
                    }
                }
            })
            .collect();
        KaiQuestion::new(
            "fixture",
            KaiPayload::Text(case.state.clone()),
            KaiPayload::Text(case.instructions.clone()),
            case.kind,
            options,
        )
        .unwrap()
    }

    /// The gate of the arm: render + tokenize + pointer math must
    /// reproduce the recorded ids, option endpoints, and query position
    /// exactly, and the frozen score bias must sit in the fixture.
    #[test]
    fn fixture_cases_reproduce_ids_and_pointers_exactly() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        for case in fixture_cases() {
            let question = fixture_question(&case);
            let encoding =
                KaiEncoding::encode(&question, &tokenizer, case.max_input_tokens).unwrap();
            assert_eq!(encoding.ids, case.ids, "ids for {} / {:?}", case.state, case.keys);
            assert_eq!(encoding.option_pos, case.option_pos, "option_pos for {:?}", case.keys);
            assert_eq!(encoding.answer_pos, case.answer_pos, "answer_pos for {:?}", case.keys);
            assert_eq!(encoding.keys, case.keys);
            encoding.validate().unwrap();
        }
    }

    #[test]
    fn fixture_carries_the_fitted_five_level_bias() {
        let parsed: Value = serde_json::from_str(FIXTURE).unwrap();
        let bias = parsed["score_bias"]["5"].as_array().unwrap();
        let offsets: Vec<f64> = bias.iter().map(|value| value.as_f64().unwrap()).collect();
        assert_eq!(offsets, SCORE_BIAS[0].1.to_vec());
        let mean = offsets.iter().sum::<f64>() / offsets.len() as f64;
        assert!(mean.abs() < 1e-6, "the fit is mean-zero, got {mean}");
    }

    /// Bias-then-softmax composition, checked against the recorded
    /// probabilities. The logits are *reconstructed* from them
    /// (`ln p − bias`, shift-invariant), so this proves the order of
    /// operations, the level-count gating, and the key ordering — it does
    /// not prove the graph produces these logits, which is the
    /// compute-host parity run's job.
    #[test]
    fn bias_then_softmax_reproduces_the_recorded_probabilities() {
        for case in fixture_cases() {
            let offsets = if case.kind.takes_score_bias() {
                score_bias(case.probabilities.len())
                    .expect("the fixture carries a fit for this rung")
                    .to_vec()
            } else {
                vec![0.0; case.probabilities.len()]
            };
            // Logits reconstructed as `ln p − b`: softmax is shift
            // invariant, so undoing the bias recovers a logit vector that
            // re-renders the recorded distribution exactly.
            let logits: Vec<f64> = case
                .probabilities
                .iter()
                .zip(offsets)
                .map(|(probability, offset)| probability.ln() - offset)
                .collect();
            let distribution = probabilities(case.kind, &case.keys, &logits).unwrap();
            for (produced, recorded) in distribution.iter().zip(case.probabilities.iter()) {
                assert!(
                    (produced - recorded).abs() < 1e-4,
                    "{:?}: produced {produced} vs recorded {recorded}",
                    case.keys
                );
            }
        }
    }

    #[test]
    fn score_bias_applies_only_to_the_fitted_level_count() {
        let mut three = vec![0.0_f64; 3];
        assert!(!apply_score_bias(&mut three), "no fit exists for 3 levels");
        assert_eq!(three, vec![0.0; 3]);
        assert!(score_bias(5).is_some());
        assert!(score_bias(4).is_none());
        assert!(score_bias(6).is_none());

        let mut five = vec![1.0_f64; 5];
        assert!(apply_score_bias(&mut five));
        for (produced, expected) in five.iter().zip(SCORE_BIAS[0].1.iter()) {
            assert_eq!(*produced, 1.0 + expected);
        }
        // Choice logits never take the bias, whatever their length.
        let mut choice = vec![1.0_f64; 5];
        assert!(!KaiKind::Choice.takes_score_bias());
        assert!(!KaiKind::Noul.takes_score_bias());
        let _ = &mut choice;
    }

    #[test]
    fn softmax_is_strict_about_garbage() {
        assert_eq!(softmax(&[0.0, 0.0]).unwrap(), vec![0.5, 0.5]);
        assert!((softmax(&[1000.0, 1000.0]).unwrap()).iter().all(|value| value.is_finite()));
        let error = softmax(&[]).unwrap_err();
        assert_eq!(error.code(), "model.contract_violation");
        let error = softmax(&[0.0, f64::NAN]).unwrap_err();
        assert!(error.to_string().contains("not finite"), "{error}");
        let error = softmax(&[f64::NEG_INFINITY, 0.0]).unwrap_err();
        assert!(error.to_string().contains("not finite"), "{error}");
    }

    /// The §6.5 incident, as a test: question-relative pointers and a
    /// shuffled pointer list are refused, never read.
    #[test]
    fn pointer_violations_are_errors_not_readouts() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let case = &fixture_cases()[0];
        let question = fixture_question(case);
        let encoding = KaiEncoding::encode(&question, &tokenizer, case.max_input_tokens).unwrap();
        encoding.validate().unwrap();

        let question_relative: Vec<usize> = encoding.option_pos.iter().rev().copied().collect();
        let broken = KaiEncoding {
            ids: encoding.ids.clone(),
            option_pos: question_relative,
            answer_pos: encoding.answer_pos,
            keys: encoding.keys.clone(),
            kind: encoding.kind,
        };
        let error = broken.validate().unwrap_err();
        assert_eq!(error.code(), "model.contract_violation");
        assert!(error.to_string().contains("strictly increasing"), "{error}");

        let past_the_query = KaiEncoding {
            ids: encoding.ids.clone(),
            option_pos: vec![encoding.answer_pos, encoding.answer_pos + 1],
            answer_pos: encoding.answer_pos,
            keys: encoding.keys.clone(),
            kind: encoding.kind,
        };
        let error = past_the_query.validate().unwrap_err();
        assert!(error.to_string().contains("below answer_pos"), "{error}");

        let miscounted = KaiEncoding {
            ids: encoding.ids.clone(),
            option_pos: encoding.option_pos[..1].to_vec(),
            answer_pos: encoding.answer_pos,
            keys: encoding.keys.clone(),
            kind: encoding.kind,
        };
        let error = miscounted.validate().unwrap_err();
        assert!(error.to_string().contains("candidate count"), "{error}");

        let wrong_query = KaiEncoding {
            ids: encoding.ids.clone(),
            option_pos: encoding.option_pos.clone(),
            answer_pos: encoding.answer_pos - 1,
            keys: encoding.keys.clone(),
            kind: encoding.kind,
        };
        let error = wrong_query.validate().unwrap_err();
        assert!(error.to_string().contains("last token index"), "{error}");
    }

    #[test]
    fn over_budget_prompts_are_refused_never_truncated() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let case = &fixture_cases()[0];
        let question = fixture_question(case);
        let error = KaiEncoding::encode(&question, &tokenizer, 8).unwrap_err();
        assert_eq!(error.code(), "model.prompt_too_long");
        assert!(error.to_string().contains("above the 8 token budget"), "{error}");
        // The same prompt fits under the real budget.
        let encoding = KaiEncoding::encode(&question, &tokenizer, MAX_INPUT_TOKENS).unwrap();
        assert_eq!(encoding.ids, case.ids);
    }

    #[test]
    fn collate_pads_right_and_zero_fills_the_option_width() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let cases = fixture_cases();
        let encodings: Vec<KaiEncoding> = cases
            .iter()
            .map(|case| {
                KaiEncoding::encode(&fixture_question(case), &tokenizer, case.max_input_tokens)
                    .unwrap()
            })
            .collect();
        let references: Vec<&KaiEncoding> = encodings.iter().collect();
        let batch = KaiBatch::collate(&references).unwrap();
        assert_eq!(batch.batch, 3);
        assert_eq!(batch.width, 5, "the widest row is the 5-level score case");
        assert!(batch.length >= encodings.iter().map(|encoding| encoding.ids.len()).max().unwrap());

        let tensors = batch.to_tensors().unwrap();
        let ids = &tensors[INPUT_IDS];
        assert_eq!(ids.shape(), &[3, batch.length]);
        // Row 0 (noul, 2 candidates): the mask stops at its length, and the
        // option lanes past 2 are zero-filled.
        let mask = &tensors[ATTENTION_MASK];
        let row_length = encodings[0].ids.len();
        assert_eq!(mask.row(0).unwrap()[..row_length], vec![1.0; row_length][..]);
        assert!(mask.row(0).unwrap()[row_length..].iter().all(|value| *value == 0.0));
        let options = &tensors[OPTION_POS];
        assert_eq!(options.shape(), &[3, 5]);
        let row = options.row(0).unwrap();
        assert_eq!(
            &row[..2],
            &[encodings[0].option_pos[0] as f32, encodings[0].option_pos[1] as f32]
        );
        assert!(row[2..].iter().all(|value| *value == 0.0));
        assert_eq!(tensors[ANSWER_POS].data()[0] as usize, encodings[0].answer_pos);

        let error = KaiBatch::collate(&[]).unwrap_err();
        assert!(error.to_string().contains("empty batch"), "{error}");
    }

    /// Full serving path over a scripted backend: tensors in, distributions
    /// out, with the logits inverted from the recorded probabilities.
    #[test]
    fn score_batch_returns_one_distribution_per_row() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let cases = fixture_cases();
        let encodings: Vec<KaiEncoding> = cases
            .iter()
            .map(|case| {
                KaiEncoding::encode(&fixture_question(case), &tokenizer, case.max_input_tokens)
                    .unwrap()
            })
            .collect();
        let references: Vec<&KaiEncoding> = encodings.iter().collect();
        let width = 5;
        let mut logits = Vec::new();
        for case in &cases {
            let offsets = score_bias(case.probabilities.len())
                .map_or_else(|| vec![0.0; case.probabilities.len()], <[f64]>::to_vec);
            let row: Vec<f64> = case
                .probabilities
                .iter()
                .zip(offsets)
                .map(|(probability, offset)| probability.ln() - offset)
                .collect();
            logits.extend(row.iter().map(|value| *value as f32));
            logits.extend(std::iter::repeat_n(0.0_f32, width - case.probabilities.len()));
        }
        let backend = MockInferenceBackend::new(
            opencodifier_runtime::kai::MODEL_ID,
            BTreeMap::from([(
                LOGITS_OUTPUT.to_owned(),
                DenseTensor::new(vec![cases.len(), width], logits).unwrap(),
            )]),
        );
        let distributions = score_batch(&backend, &references).unwrap();
        assert_eq!(distributions.len(), cases.len());
        for (distribution, case) in distributions.iter().zip(cases.iter()) {
            assert_eq!(distribution.len(), case.keys.len());
            for (produced, recorded) in distribution.iter().zip(case.probabilities.iter()) {
                assert!((produced - recorded).abs() < 1e-4, "{produced} vs {recorded}");
            }
        }
    }

    #[test]
    fn a_backend_that_returns_the_wrong_shape_is_refused() {
        let tokenizer = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let case = &fixture_cases()[0];
        let encoding =
            KaiEncoding::encode(&fixture_question(case), &tokenizer, case.max_input_tokens)
                .unwrap();
        let backend = MockInferenceBackend::new(
            "m",
            BTreeMap::from([(
                LOGITS_OUTPUT.to_owned(),
                DenseTensor::new(vec![1, 7], vec![0.0; 7]).unwrap(),
            )]),
        );
        let error = score_batch(&backend, &[&encoding]).unwrap_err();
        assert!(error.to_string().contains("must have shape [1, 2]"), "{error}");
    }

    #[test]
    fn maps_the_decision_ir_onto_the_contract() {
        let state = State::from_text("prove a large theorem");
        let choice = ChoiceQuestion::new(
            "model",
            "Which model should run deep reasoning?",
            vec![
                Candidate::new("local-qwen", "fast general coding").unwrap(),
                Candidate::new("local-glm", "").unwrap(),
            ],
        )
        .unwrap();
        let question =
            KaiQuestion::from_decision(&state, &DecisionQuestion::Choice(choice)).unwrap();
        assert_eq!(question.kind, KaiKind::Choice);
        assert_eq!(question.state, KaiPayload::Text("prove a large theorem".to_owned()));
        assert_eq!(
            question.options,
            vec![
                KaiOption::described("local-qwen", "fast general coding"),
                KaiOption::bare("local-glm"),
            ]
        );

        let boolean = BooleanQuestion::new("urgent", "Does this convey urgency?").unwrap();
        let question =
            KaiQuestion::from_decision(&state, &DecisionQuestion::Boolean(boolean)).unwrap();
        assert_eq!(question.kind, KaiKind::Noul);
        assert_eq!(question.keys(), vec!["false", "true"]);
        assert_eq!(question.options[1].description, Some("Yes".to_owned()));

        let score = ScoreQuestion::new(
            "frustration",
            "How frustrated is the writer?",
            vec![ScoreLevel::new("calm").unwrap(), ScoreLevel::new("frustrated").unwrap()],
        )
        .unwrap();
        let question = KaiQuestion::from_decision(&state, &DecisionQuestion::Score(score)).unwrap();
        assert_eq!(question.kind, KaiKind::Score);
        assert_eq!(question.keys(), vec!["0", "1"]);

        let factual = State::from_text("s")
            .with_fact("modality", opencodifier_core::FactValue::Text("text".into()));
        let error = KaiQuestion::from_decision(
            &factual,
            &DecisionQuestion::Score(
                ScoreQuestion::new(
                    "f",
                    "t",
                    vec![ScoreLevel::new("calm").unwrap(), ScoreLevel::new("upset").unwrap()],
                )
                .unwrap(),
            ),
        )
        .unwrap_err();
        assert!(error.to_string().contains("text states only"), "{error}");
    }

    #[test]
    fn noul_defaults_and_segments_survive_a_round_trip() {
        let question = KaiQuestion::noul(
            "q",
            KaiPayload::Text("state".to_owned()),
            KaiPayload::Text("urgent?".to_owned()),
        );
        let segments = question.segments().unwrap();
        assert!(segments.prompt().ends_with(UPSTREAM_SUFFIX));
        assert_eq!(
            segments.prompt().len(),
            segments.prefix.len() + segments.options.concat().len() + segments.suffix.len()
        );
    }

    /// The reference encoder and the built-in oracle must agree on every
    /// frozen segment: this is what keeps the feature-free contract honest
    /// against the `tokenizers` crate a deployment actually loads.
    #[cfg(feature = "tokenizers")]
    #[test]
    fn hf_reference_encoder_agrees_with_the_builtin_oracle() {
        use opencodifier_core::ScoreLevel;

        let reference = super::HfTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let oracle = BpeTokenizer::from_tokenizer_json(FIXTURE_TOKENIZER).unwrap();
        let mut segments = 0;
        for case in fixture_cases() {
            let question = fixture_question(&case);
            let rendered = question.segments().unwrap();
            let mut parts = vec![rendered.prefix.clone()];
            parts.extend(rendered.options.iter().cloned());
            parts.push(rendered.suffix.clone());
            for part in parts {
                segments += 1;
                assert_eq!(
                    oracle.encode(&part).unwrap(),
                    reference.encode(&part).unwrap(),
                    "encoder disagreement on {part:?}"
                );
            }
        }
        assert!(segments >= 12, "expected the three frozen cases' segments, got {segments}");

        // A 2-level score question the fixture does not cover, built from
        // the damage case's own text so the trimmed asset's vocabulary can
        // encode it: the agreement is not limited to one level count.
        let score = ScoreQuestion::new(
            "damage_narrow",
            "How damaging is this review to the brand?",
            vec![ScoreLevel::new("harmless").unwrap(), ScoreLevel::new("severe").unwrap()],
        )
        .unwrap();
        let rendered = KaiQuestion::from_decision(
            &State::from_text(fixture_cases()[2].state.clone()),
            &DecisionQuestion::Score(score),
        )
        .unwrap()
        .segments()
        .unwrap();
        for part in
            [rendered.prefix, rendered.options.into_iter().collect::<String>(), rendered.suffix]
        {
            assert_eq!(oracle.encode(&part).unwrap(), reference.encode(&part).unwrap());
        }
    }
}
