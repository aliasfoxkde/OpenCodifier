//! `Jev` / System One codec (PLANNING.md §7, §36, §42).
//!
//! `Jev` is a typed-decision system whose public primitives map onto the IR
//! almost one-to-one, which is why the compatibility endpoint
//! `/v1/systemone` exists (PLANNING.md §36). The verified wire format:
//!
//! Request:
//!
//! ```json
//! {
//!   "state": "<unstructured state text>",
//!   "model": "<model identifier>",
//!   "questions": {
//!     "<id>": {
//!       "type": "choice" | "score" | "noul",
//!       "instructions": "<question text>",
//!       "criteria": {"<label>": "<description>"}
//!     }
//!   }
//! }
//! ```
//!
//! * `choice` — `criteria` maps candidate label to description, at most
//!   255 entries, never empty.
//! * `score` — `criteria` is the *ordered* level-label → description map.
//! * `noul` — a boolean question; `criteria` is absent.
//! * `model` is request context, stored as the `model` fact in
//!   [`State`]; `state` becomes [`State::from_text`].
//!
//! Response:
//!
//! ```json
//! {"model": "<model identifier>", "answers": {"<id>": <answer>}, "usage": {}}
//! ```
//!
//! * `score` — an object mapping **level index** keys `"0"`..`"n-1"` to
//!   probabilities. The decision value is the probability-weighted mean
//!   index, and the reported level is the level that mean falls into.
//! * `noul` — the probability of yes; `>= 0.5` decides `true`.
//! * `choice` — the chosen criteria label, mapped back to the candidate id.
//!
//! # Documented policies and limitations
//!
//! * **Missing answers are errors.** A question present in the request but
//!   absent from `answers` fails with `schema.missing_field`. Guessing a
//!   distribution for an unanswered question would fabricate a decision.
//! * **Unknown fields are ignored** in requests and responses, including
//!   `model` and `usage` on responses: `OpenCodifier` does not fabricate
//!   values for fields it cannot fill.
//! * **Score order.** Level order is carried by the document order of the
//!   `criteria` object. [`serde_json::Value`] cannot represent key order
//!   (its map is sorted unless the `preserve_order` feature is enabled,
//!   which this workspace avoids to keep canonical serialization stable),
//!   so [`Jev::decode_request_str`] parses the document in order and is
//!   the entry point the HTTP layer uses for vendor payloads. The
//!   [`WireFormat`] methods are deterministic but see `criteria` in
//!   sorted-key order. Because of that, a score answer's index keys
//!   (`"0".."n-1"`) are only meaningful against the request object they
//!   were produced from: never round-trip a request through a
//!   [`serde_json::Value`] before decoding its answers, or the index →
//!   level mapping shifts with the sorted order.
//! * **Choice answers are single-valued.** `Jev` reports the chosen label
//!   only, so the reconstructed distribution puts all mass on that
//!   candidate; as in [`crate::openai`] this describes the wire payload,
//!   and decoded responses carry
//!   [`opencodifier_core::DecisionOutcome::Verify`].
//! * **Labels are ids.** A `choice` candidate label must be a valid
//!   [`opencodifier_core::CandidateId`]; `Jev` labels that are not (spaces,
//!   unicode) are rejected rather than mangled.
//! * **Level descriptions are dropped.** The IR's
//!   [`opencodifier_core::ScoreLevel`] carries a label only, so a score
//!   level's description is not representable and is discarded on decode;
//!   [`WireFormat::encode_request`] writes empty descriptions rather than
//!   inventing any.
//!
//! # Example
//!
//! ```
//! use opencodifier_core::{DecisionQuestion, Limits};
//! use opencodifier_schema::WireFormat;
//! use opencodifier_schema::jev::Jev;
//!
//! // Raw text, exactly as a vendor sends it: `criteria` document order is
//! // the level order ("trivial" first), which the text entry point keeps.
//! let payload = r#"{
//!     "state": "one-shot refactor of the parser module",
//!     "model": "local-qwen",
//!     "questions": {
//!         "difficulty": {
//!             "type": "score",
//!             "instructions": "How difficult is this request?",
//!             "criteria": {"trivial": "no thought", "difficult": "real work"}
//!         }
//!     }
//! }"#;
//!
//! let request = Jev.decode_request_str(payload, &Limits::default()).unwrap();
//! let DecisionQuestion::Score(score) = &request.questions()[0] else {
//!     panic!("expected a score question");
//! };
//! assert_eq!(score.levels()[0].label(), "trivial");
//! assert_eq!(
//!     request.state().fact("model").map(opencodifier_core::FactValue::as_str),
//!     Some(Some("local-qwen"))
//! );
//! ```

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use opencodifier_core::{
    BooleanQuestion, Candidate, ChoiceQuestion, ConfidenceReport, DecisionAnswer, DecisionMetrics,
    DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest, DecisionResponse,
    DecisionTrace, Distribution, Limits, QuestionId, ScoreLevel, ScoreQuestion, State,
};

use crate::codec::{WireFormat, enforce_candidate_limit, enforce_limits, enforce_unique_questions};
use crate::error::{SchemaError, SchemaResult};
use crate::support::{
    as_object, as_string, non_empty, required_object, required_string, unit_interval,
};

/// Maximum entries in a `choice` criteria map, per the `Jev` wire contract.
const MAX_CRITERIA: usize = 255;

/// The `Jev` / System One codec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Jev;

/// A `Jev` request body as it arrives on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RequestWire {
    /// Unstructured state text.
    state: String,
    /// Model identifier, stored as the `model` fact.
    model: String,
    /// Extra typed facts, when a caller supplies them.
    #[serde(default)]
    facts: Option<serde_json::Map<String, Value>>,
    /// The questions to decide, in document order.
    questions: OrderedQuestions,
}

/// One `Jev` question as it arrives on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct QuestionWire {
    /// The `Jev` discriminator: `choice`, `score`, or `noul`.
    #[serde(rename = "type")]
    kind: String,
    /// The question text.
    instructions: String,
    /// Candidate or level map; absent for `noul`.
    #[serde(default)]
    criteria: Option<OrderedMap>,
}

/// A string→string map that keeps document order.
///
/// `serde_json::Value` sorts keys, so a score question's level order has
/// to be captured by the deserializer itself (module docs).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct OrderedMap(Vec<(String, String)>);

impl Serialize for OrderedMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for OrderedMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor;

        impl<'de> serde::de::Visitor<'de> for MapVisitor {
            type Value = OrderedMap;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object mapping strings to strings")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut entries: Vec<(String, String)> = Vec::new();
                while let Some(key) = access.next_key::<String>()? {
                    let value = access.next_value::<String>()?;
                    if entries.iter().any(|(existing, _)| *existing == key) {
                        return Err(serde::de::Error::custom(format_args!(
                            "duplicate key `{key}`"
                        )));
                    }
                    entries.push((key, value));
                }
                Ok(OrderedMap(entries))
            }
        }
        deserializer.deserialize_map(MapVisitor)
    }
}

/// An id→question map that keeps document order.
#[derive(Debug, Clone, PartialEq)]
struct OrderedQuestions(Vec<(String, QuestionWire)>);

impl<'de> Deserialize<'de> for OrderedQuestions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct QuestionsVisitor;

        impl<'de> serde::de::Visitor<'de> for QuestionsVisitor {
            type Value = OrderedQuestions;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON object of Jev questions")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut questions = Vec::new();
                while let Some(id) = access.next_key::<String>()? {
                    let question: QuestionWire = access.next_value()?;
                    if questions
                        .iter()
                        .any(|(existing, _): &(String, QuestionWire)| *existing == id)
                    {
                        return Err(serde::de::Error::custom(format_args!(
                            "duplicate question id `{id}`"
                        )));
                    }
                    questions.push((id, question));
                }
                Ok(OrderedQuestions(questions))
            }
        }
        deserializer.deserialize_map(QuestionsVisitor)
    }
}

impl Serialize for OrderedQuestions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, question) in &self.0 {
            map.serialize_entry(id, question)?;
        }
        map.end()
    }
}

impl Jev {
    // The codecs are stateless unit structs, but every operation takes
    // `&self` so a future registry can hand out configured variants without
    // an API break (PLANNING.md §36). The private helpers follow the same
    // shape for symmetry, hence the two allows.
    #![allow(clippy::unused_self, clippy::trivially_copy_pass_by_ref)]
    /// Parses a request from its JSON text, preserving `criteria` order.
    ///
    /// This is the entry point for vendor payloads: it is the only one that
    /// can see the document order of a score question's level map.
    pub fn decode_request_str(&self, text: &str, limits: &Limits) -> SchemaResult<DecisionRequest> {
        let wire: RequestWire =
            serde_json::from_str(text).map_err(|error| SchemaError::Json(error.to_string()))?;
        self.build(wire, limits)
    }

    /// Serializes a request to `Jev` JSON text, preserving level order.
    ///
    /// [`WireFormat::encode_request`] has to return a
    /// [`serde_json::Value`], whose maps are sorted by key. That is harmless
    /// for `choice` and `noul`, but for a `score` question the criteria
    /// order *is* the index mapping, so a sorted map would renumber the
    /// levels behind the caller's back. This entry point writes `criteria`
    /// in question order and is the form to send to a `Jev` peer; decode the
    /// peer's reply against the request it produced.
    pub fn encode_request_str(&self, request: &DecisionRequest) -> SchemaResult<String> {
        enforce_unique_questions(request)?;
        let mut questions = Vec::with_capacity(request.questions().len());
        for question in request.questions() {
            let (kind, criteria) = match question {
                DecisionQuestion::Choice(choice) => (
                    "choice",
                    Some(OrderedMap(
                        choice
                            .candidates()
                            .iter()
                            .map(|candidate| {
                                (candidate.id().to_string(), candidate.description().to_owned())
                            })
                            .collect(),
                    )),
                ),
                DecisionQuestion::Score(score) => (
                    "score",
                    Some(OrderedMap(
                        score
                            .levels()
                            .iter()
                            .map(|level| (level.label().to_owned(), String::new()))
                            .collect(),
                    )),
                ),
                DecisionQuestion::Boolean(_) => ("noul", None),
                _ => return Err(SchemaError::unsupported("question type")),
            };
            questions.push((
                question.id().to_string(),
                QuestionWire {
                    kind: kind.to_owned(),
                    instructions: question.text().to_owned(),
                    criteria,
                },
            ));
        }
        let model = request
            .state()
            .fact("model")
            .and_then(opencodifier_core::FactValue::as_str)
            .unwrap_or("")
            .to_owned();
        let wire = RequestWire {
            state: request.state().text().to_owned(),
            model,
            facts: None,
            questions: OrderedQuestions(questions),
        };
        serde_json::to_string(&wire).map_err(|error| SchemaError::Json(error.to_string()))
    }

    /// Parses a response from its JSON text.
    pub fn decode_response_str(
        &self,
        text: &str,
        request: &DecisionRequest,
        limits: &Limits,
    ) -> SchemaResult<DecisionResponse> {
        let payload: Value =
            serde_json::from_str(text).map_err(|error| SchemaError::Json(error.to_string()))?;
        self.decode_response(&payload, request, limits)
    }

    /// Builds the canonical request from a wire request.
    fn build(&self, wire: RequestWire, limits: &Limits) -> SchemaResult<DecisionRequest> {
        enforce_limits(wire.state.len(), wire.questions.0.len(), limits)?;
        let mut state = State::from_text(wire.state)
            .with_fact("model", opencodifier_core::FactValue::Text(wire.model));
        if let Some(facts) = wire.facts {
            for (key, value) in &facts {
                let fact: opencodifier_core::FactValue = serde_json::from_value(value.clone())
                    .map_err(|error| SchemaError::invalid_value(key, error.to_string()))?;
                state = state.with_fact(key.clone(), fact);
            }
        }
        let mut questions = Vec::with_capacity(wire.questions.0.len());
        for (id, question) in &wire.questions.0 {
            questions.push(self.question(id, question, limits)?);
        }
        // `enforce_limits` above has already rejected an empty question set.
        DecisionRequest::new(state, questions, DecisionPolicy::default(), metadata(limits))
            .map_err(SchemaError::from)
    }

    /// Converts one `Jev` question into a canonical question.
    fn question(
        &self,
        id: &str,
        question: &QuestionWire,
        limits: &Limits,
    ) -> SchemaResult<DecisionQuestion> {
        let instructions = non_empty("instructions", question.instructions.clone())?;
        match question.kind.as_str() {
            "choice" => {
                let criteria =
                    question.criteria.as_ref().ok_or_else(|| SchemaError::missing("criteria"))?;
                if criteria.0.is_empty() {
                    return Err(SchemaError::invalid_value(
                        id,
                        "a choice question needs at least one criterion",
                    ));
                }
                if criteria.0.len() > MAX_CRITERIA {
                    return Err(SchemaError::limit("max_criteria", criteria.0.len(), MAX_CRITERIA));
                }
                enforce_candidate_limit(criteria.0.len(), limits)?;
                let candidates = criteria
                    .0
                    .iter()
                    .map(|(label, description)| {
                        Candidate::new(label.clone(), description.clone())
                            .map_err(|error| SchemaError::invalid_value(id, error.to_string()))
                    })
                    .collect::<SchemaResult<Vec<_>>>()?;
                ChoiceQuestion::new(id, instructions, candidates)
                    .map(DecisionQuestion::Choice)
                    .map_err(SchemaError::from)
            }
            "score" => {
                let criteria =
                    question.criteria.as_ref().ok_or_else(|| SchemaError::missing("criteria"))?;
                if criteria.0.len() < 2 {
                    return Err(SchemaError::invalid_value(
                        id,
                        format!("a score needs at least two levels, got {}", criteria.0.len()),
                    ));
                }
                enforce_candidate_limit(criteria.0.len(), limits)?;
                let levels = criteria
                    .0
                    .iter()
                    .map(|(label, _)| {
                        ScoreLevel::new(label.clone())
                            .map_err(|error| SchemaError::invalid_value(id, error.to_string()))
                    })
                    .collect::<SchemaResult<Vec<_>>>()?;
                ScoreQuestion::new(id, instructions, levels)
                    .map(DecisionQuestion::Score)
                    .map_err(SchemaError::from)
            }
            "noul" => BooleanQuestion::new(id, instructions)
                .map(DecisionQuestion::Boolean)
                .map_err(SchemaError::from),
            other => Err(SchemaError::InvalidType {
                field: format!("{id}.type"),
                expected: "choice|score|noul",
                got: other.to_owned(),
            }),
        }
    }

    /// Converts one `Jev` answer value into a canonical answer.
    fn answer(&self, question: &DecisionQuestion, answer: &Value) -> SchemaResult<DecisionAnswer> {
        let id = question.id().to_string();
        match question {
            DecisionQuestion::Choice(choice) => {
                let label = as_string(&id, answer)?;
                let candidate = choice
                    .candidates()
                    .iter()
                    .find(|candidate| candidate.id().as_str() == label)
                    .ok_or_else(|| {
                        SchemaError::invalid_value(
                            &id,
                            format!("`{label}` is not an offered candidate"),
                        )
                    })?;
                let key = candidate.id().to_string();
                let distribution = Distribution::from_pairs([(key.as_str(), 1.0)])
                    .map_err(|error| SchemaError::invalid_value(&id, error.to_string()))?;
                Ok(DecisionAnswer::Choice {
                    question_id: QuestionId::new(&id).map_err(SchemaError::from)?,
                    choice: candidate.id().clone(),
                    distribution,
                    confidence: 1.0,
                })
            }
            DecisionQuestion::Boolean(_) => {
                let yes = answer
                    .as_f64()
                    .ok_or_else(|| SchemaError::invalid_type(&id, "number", answer))?;
                let yes = unit_interval(&id, yes)?;
                let value = yes >= 0.5;
                // `probability` in the IR is the probability that the
                // decided value is correct, not the probability of yes.
                let probability = if value { yes } else { 1.0 - yes };
                Ok(DecisionAnswer::Boolean {
                    question_id: QuestionId::new(&id).map_err(SchemaError::from)?,
                    value,
                    probability,
                    confidence: probability,
                })
            }
            DecisionQuestion::Score(score) => {
                let weights = as_object(&id, answer)?;
                let mut pairs = Vec::with_capacity(score.levels().len());
                let mut expected = 0.0;
                for (index, level) in score.levels().iter().enumerate() {
                    let key = index.to_string();
                    let field = format!("{id}.{key}");
                    let Some(probability) = weights.get(&key) else {
                        return Err(SchemaError::MissingField {
                            field,
                            context: ": score answers must cover every level index".to_owned(),
                        });
                    };
                    let probability = probability
                        .as_f64()
                        .ok_or_else(|| SchemaError::invalid_type(&field, "number", probability))?;
                    let probability = unit_interval(&field, probability)?;
                    pairs.push((level.label().to_owned(), probability));
                    // Level counts are tiny; widening through `u32` keeps
                    // the cast precise.
                    let index = f64::from(u32::try_from(index).unwrap_or(u32::MAX));
                    expected += probability * index;
                }
                let distribution = Distribution::from_pairs(pairs)
                    .map_err(|error| SchemaError::invalid_value(&id, error.to_string()))?;
                let confidence = distribution.top().probability;
                Ok(DecisionAnswer::Score {
                    question_id: QuestionId::new(&id).map_err(SchemaError::from)?,
                    expected,
                    level: score.level_for(expected).label().to_owned(),
                    distribution,
                    confidence,
                })
            }
            _ => Err(SchemaError::unsupported("question type")),
        }
    }
}

impl WireFormat for Jev {
    fn name(&self) -> &'static str {
        "jev"
    }
    /// Projects a request onto the `Jev` request shape.
    ///
    /// The state's `model` fact becomes the `model` field (empty when
    /// absent — no model is invented). Questions are emitted in canonical
    /// (sorted id) order.
    fn encode_request(&self, request: &DecisionRequest) -> SchemaResult<Value> {
        enforce_unique_questions(request)?;
        let mut questions = serde_json::Map::new();
        for question in request.questions() {
            let (kind, criteria) = match question {
                DecisionQuestion::Choice(choice) => {
                    let mut criteria = serde_json::Map::new();
                    for candidate in choice.candidates() {
                        criteria.insert(candidate.id().to_string(), json!(candidate.description()));
                    }
                    ("choice", Value::Object(criteria))
                }
                DecisionQuestion::Score(score) => {
                    let mut criteria = serde_json::Map::new();
                    for level in score.levels() {
                        criteria.insert(level.label().to_owned(), json!(""));
                    }
                    ("score", Value::Object(criteria))
                }
                DecisionQuestion::Boolean(_) => ("noul", json!({})),
                _ => return Err(SchemaError::unsupported("question type")),
            };
            questions.insert(
                question.id().to_string(),
                json!({"type": kind, "instructions": question.text(), "criteria": criteria}),
            );
        }
        let model = request
            .state()
            .fact("model")
            .and_then(opencodifier_core::FactValue::as_str)
            .unwrap_or("");
        Ok(json!({
            "state": request.state().text(),
            "model": model,
            "questions": Value::Object(questions),
        }))
    }

    /// Normalizes a `Jev` request payload.
    ///
    /// Deterministic but order-normalizing: score levels arrive in sorted
    /// key order because [`serde_json::Value`] does not carry key order.
    /// Use [`Jev::decode_request_str`] for vendor payloads.
    fn decode_request(&self, payload: &Value, limits: &Limits) -> SchemaResult<DecisionRequest> {
        let body = as_object("payload", payload)?;
        let state = required_string(body, "state")?;
        let model = required_string(body, "model")?;
        let facts = match body.get("facts") {
            Some(Value::Null) | None => None,
            Some(value) => Some(as_object("facts", value)?.clone()),
        };
        let questions = required_object(body, "questions")?;
        let ordered = questions
            .iter()
            .map(|(id, value)| {
                let question: QuestionWire = serde_json::from_value(value.clone())
                    .map_err(|error| wire_error(id, &error))?;
                Ok((id.clone(), question))
            })
            .collect::<SchemaResult<Vec<_>>>()?;
        self.build(
            RequestWire { state, model, facts, questions: OrderedQuestions(ordered) },
            limits,
        )
    }

    /// Projects a response onto the `Jev` response shape.
    ///
    /// Score answers are emitted as probability maps over level indices
    /// (`"0"`..`"n-1"`), `noul` answers as the probability of yes, and
    /// choice answers as the chosen label. `model` and `usage` are left
    /// out: `OpenCodifier` does not invent values for them (module docs).
    fn encode_response(&self, response: &DecisionResponse) -> SchemaResult<Value> {
        let mut answers = serde_json::Map::new();
        for answer in response.answers() {
            let (id, value) = match answer {
                DecisionAnswer::Choice { question_id, choice, .. } => {
                    (question_id.to_string(), json!(choice.as_str()))
                }
                DecisionAnswer::Boolean { question_id, value, probability, .. } => {
                    let yes = if *value { *probability } else { 1.0 - *probability };
                    (question_id.to_string(), json!(yes))
                }
                DecisionAnswer::Score { question_id, distribution, .. } => {
                    (question_id.to_string(), index_distribution(distribution))
                }
                _ => return Err(SchemaError::unsupported("answer type")),
            };
            answers.insert(id, value);
        }
        Ok(json!({"answers": Value::Object(answers)}))
    }

    /// Normalizes a `Jev` response into canonical IR answers.
    fn decode_response(
        &self,
        payload: &Value,
        request: &DecisionRequest,
        _limits: &Limits,
    ) -> SchemaResult<DecisionResponse> {
        let body = as_object("payload", payload)?;
        let answers = required_object(body, "answers")?;
        let mut decoded = Vec::with_capacity(request.questions().len());
        for question in request.questions() {
            let id = question.id().to_string();
            let Some(answer) = answers.get(&id) else {
                return Err(SchemaError::MissingField {
                    field: id,
                    context: ": the request question has no answer".to_owned(),
                });
            };
            decoded.push(self.answer(question, answer)?);
        }
        let report = report_for(&decoded);
        DecisionResponse::new(
            decoded,
            DecisionOutcome::Verify,
            report,
            DecisionTrace::new(),
            DecisionMetrics::default(),
        )
        .map_err(SchemaError::from)
    }
}

/// Maps a `serde` deserialization failure onto the closest schema code.
///
/// `serde` reports absent required fields as "missing field `x`", which is
/// exactly [`SchemaError::MissingField`]; everything else is a value that
/// has the wrong shape.
fn wire_error(field: &str, error: &serde_json::Error) -> SchemaError {
    let text = error.to_string();
    if let Some(name) = text.strip_prefix("missing field `").and_then(|rest| rest.split('`').next())
    {
        return SchemaError::MissingField {
            field: name.to_owned(),
            context: format!(" in Jev question `{field}`"),
        };
    }
    SchemaError::invalid_value(field, text)
}

/// Wraps caller limits into request metadata.
fn metadata(limits: &Limits) -> opencodifier_core::RequestMetadata {
    opencodifier_core::RequestMetadata { request_id: None, limits: limits.clone() }
}

/// Re-expresses a level distribution as a probability map over indices.
fn index_distribution(distribution: &Distribution) -> Value {
    let mut indices = serde_json::Map::new();
    for (index, entry) in distribution.entries().iter().enumerate() {
        indices.insert(index.to_string(), json!(entry.probability));
    }
    Value::Object(indices)
}

/// Builds the confidence report for `Jev` answers.
///
/// `Jev` reports real probability vectors, so this report is conservative
/// and meaningful: the calibrated confidence is the *weakest* answer's top
/// probability, the margin the smallest margin, and the entropy the
/// largest — the cascade must gate on the least certain question in the
/// request, not on their average.
pub(crate) fn report_for(answers: &[DecisionAnswer]) -> ConfidenceReport {
    let mut top_probability = 1.0;
    let mut margin = 1.0;
    let mut entropy = 0.0;
    for answer in answers {
        let (probability, answer_margin, answer_entropy) = match answer {
            DecisionAnswer::Choice { distribution, .. }
            | DecisionAnswer::Score { distribution, .. } => {
                (distribution.top().probability, distribution.margin(), distribution.entropy())
            }
            DecisionAnswer::Boolean { probability, .. } => (*probability, *probability, 0.0),
            // An answer type added after this crate was built is treated
            // as maximally uncertain: the cascade must not accept it.
            _ => (0.0, 0.0, 0.0),
        };
        if probability < top_probability {
            top_probability = probability;
        }
        if answer_margin < margin {
            margin = answer_margin;
        }
        if answer_entropy > entropy {
            entropy = answer_entropy;
        }
    }
    ConfidenceReport {
        top_probability,
        margin,
        entropy,
        calibrated_confidence: top_probability,
        ood_score: 0.0,
        verifier_agreement: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    fn limits() -> Limits {
        Limits { max_questions: 4, max_candidates: 8, ..Limits::default() }
    }

    /// Raw text, not a `serde_json::Value`: the whole point of these tests
    /// is that `criteria` document order survives, and a `Value` sorts keys.
    fn choice_request_text() -> String {
        String::from(
            r#"{
  "state": "pick a model for this request",
  "model": "local-qwen",
  "questions": {
    "model": {
      "type": "choice",
      "instructions": "Which model should answer?",
      "criteria": {"local-qwen": "general coding", "local-glm": "deep reasoning"}
    }
  }
}"#,
        )
    }

    /// Level order here (`trivial`, `hard`, `expert`) is deliberately not
    /// the sorted key order, so an order-losing decode would fail.
    fn score_request_text() -> String {
        String::from(
            r#"{
  "state": "estimate task difficulty",
  "model": "local-qwen",
  "questions": {
    "difficulty": {
      "type": "score",
      "instructions": "How difficult?",
      "criteria": {"trivial": "none", "hard": "some", "expert": "lots"}
    }
  }
}"#,
        )
    }

    fn noul_request_text() -> String {
        String::from(
            r#"{
  "state": "does this need tools?",
  "model": "local-qwen",
  "questions": {
    "needs_tools": {"type": "noul", "instructions": "Are tools required?"}
  }
}"#,
        )
    }

    /// All three question kinds in one payload, so one response carries an
    /// answer of every shape.
    fn mixed_request_text() -> String {
        String::from(
            r#"{
  "state": "estimate difficulty and pick a model",
  "model": "local-qwen",
  "questions": {
    "difficulty": {
      "type": "score",
      "instructions": "How difficult?",
      "criteria": {"trivial": "none", "hard": "some", "expert": "lots"}
    },
    "needs_tools": {"type": "noul", "instructions": "Are tools required?"},
    "model": {
      "type": "choice",
      "instructions": "Which model should answer?",
      "criteria": {"local-qwen": "general coding", "local-glm": "deep reasoning"}
    }
  }
}"#,
        )
    }

    /// Finds the score answer and its reconstructed fields.
    fn score_answer(answers: &[DecisionAnswer]) -> Option<(f64, &str, &Distribution, f64)> {
        answers.iter().find_map(|answer| match answer {
            DecisionAnswer::Score { expected, level, distribution, confidence, .. } => {
                Some((*expected, level.as_str(), distribution, *confidence))
            }
            _ => None,
        })
    }

    /// Finds the boolean answer and the probability of its decided value.
    fn boolean_answer(answers: &[DecisionAnswer]) -> Option<(bool, f64)> {
        answers.iter().find_map(|answer| match answer {
            DecisionAnswer::Boolean { value, probability, .. } => Some((*value, *probability)),
            _ => None,
        })
    }

    /// Finds the choice answer and its reconstructed distribution.
    fn choice_answer(
        answers: &[DecisionAnswer],
    ) -> Option<(&opencodifier_core::CandidateId, &Distribution)> {
        answers.iter().find_map(|answer| match answer {
            DecisionAnswer::Choice { choice, distribution, .. } => Some((choice, distribution)),
            _ => None,
        })
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(Jev.name(), "jev");
    }

    #[test]
    fn choice_request_decodes_with_candidate_metadata() {
        let request = Jev.decode_request_str(&choice_request_text(), &limits()).unwrap();
        assert_eq!(request.state().text(), "pick a model for this request");
        assert_eq!(
            request.state().fact("model").map(opencodifier_core::FactValue::as_str),
            Some(Some("local-qwen"))
        );
        let DecisionQuestion::Choice(choice) = &request.questions()[0] else { unreachable!() };
        assert_eq!(choice.text(), "Which model should answer?");
        let ids: Vec<&str> = choice.candidates().iter().map(|c| c.id().as_str()).collect();
        assert_eq!(ids, ["local-qwen", "local-glm"]);
        assert_eq!(choice.candidates()[0].description(), "general coding");
    }

    #[test]
    fn score_request_preserves_document_order() {
        // "hard" sorts before "trivial", so a key-sorted decode would flip
        // the level order and mis-map every answer index.
        let request = Jev.decode_request_str(&score_request_text(), &limits()).unwrap();
        let DecisionQuestion::Score(score) = &request.questions()[0] else { unreachable!() };
        let labels: Vec<&str> = score.levels().iter().map(ScoreLevel::label).collect();
        assert_eq!(labels, ["trivial", "hard", "expert"]);
    }

    #[test]
    fn noul_request_decodes_as_boolean() {
        let request = Jev.decode_request_str(&noul_request_text(), &limits()).unwrap();
        assert_eq!(
            request.questions()[0],
            DecisionQuestion::Boolean(
                BooleanQuestion::new("needs_tools", "Are tools required?").unwrap()
            )
        );
    }

    #[test]
    fn questions_that_are_not_an_object_report_the_expected_shape() {
        // The failure surfaces from the order-preserving visitor, which
        // names the shape it wanted rather than a generic parse error.
        let error = Jev
            .decode_request_str(r#"{"state":"s","model":"m","questions":[]}"#, &limits())
            .unwrap_err();
        assert_eq!(error.code(), "schema.invalid_json");
        assert!(error.to_string().contains("a JSON object of Jev questions"), "{error}");
    }

    #[test]
    fn non_object_questions_report_invalid_values() {
        let payload = json!({"state": "s", "model": "m", "questions": {"q": "instructions"}});
        let error = Jev.decode_request(&payload, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert!(error.to_string().contains("`q`"), "{error}");
    }

    /// The accessors report absence instead of panicking, which is what
    /// lets a test pin down which answer kinds a response contains.
    #[test]
    fn answer_accessors_report_absence() {
        let request = Jev.decode_request_str(&noul_request_text(), &limits()).unwrap();
        let response = Jev
            .decode_response(&json!({"answers": {"needs_tools": 0.5}}), &request, &limits())
            .unwrap();
        let answers = response.answers();
        assert!(boolean_answer(answers).is_some());
        assert!(score_answer(answers).is_none());
        assert!(choice_answer(answers).is_none());
    }

    #[test]
    fn responses_decode_per_documented_rules() {
        let request = Jev.decode_request_str(&mixed_request_text(), &limits()).unwrap();
        let payload = json!({
            "model": "local-qwen",
            "answers": {
                "difficulty": {"0": 0.2, "1": 0.5, "2": 0.3},
                "needs_tools": 0.42,
                "model": "local-glm",
            },
            "usage": {"input_tokens": 12}
        });
        let response = Jev.decode_response(&payload, &request, &limits()).unwrap();
        assert_eq!(response.answers().len(), 3);
        assert_eq!(response.outcome(), DecisionOutcome::Verify);

        let (expected, level, distribution, confidence) =
            score_answer(response.answers()).expect("expected a score answer");
        assert!((expected - 1.1).abs() < 1e-9, "0.2*0 + 0.5*1 + 0.3*2 = {expected}");
        assert_eq!(level, "hard", "the weighted mean index 1.1 falls into level 1");
        assert_eq!(distribution.entries().len(), 3);
        assert!((confidence - 0.5).abs() < 1e-9);

        // A `noul` answer decides on the 0.5 threshold and carries its own
        // probability: the answer is `false` at 0.42, so the probability of
        // the decided value is 0.58.
        let (value, probability) =
            boolean_answer(response.answers()).expect("expected a boolean answer");
        assert!(!value, "0.42 < 0.5 is false");
        assert!((probability - 0.58).abs() < 1e-9, "the decided value carries its own probability");

        let (choice, distribution) =
            choice_answer(response.answers()).expect("expected a choice answer");
        assert_eq!(choice.as_str(), "local-glm");
        assert_eq!(distribution.top().key, "local-glm");
    }

    #[test]
    fn responses_encode_back_to_the_wire_shape() {
        let score_request = Jev.decode_request_str(&score_request_text(), &limits()).unwrap();
        let payload = json!({"model": "m", "answers": {"difficulty": {"0": 0.25, "1": 0.25, "2": 0.5}}, "usage": {}});
        let response =
            Jev.decode_response_str(&payload.to_string(), &score_request, &limits()).unwrap();
        let encoded = Jev.encode_response(&response).unwrap();
        assert_eq!(encoded["answers"]["difficulty"], json!({"0": 0.25, "1": 0.25, "2": 0.5}));

        let noul_request = Jev.decode_request_str(&noul_request_text(), &limits()).unwrap();
        let response = Jev
            .decode_response(&json!({"answers": {"needs_tools": 0.4}}), &noul_request, &limits())
            .unwrap();
        let encoded = Jev.encode_response(&response).unwrap();
        assert_eq!(encoded["answers"]["needs_tools"], json!(0.4), "p_yes is restored");

        let choice_request = Jev.decode_request_str(&choice_request_text(), &limits()).unwrap();
        let response = Jev
            .decode_response(
                &json!({"answers": {"model": "local-qwen"}}),
                &choice_request,
                &limits(),
            )
            .unwrap();
        let encoded = Jev.encode_response(&response).unwrap();
        assert_eq!(encoded["answers"]["model"], json!("local-qwen"));
    }

    #[test]
    fn noul_answers_decide_on_the_threshold() {
        let request = Jev.decode_request_str(&noul_request_text(), &limits()).unwrap();
        let response = Jev
            .decode_response(&json!({"answers": {"needs_tools": 0.9}}), &request, &limits())
            .unwrap();
        let (value, probability) =
            boolean_answer(response.answers()).expect("expected a boolean answer");
        assert!(value, "0.9 >= 0.5 is true");
        assert!((probability - 0.9).abs() < 1e-9);
    }

    #[test]
    fn requests_encode_back_to_the_wire_shape() {
        let request = Jev.decode_request_str(&choice_request_text(), &limits()).unwrap();
        let encoded = Jev.encode_request(&request).unwrap();
        assert_eq!(encoded["state"], json!("pick a model for this request"));
        assert_eq!(encoded["model"], json!("local-qwen"));
        assert_eq!(encoded["questions"]["model"]["type"], json!("choice"));
        assert_eq!(
            encoded["questions"]["model"]["criteria"]["local-qwen"],
            json!("general coding")
        );

        let score_request = Jev.decode_request_str(&score_request_text(), &limits()).unwrap();
        let encoded = Jev.encode_request(&score_request).unwrap();
        assert_eq!(encoded["questions"]["difficulty"]["type"], json!("score"));
        assert_eq!(
            encoded["questions"]["difficulty"]["criteria"]["trivial"],
            json!(""),
            "score level descriptions are not representable in the IR"
        );

        let boolean_only = DecisionRequest::new(
            State::from_text("no model fact here"),
            vec![DecisionQuestion::Boolean(BooleanQuestion::new("q", "text").unwrap())],
            DecisionPolicy::default(),
            metadata(&limits()),
        )
        .unwrap();
        let encoded = Jev.encode_request(&boolean_only).unwrap();
        assert_eq!(encoded["model"], json!(""), "no model fact means no invented model");
        assert_eq!(encoded["questions"]["q"]["type"], json!("noul"));
        assert_eq!(encoded["questions"]["q"]["criteria"], json!({}));
    }

    /// The text path is the only projection that can carry `criteria`
    /// document order, so a request serialized as text must keep the IR's
    /// question order and level order — and decode back to exactly the
    /// questions that were encoded.
    #[test]
    fn encoded_text_preserves_document_order() {
        // Neither the level list nor the candidate list is in sorted key
        // order, so a key-sorting projection would fail the assertions
        // below rather than pass them vacuously.
        let request = DecisionRequest::new(
            State::from_text("estimate task difficulty"),
            vec![
                DecisionQuestion::Score(
                    ScoreQuestion::new(
                        "difficulty",
                        "How difficult?",
                        ["trivial", "hard", "expert"]
                            .iter()
                            .map(|label| ScoreLevel::new(*label).unwrap())
                            .collect(),
                    )
                    .unwrap(),
                ),
                DecisionQuestion::Choice(
                    ChoiceQuestion::new(
                        "model",
                        "Which model should answer?",
                        vec![
                            Candidate::new("local-qwen", "general coding").unwrap(),
                            Candidate::new("local-glm", "deep reasoning").unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
                DecisionQuestion::Boolean(BooleanQuestion::new("needs_tools", "Tools?").unwrap()),
            ],
            DecisionPolicy::default(),
            metadata(&limits()),
        )
        .unwrap();

        let text = Jev.encode_request_str(&request).unwrap();
        let at = |label: &str| {
            text.find(label).unwrap_or_else(|| panic!("`{label}` is absent from: {text}"))
        };
        assert!(at("\"trivial\"") < at("\"hard\""), "level order: {text}");
        assert!(at("\"hard\"") < at("\"expert\""), "level order: {text}");
        assert!(
            at("\"local-qwen\"") < at("\"local-glm\""),
            "candidate order is the IR's, not the sorted key order: {text}"
        );
        // Question order follows the IR as well (`needs_tools` sorts first,
        // so a key-sorted projection would emit it before the score).
        assert!(at("\"difficulty\"") < at("\"needs_tools\""), "question order: {text}");

        let decoded = Jev.decode_request_str(&text, &limits()).unwrap();
        assert_eq!(
            decoded.questions(),
            request.questions(),
            "the text path round-trips question and member order"
        );
        assert_eq!(decoded.state().text(), request.state().text());
        assert_eq!(
            decoded.questions()[2],
            DecisionQuestion::Boolean(BooleanQuestion::new("needs_tools", "Tools?").unwrap()),
            "a `noul` question carries no criteria"
        );
    }

    /// [`WireFormat::decode_request`] sees `criteria` in sorted key order,
    /// so the level order — and with it every answer's index mapping — is
    /// normalized. This is the hazard the module docs warn about: the same
    /// answer payload names a *different* level depending on which entry
    /// point decoded the request it belongs to.
    #[test]
    fn value_path_sorts_levels_and_shifts_score_indices() {
        let from_text = Jev.decode_request_str(&score_request_text(), &limits()).unwrap();
        let DecisionQuestion::Score(score) = &from_text.questions()[0] else { unreachable!() };
        let labels: Vec<&str> = score.levels().iter().map(ScoreLevel::label).collect();
        assert_eq!(labels, ["trivial", "hard", "expert"]);

        let value: Value = serde_json::from_str(&score_request_text()).unwrap();
        let from_value = Jev.decode_request(&value, &limits()).unwrap();
        let DecisionQuestion::Score(sorted) = &from_value.questions()[0] else { unreachable!() };
        let sorted_labels: Vec<&str> = sorted.levels().iter().map(ScoreLevel::label).collect();
        assert_eq!(
            sorted_labels,
            ["expert", "hard", "trivial"],
            "a `serde_json::Value` carries keys in sorted order"
        );

        // Index 2 is `expert` for the text decode and `trivial` for the
        // sorted one, so an answer built for one is wrong for the other.
        let payload = json!({"answers": {"difficulty": {"0": 0.0, "1": 0.0, "2": 1.0}}});
        let text_answer = Jev.decode_response(&payload, &from_text, &limits()).unwrap();
        let value_answer = Jev.decode_response(&payload, &from_value, &limits()).unwrap();
        assert_eq!(score_answer(text_answer.answers()).unwrap().1, "expert");
        assert_eq!(score_answer(value_answer.answers()).unwrap().1, "trivial");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let mut payload: Value = serde_json::from_str(&choice_request_text()).unwrap();
        let body = payload.as_object_mut().unwrap();
        body.insert("temperature".to_owned(), json!(0.5));
        body.insert("stream".to_owned(), json!(true));
        assert!(Jev.decode_request(&payload, &limits()).is_ok());

        let request = Jev.decode_request_str(&choice_request_text(), &limits()).unwrap();
        let response =
            json!({"model": "m", "answers": {"model": "local-glm"}, "usage": {}, "extra": [1]});
        assert!(Jev.decode_response(&response, &request, &limits()).is_ok());
    }

    #[test]
    fn facts_supplied_alongside_the_model_fact_are_kept() {
        let payload = json!({
            "state": "s",
            "model": "m",
            "facts": {"context_tokens": {"kind": "integer", "value": 4096}},
            "questions": {"q": {"type": "noul", "instructions": "x"}}
        });
        let request = Jev.decode_request(&payload, &limits()).unwrap();
        assert_eq!(
            request.state().fact("context_tokens").map(opencodifier_core::FactValue::as_f64),
            Some(Some(4_096.0))
        );
        let bad_facts = json!({
            "state": "s", "model": "m",
            "facts": {"context_tokens": {"kind": "integer", "value": "many"}},
            "questions": {"q": {"type": "noul", "instructions": "x"}}
        });
        assert_eq!(
            Jev.decode_request(&bad_facts, &limits()).unwrap_err().code(),
            "schema.invalid_value"
        );
    }

    #[test]
    fn malformed_requests_report_typed_errors() {
        let limits = limits();

        assert_eq!(
            Jev.decode_request_str("{not json", &limits).unwrap_err().code(),
            "schema.invalid_json"
        );

        let missing_state = json!({"model": "m", "questions": {}});
        assert_eq!(
            Jev.decode_request(&missing_state, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let missing_model = json!({"state": "s", "questions": {}});
        assert_eq!(
            Jev.decode_request(&missing_model, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let missing_questions = json!({"state": "s", "model": "m"});
        assert_eq!(
            Jev.decode_request(&missing_questions, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let empty_questions = json!({"state": "s", "model": "m", "questions": {}});
        assert_eq!(
            Jev.decode_request(&empty_questions, &limits).unwrap_err().code(),
            "schema.empty_questions"
        );

        let unknown_type = json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "regress", "instructions": "x", "criteria": {}}}
        });
        let error = Jev.decode_request(&unknown_type, &limits).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");
        assert!(error.to_string().contains("choice|score|noul"), "{error}");

        let missing_instructions =
            json!({"state": "s", "model": "m", "questions": {"q": {"type": "noul"}}});
        assert_eq!(
            Jev.decode_request(&missing_instructions, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let blank_instructions = json!({"state": "s", "model": "m", "questions": {"q": {"type": "noul", "instructions": ""}}});
        assert_eq!(
            Jev.decode_request(&blank_instructions, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let whitespace_instructions = json!({"state": "s", "model": "m", "questions": {"q": {"type": "noul", "instructions": " "}}});
        // Whitespace-only text is non-empty; the IR allows it.
        assert!(Jev.decode_request(&whitespace_instructions, &limits).is_ok());

        let missing_criteria = json!({"state": "s", "model": "m", "questions": {"q": {"type": "choice", "instructions": "x"}}});
        assert_eq!(
            Jev.decode_request(&missing_criteria, &limits).unwrap_err().code(),
            "schema.missing_field"
        );

        let payload_not_object = json!(["questions"]);
        assert_eq!(
            Jev.decode_request(&payload_not_object, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );

        let questions_not_object = json!({"state": "s", "model": "m", "questions": []});
        assert_eq!(
            Jev.decode_request(&questions_not_object, &limits).unwrap_err().code(),
            "schema.invalid_type"
        );
    }

    /// Limit and criteria-shape violations: the ones that need a payload
    /// big enough to be worth building separately.
    #[test]
    fn request_limits_report_typed_errors() {
        let limits = limits();

        let empty_criteria = json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "choice", "instructions": "x", "criteria": {}}}
        });
        assert_eq!(
            Jev.decode_request(&empty_criteria, &limits).unwrap_err().code(),
            "schema.invalid_value"
        );

        let one_level = json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "score", "instructions": "x", "criteria": {"only": "d"}}}
        });
        assert_eq!(
            Jev.decode_request(&one_level, &limits).unwrap_err().code(),
            "schema.invalid_value"
        );

        let illegal_label = json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "choice", "instructions": "x", "criteria": {"has space": "d"}}}
        });
        assert_eq!(
            Jev.decode_request(&illegal_label, &limits).unwrap_err().code(),
            "schema.invalid_value"
        );

        let too_many_questions: serde_json::Map<String, Value> = (0..9)
            .map(|index| (format!("q{index}"), json!({"type": "noul", "instructions": "x"})))
            .collect();
        let too_many = json!({"state": "s", "model": "m", "questions": too_many_questions});
        assert_eq!(
            Jev.decode_request(&too_many, &limits).unwrap_err().code(),
            "schema.limit_exceeded"
        );

        let many_candidates: serde_json::Map<String, Value> =
            (0..9).map(|index| (format!("c{index}"), Value::String("d".to_owned()))).collect();
        let big_choice = json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "choice", "instructions": "x", "criteria": many_candidates}}
        });
        assert_eq!(
            Jev.decode_request(&big_choice, &limits).unwrap_err().code(),
            "schema.limit_exceeded"
        );
    }

    #[test]
    fn choice_criteria_cap_is_enforced() {
        let entries: serde_json::Map<String, Value> = (0..=MAX_CRITERIA)
            .map(|index| (format!("c{index}"), Value::String("d".to_owned())))
            .collect();
        let payload = json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "choice", "instructions": "x", "criteria": entries}}
        });
        let generous = Limits { max_candidates: MAX_CRITERIA * 2, ..Limits::default() };
        let error = Jev.decode_request(&payload, &generous).unwrap_err();
        assert!(
            matches!(error, SchemaError::LimitExceeded { limit: "max_criteria", .. }),
            "{error}"
        );
    }

    #[test]
    fn malformed_responses_report_typed_errors() {
        let choice_request = Jev.decode_request_str(&choice_request_text(), &limits()).unwrap();

        let missing_answers = json!({"model": "m"});
        let error = Jev.decode_response(&missing_answers, &choice_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");

        let missing_answer = json!({"answers": {}});
        let error = Jev.decode_response(&missing_answer, &choice_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert!(error.to_string().contains("model"), "{error}");

        let unknown_candidate = json!({"answers": {"model": "gpt-9"}});
        let error =
            Jev.decode_response(&unknown_candidate, &choice_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let wrong_type = json!({"answers": {"model": 7}});
        let error = Jev.decode_response(&wrong_type, &choice_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let payload_not_object = json!([]);
        let error =
            Jev.decode_response(&payload_not_object, &choice_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        assert_eq!(
            Jev.decode_response_str("nope", &choice_request, &limits()).unwrap_err().code(),
            "schema.invalid_json"
        );

        let score_request = Jev.decode_request_str(&score_request_text(), &limits()).unwrap();
        let partial = json!({"answers": {"difficulty": {"0": 1.0}}});
        let error = Jev.decode_response(&partial, &score_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.missing_field");
        assert!(error.to_string().contains("difficulty.1"), "{error}");

        let not_numbers = json!({"answers": {"difficulty": {"0": "high", "1": 0.0, "2": 0.0}}});
        let error = Jev.decode_response(&not_numbers, &score_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let out_of_range = json!({"answers": {"difficulty": {"0": 1.5, "1": 0.0, "2": 0.0}}});
        let error = Jev.decode_response(&out_of_range, &score_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let unnormalized = json!({"answers": {"difficulty": {"0": 0.5, "1": 0.5, "2": 0.5}}});
        let error = Jev.decode_response(&unnormalized, &score_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");

        let noul_request = Jev.decode_request_str(&noul_request_text(), &limits()).unwrap();
        let wrong_boolean = json!({"answers": {"needs_tools": "yes"}});
        let error = Jev.decode_response(&wrong_boolean, &noul_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_type");

        let out_of_range_boolean = json!({"answers": {"needs_tools": 1.4}});
        let error =
            Jev.decode_response(&out_of_range_boolean, &noul_request, &limits()).unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
    }

    #[test]
    fn ordered_map_keeps_document_order_and_rejects_duplicates() {
        let map: OrderedMap = serde_json::from_str(r#"{"b":"2","a":"1","c":"3"}"#).unwrap();
        let keys: Vec<&str> = map.0.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["b", "a", "c"]);
        assert_eq!(serde_json::to_string(&map).unwrap(), r#"{"b":"2","a":"1","c":"3"}"#);

        assert!(serde_json::from_str::<OrderedMap>("[]").is_err());
        assert!(serde_json::from_str::<OrderedMap>(r#"{"a":1}"#).is_err());
        assert!(serde_json::from_str::<OrderedMap>(r#"{"a":"1","a":"2"}"#).is_err());
    }

    #[test]
    fn duplicate_question_ids_are_rejected() {
        let text = r#"{"state":"s","model":"m","questions":{"q":{"type":"noul","instructions":"x"},"q":{"type":"noul","instructions":"y"}}}"#;
        assert_eq!(
            Jev.decode_request_str(text, &limits()).unwrap_err().code(),
            "schema.invalid_json"
        );
    }

    #[test]
    fn report_gates_on_the_weakest_answer() {
        let answers = vec![
            DecisionAnswer::Boolean {
                question_id: QuestionId::new("a").unwrap(),
                value: true,
                probability: 0.55,
                confidence: 0.55,
            },
            DecisionAnswer::Boolean {
                question_id: QuestionId::new("b").unwrap(),
                value: false,
                probability: 0.99,
                confidence: 0.99,
            },
        ];
        let report = report_for(&answers);
        assert!((report.calibrated_confidence - 0.55).abs() < 1e-9);
        assert!((report.top_probability - 0.55).abs() < 1e-9);
        assert!((report.margin - 0.55).abs() < 1e-9);
        assert!((report.entropy - 0.0).abs() < 1e-9);
        assert_eq!(report.ood_score, 0.0);
        assert_eq!(report.verifier_agreement, None);
    }

    #[test]
    fn request_wire_is_debug_and_clone() {
        let text = noul_request_text();
        let wire: RequestWire = serde_json::from_str(&text).unwrap();
        assert_eq!(wire.clone(), wire);
        assert!(!format!("{wire:?}").is_empty());
    }
}
