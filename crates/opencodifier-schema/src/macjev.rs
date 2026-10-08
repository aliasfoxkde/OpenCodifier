//! The macjev readout renderer (`macjev-render-v1`) — the single
//! byte-exact definition of the prompt text the decision-model rungs
//! train and serve on (TRAINING.md §9.7 Phase F2).
//!
//! Until now the layout lived in two hand-synced implementations: the
//! python prep (`benchmarks/decision-model/runner/decision_sft_prep.py`,
//! which built every training corpus) and the llama.cpp fork's tree-mode
//! prompt builder (which renders what the serving rung sees). Both were
//! validated against each other by measurement only. This module is the
//! workspace-owned definition: the same record in, the same bytes out,
//! pinned against the python renderer by `tests/macjev_parity.rs` (a
//! committed corpus sample with python-derived expected bytes runs in
//! plain CI; `just check-macjev` re-derives them from the live python
//! implementation on the host lane).
//!
//! # Layout
//!
//! ```text
//! State:
//! <state>
//!
//! Question [<qtype>]: <instructions>
//! Options:
//! - <option 1>
//! ...
//! Judge each option:
//! <option 1> -> yes|no
//! ...
//! ```
//!
//! Supervision is one verdict token (` yes`/` no`) per option slot, each
//! its own supervised segment, exactly as the trainer's loss mask
//! expects.
//!
//! # Python-semantics boundaries
//!
//! The renderer ports the prep's exact string semantics — python's
//! `strip()`/`rstrip()` whitespace class, `json.dumps(...,
//! ensure_ascii=False)` object spacing and document member order —
//! because the corpora were built with those bytes. Four places where
//! the prep coerces via `str(...)` (`criteria` values — choice
//! candidates, noul verdict sides, score levels —, question
//! instructions, target labels of non-string JSON type) are treated as
//! unrenderable/skipped here instead of re-implementing python `repr`:
//! the merged-v3 corpus carries strings (labels may be bare score
//! integers, which are handled), so the cases never arise in real data,
//! and no serving path ever reproduced python `repr` bytes.
//!
//! The renderer never panics: malformed records are a typed
//! [`MacjevError`]; everything else is a typed skip, mirroring the
//! prep's count buckets.

use std::fmt;

use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

/// A question family the macjev renderer knows how to lay out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacjevKind {
    /// Select one candidate: options render as `<id>: <description>`.
    Choice,
    /// Two-sided verdict: options render as `false: <c>` then
    /// `true: <c>`, in that order.
    Noul,
    /// Ordered scale: options render as `level <i>: <description>`.
    Score,
}

impl MacjevKind {
    /// The question-type string the corpus records and the prep carry.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Noul => "noul",
            Self::Score => "score",
        }
    }

    /// Parses a record's question-type string; `None` for families this
    /// renderer does not lay out (they skip, as in the prep).
    #[must_use]
    pub fn parse(qtype: &str) -> Option<Self> {
        match qtype {
            "choice" => Some(Self::Choice),
            "noul" => Some(Self::Noul),
            "score" => Some(Self::Score),
            _ => None,
        }
    }
}

impl fmt::Display for MacjevKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rendered macjev segment: prompt text plus its loss-mask flag.
/// `supervised` segments are the verdict tokens the trainer scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacjevSegment {
    /// The segment's exact text, concatenated in order to form the
    /// full prompt.
    pub text: String,
    /// Whether the verdict token inside this segment carries the loss.
    pub supervised: bool,
}

/// One rendered question: the prep's row shape minus the source
/// bookkeeping the trainer does not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacjevRow {
    /// `<record_id>::<question name>` — the prep's row identity.
    pub id: String,
    /// The question's name inside the record's `request.questions`.
    pub question: String,
    /// The question family.
    pub qtype: MacjevKind,
    /// The target label (an option id: candidate id, `false`/`true`,
    /// or a level index as a decimal string).
    pub label: String,
    /// Number of option slots (== supervised segment count).
    pub n_options: usize,
    /// Header segment (unsupervised) plus one verdict segment per
    /// slot, in render order.
    pub segments: Vec<MacjevSegment>,
    /// Total character length of every segment concatenated.
    pub render_chars: usize,
}

impl MacjevRow {
    /// The full prompt text: header plus every segment concatenated.
    #[must_use]
    pub fn full_text(&self) -> String {
        let mut text = String::with_capacity(self.render_chars);
        for segment in &self.segments {
            text.push_str(&segment.text);
        }
        text
    }
}

/// Why a question inside a renderable record produced no row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacjevSkip {
    /// The question's `criteria` are missing, malformed, below the
    /// two-option floor, or of a non-string shape this renderer does
    /// not lay out.
    UnrenderableCriteria,
    /// The record's target label is absent from the rendered option
    /// ids (or the target's type disagrees with the question's).
    LabelOutsideOptions,
    /// The question's instructions strip to nothing.
    EmptyInstructions,
}

impl fmt::Display for MacjevSkip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnrenderableCriteria => f.write_str("unrenderable criteria"),
            Self::LabelOutsideOptions => f.write_str("target label outside options"),
            Self::EmptyInstructions => f.write_str("empty instructions"),
        }
    }
}

/// The outcome of rendering one corpus record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacjevOutcome {
    /// `source` starts with `suite` — suite items never enter training
    /// corpora (the suite stays the judge).
    SuiteRow,
    /// The record's state strips to nothing.
    EmptyState,
    /// The state exceeds `max_state_chars`; skipped and counted,
    /// never truncated.
    StateOverBudget {
        /// The state's length in characters (python `len`).
        state_chars: usize,
    },
    /// The record rendered; carries every row and any per-question
    /// skips, keyed by question name in document order.
    Rendered(MacjevRecordRender),
}

/// The rendered rows of one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacjevRecordRender {
    /// The state text as rendered (already stripped or python-dumped).
    pub state: String,
    /// One row per renderable question, in the record's document
    /// order.
    pub rows: Vec<MacjevRow>,
    /// Per-question skips in document order.
    pub skips: Vec<(String, MacjevSkip)>,
}

/// Why a record could not be rendered at all. Everything the prep
/// tolerates is a typed [`MacjevOutcome`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacjevError {
    /// The record line is not valid JSON or is not a JSON object.
    InvalidRecord {
        /// Where the record came from (line number or path, for the
        /// caller to label).
        origin: String,
    },
}

impl fmt::Display for MacjevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecord { origin } => {
                write!(f, "macjev.invalid_record: {origin} is not a JSON object")
            }
        }
    }
}

impl std::error::Error for MacjevError {}

/// An order-preserving JSON value: the minimal DOM the renderer needs.
/// Object member order is the document's, which is what python's dict
/// iteration (and therefore the prep's option order and `json.dumps`)
/// preserves — `serde_json::Map` alone would sort it away.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Jv {
    /// A number, boolean, or null, kept as raw text so re-emission
    /// matches the source bytes (python round-trips these exactly).
    Scalar(String),
    /// A JSON string (decoded).
    Str(String),
    /// An array in document order.
    Arr(Vec<Jv>),
    /// An object's members in document order.
    Obj(Vec<(String, Jv)>),
}

impl Jv {
    fn get(&self, key: &str) -> Option<&Jv> {
        match self {
            Jv::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Jv::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The python `str(...)` of a scalar-string-or-int label: the
    /// corpus's score labels arrive as bare integers, the rest as
    /// strings. Anything else is outside the renderer's contract.
    fn label_text(&self) -> Option<String> {
        match self {
            Jv::Str(s) => Some(s.clone()),
            Jv::Scalar(raw) if raw.chars().all(|c| c.is_ascii_digit()) => Some(raw.clone()),
            _ => None,
        }
    }

    fn is_empty_obj(&self) -> bool {
        matches!(self, Jv::Obj(members) if members.is_empty())
    }
}

/// Parses a JSON document into the order-preserving DOM: member order
/// is the document's at every depth, which is what python's dict
/// iteration (and therefore the prep's option order and `json.dumps`)
/// preserves.
impl<'de> serde::Deserialize<'de> for Jv {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JvVisitor)
    }
}

/// Builds [`Jv`] values from any JSON shape, collecting objects and
/// arrays through the `MapAccess`/`SeqAccess` order.
struct JvVisitor;

impl<'de> Visitor<'de> for JvVisitor {
    type Value = Jv;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Jv, E> {
        Ok(Jv::Scalar(if value { "true".to_owned() } else { "false".to_owned() }))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Jv, E> {
        Ok(Jv::Scalar(value.to_string()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Jv, E> {
        Ok(Jv::Scalar(value.to_string()))
    }

    /// Floats never occur in the corpora's structured states; they
    /// render as Rust's shortest round-trip form, which diverges from
    /// python `repr` in exponent notation — a documented boundary of
    /// the byte-parity claim, not a panic path.
    fn visit_f64<E>(self, value: f64) -> Result<Jv, E> {
        Ok(Jv::Scalar(format!("{value}")))
    }

    fn visit_unit<E>(self) -> Result<Jv, E> {
        Ok(Jv::Scalar("null".to_owned()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Jv, E> {
        Ok(Jv::Str(value.to_owned()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Jv, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = access.next_element::<Jv>()? {
            items.push(item);
        }
        Ok(Jv::Arr(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Jv, A::Error> {
        let mut members = Vec::new();
        while let Some((key, value)) = access.next_entry::<String, Jv>()? {
            members.push((key, value));
        }
        Ok(Jv::Obj(members))
    }
}

/// Python's `str.strip()` whitespace class — exactly
/// `" \t\n\r\v\f"`, deliberately narrower than Rust's
/// `char::is_whitespace` (no NBSP, no line/paragraph separators).
fn py_strip(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\n', '\r', '\u{000b}', '\u{000c}'])
}

/// Python's `str.rstrip()` over the same whitespace class.
fn py_rstrip(text: &str) -> &str {
    text.trim_end_matches([' ', '\t', '\n', '\r', '\u{000b}', '\u{000c}'])
}

/// A JSON string the way python's `json.dumps(..., ensure_ascii=False)`
/// emits it: raw UTF-8, short escapes for `\b\t\n\f\r`, `\u00xx` for
/// the remaining control characters. `serde_json`'s string emitter
/// matches byte-for-byte on every input class python's does.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// Python's `json.dumps(value, ensure_ascii=False)`: `", "` between
/// members, `": "` after keys, document member order, no trailing
/// spaces inside empty containers.
fn py_dumps(value: &Jv) -> String {
    match value {
        Jv::Obj(members) if members.is_empty() => "{}".to_owned(),
        Jv::Obj(members) => {
            let rendered: Vec<String> = members
                .iter()
                .map(|(key, value)| format!("{}: {}", json_string(key), py_dumps(value)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
        Jv::Arr(items) if items.is_empty() => "[]".to_owned(),
        Jv::Arr(items) => {
            let rendered: Vec<String> = items.iter().map(py_dumps).collect();
            format!("[{}]", rendered.join(", "))
        }
        Jv::Str(text) => json_string(text),
        Jv::Scalar(raw) => raw.clone(),
    }
}

/// The prep's `state_text`: the record's state as plain string, a
/// `{"text": ...}` wrapper, or a structured dict serialized exactly
/// the way the serving adapter does.
fn state_text(record: &Jv) -> String {
    let Some(state) = record.get("request").and_then(|request| request.get("state")) else {
        return String::new();
    };
    match state {
        Jv::Str(text) => py_strip(text).to_owned(),
        Jv::Obj(members) => {
            if let Some(Jv::Str(text)) = members.iter().find(|(k, _)| k == "text").map(|(_, v)| v) {
                let stripped = py_strip(text);
                if !stripped.is_empty() {
                    return stripped.to_owned();
                }
            }
            if state.is_empty_obj() { String::new() } else { py_dumps(state) }
        }
        _ => String::new(),
    }
}

/// One option slot: the option line as rendered (without the
/// `"- "` bullet) and its id in the verdict distribution.
struct MacjevSlot {
    id: String,
    line: String,
}

/// The prep's `options_for` + `option_ids`: the option slots in
/// render order, or `None` when the criteria are not renderable.
fn option_slots(qtype: MacjevKind, criteria: Option<&Jv>) -> Option<Vec<MacjevSlot>> {
    match qtype {
        MacjevKind::Choice => {
            let Jv::Obj(candidates) = criteria? else { return None };
            if candidates.len() < 2 {
                return None;
            }
            let mut slots = Vec::with_capacity(candidates.len());
            for (id, description) in candidates {
                let text = description.as_str()?;
                slots
                    .push(MacjevSlot { id: id.clone(), line: format!("{id}: {}", py_strip(text)) });
            }
            Some(slots)
        }
        MacjevKind::Noul => {
            let empty = Jv::Obj(Vec::new());
            let criteria = match criteria {
                // A missing or non-object `criteria` renders both sides
                // empty, as the prep's `isinstance(criteria, dict)`
                // guard does.
                Some(criteria @ Jv::Obj(_)) => criteria,
                _ => &empty,
            };
            let side = |key: &str| -> Option<String> {
                match criteria.get(key) {
                    Some(Jv::Str(text)) => Some(py_strip(text).to_owned()),
                    // Absent or `null` sides are falsy in the prep and
                    // coerce to the empty string.
                    None => Some(String::new()),
                    Some(Jv::Scalar(raw)) if raw == "null" => Some(String::new()),
                    // The prep `str(...)`-coerces truthy non-strings
                    // (python `repr` bytes); like the other families,
                    // that shape is unrenderable here.
                    Some(_) => None,
                }
            };
            let held = side("true")?;
            let not_held = side("false")?;
            Some(vec![
                MacjevSlot {
                    id: "false".to_owned(),
                    line: py_rstrip(&format!("false: {not_held}")).to_owned(),
                },
                MacjevSlot {
                    id: "true".to_owned(),
                    line: py_rstrip(&format!("true: {held}")).to_owned(),
                },
            ])
        }
        MacjevKind::Score => {
            let Jv::Arr(levels) = criteria? else { return None };
            if levels.len() < 2 {
                return None;
            }
            let mut slots = Vec::with_capacity(levels.len());
            for (index, level) in levels.iter().enumerate() {
                let text = level.as_str()?;
                slots.push(MacjevSlot {
                    id: index.to_string(),
                    line: format!("level {index}: {}", py_strip(text)),
                });
            }
            Some(slots)
        }
    }
}

/// The prep's `render_header`: everything before the first verdict
/// slot.
fn render_header(
    qtype: MacjevKind,
    instructions: &str,
    slots: &[MacjevSlot],
    state: &str,
) -> String {
    let options: Vec<String> = slots.iter().map(|slot| format!("- {}", slot.line)).collect();
    format!(
        "State:\n{state}\n\nQuestion [{}]: {instructions}\nOptions:\n{}\nJudge each option:\n",
        qtype.as_str(),
        options.join("\n")
    )
}

/// Renders one question of a record into its prep row, or `None` when
/// the question skips.
fn render_question(
    record_id: &str,
    name: &str,
    question: &Jv,
    target: Option<&Jv>,
    state: &str,
    skips: &mut Vec<(String, MacjevSkip)>,
) -> Option<MacjevRow> {
    let qtype_str = question.get("type").and_then(Jv::as_str).unwrap_or_default();
    let Some(qtype) = MacjevKind::parse(qtype_str) else {
        skips.push((name.to_owned(), MacjevSkip::UnrenderableCriteria));
        return None;
    };
    let Some(slots) = option_slots(qtype, question.get("criteria")) else {
        skips.push((name.to_owned(), MacjevSkip::UnrenderableCriteria));
        return None;
    };
    let target_type = target.and_then(|target| target.get("type")).and_then(Jv::as_str);
    let label = target.and_then(|target| target.get("label")).and_then(Jv::label_text);
    let type_matches = target_type.is_some_and(|wire| MacjevKind::parse(wire) == Some(qtype));
    let label_in_slots =
        label.as_ref().is_some_and(|label| slots.iter().any(|slot| &slot.id == label));
    if !type_matches || !label_in_slots {
        skips.push((name.to_owned(), MacjevSkip::LabelOutsideOptions));
        return None;
    }
    let Some(instructions) = question.get("instructions").and_then(Jv::as_str).map(py_strip) else {
        skips.push((name.to_owned(), MacjevSkip::EmptyInstructions));
        return None;
    };
    if instructions.is_empty() {
        skips.push((name.to_owned(), MacjevSkip::EmptyInstructions));
        return None;
    }
    let label = label.unwrap_or_default();
    let yes_at = slots.iter().position(|slot| slot.id == label);
    let mut segments = Vec::with_capacity(slots.len() * 2 + 1);
    segments.push(MacjevSegment {
        text: render_header(qtype, instructions, &slots, state),
        supervised: false,
    });
    for (index, slot) in slots.iter().enumerate() {
        let lead = if index == 0 { "" } else { "\n" };
        segments.push(MacjevSegment { text: format!("{lead}{} ->", slot.line), supervised: false });
        segments.push(MacjevSegment {
            text: if yes_at == Some(index) { " yes".to_owned() } else { " no".to_owned() },
            supervised: true,
        });
    }
    segments.push(MacjevSegment { text: "\n".to_owned(), supervised: false });
    let render_chars = segments.iter().map(|segment| segment.text.chars().count()).sum();
    Some(MacjevRow {
        id: format!("{record_id}::{name}"),
        question: name.to_owned(),
        qtype,
        label,
        n_options: slots.len(),
        segments,
        render_chars,
    })
}

/// Renders one corpus record line (the `decision_sft_prep` input
/// shape) into its macjev rows, or the typed skip that explains the
/// absence. `max_state_chars` mirrors the prep's `--max-state-chars`:
/// over-budget states skip the record, never truncate.
///
/// # Errors
///
/// Only a record line that is not a JSON object errors; every
/// content-level problem the prep tolerates is a typed outcome.
pub fn render_record(
    record_raw: &str,
    max_state_chars: usize,
) -> Result<MacjevOutcome, MacjevError> {
    let record: Jv = serde_json::from_str(record_raw)
        .map_err(|_| MacjevError::InvalidRecord { origin: "record line".to_owned() })?;
    if !matches!(record, Jv::Obj(_)) {
        return Err(MacjevError::InvalidRecord { origin: "record line".to_owned() });
    }
    let source = record.get("source").and_then(Jv::as_str).unwrap_or_default();
    if source.starts_with("suite") {
        return Ok(MacjevOutcome::SuiteRow);
    }
    let state = state_text(&record);
    if state.is_empty() {
        return Ok(MacjevOutcome::EmptyState);
    }
    let state_chars = state.chars().count();
    if state_chars > max_state_chars {
        return Ok(MacjevOutcome::StateOverBudget { state_chars });
    }
    let record_id = record.get("record_id").and_then(Jv::label_text).unwrap_or_default();
    let targets = record.get("target");
    let mut render = MacjevRecordRender { state, rows: Vec::new(), skips: Vec::new() };
    if let Some(Jv::Obj(questions)) =
        record.get("request").and_then(|request| request.get("questions"))
    {
        for (name, question) in questions {
            let target = targets.and_then(|targets| targets.get(name));
            if let Some(row) = render_question(
                &record_id,
                name,
                question,
                target,
                &render.state,
                &mut render.skips,
            ) {
                render.rows.push(row);
            }
        }
    }
    Ok(MacjevOutcome::Rendered(render))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::assert_is_empty)]

    use super::*;

    /// A minimal renderable choice record: `@state@`, `@criteria@`,
    /// and `@label@` are substituted, avoiding format-string brace
    /// escaping over JSON literals.
    fn choice_record(criteria: &str, label: &str, state: &str) -> String {
        r#"{"record_id": "r1", "source": "src", "request": {"state": @state@,
            "questions": {"q": {"type": "choice", "instructions": " pick one ",
            "criteria": @criteria@}}}, "target": {"q": {"type": "choice",
            "label": @label@}}}"#
            .replace("@state@", state)
            .replace("@criteria@", criteria)
            .replace("@label@", label)
            .replace('\n', "")
    }

    #[test]
    fn header_and_verdicts_match_the_prep_layout() {
        let record =
            choice_record(r#"{"zeta": "last", "alpha": "first"}"#, r#""alpha""#, r#""  s  ""#);
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.rows.len(), 1);
        let row = &render.rows[0];
        // State stripped; instructions stripped; document member order
        // (zeta first), never sorted.
        let text = row.full_text();
        let expected = concat!(
            "State:\ns\n\nQuestion [choice]: pick one\nOptions:\n",
            "- zeta: last\n- alpha: first\nJudge each option:\n",
            "zeta: last -> no\nalpha: first -> yes\n",
        );
        assert_eq!(text, expected);
        // One verdict per slot, exactly one yes, header unsupervised.
        assert_eq!(row.n_options, 2);
        assert_eq!(row.segments.iter().filter(|s| s.supervised).count(), 2);
        assert_eq!(row.segments.iter().filter(|s| s.supervised && s.text == " yes").count(), 1);
        assert!(!row.segments[0].supervised);
        assert_eq!(row.render_chars, text.chars().count());
    }

    #[test]
    fn structured_states_dump_with_python_spacing_and_document_order() {
        let record = choice_record(
            r#"{"beta": "b", "alpha": [1, true, null]}"#,
            r#""beta""#,
            r#"{"text": "", "agent": {"a": 1}}"#,
        );
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        // The blank `text` key does not short-circuit: the whole state
        // dict dumps, blank key included, in document order, with
        // `", "` / `": "` spacing (criteria never merge into it).
        assert_eq!(render.state, r#"{"text": "", "agent": {"a": 1}}"#);
    }

    #[test]
    fn strip_semantics_use_the_python_whitespace_class() {
        // NBSP and Unicode line separators are NOT python strip() class
        // members; only " \t\n\r\v\f" are.
        let record =
            choice_record(r#"{"a": "one", "b": "two"}"#, r#""a""#, "\"\u{00a0}x\u{2028}\"");
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.state, "\u{00a0}x\u{2028}");
    }

    #[test]
    fn noul_renders_false_first_with_rstripped_empty_sides() {
        let record = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {"q": {"type": "noul", "instructions": "i",
            "criteria": {"true": "held thing ", "false": ""}}}},
            "target": {"q": {"type": "noul", "label": "true"}}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        let text = render.rows[0].full_text();
        assert!(text.contains("- false:\n"), "empty side rstrips: {text:?}");
        assert!(text.contains("false: -> no\ntrue: held thing -> yes\n"), "{text:?}");
        assert_eq!(render.rows[0].label, "true");
    }

    #[test]
    fn score_levels_render_indexed_and_integer_labels_match() {
        let record = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {"q": {"type": "score", "instructions": "i",
            "criteria": ["low ", "mid", "high"]}}},
            "target": {"q": {"type": "score", "label": 2}}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.rows[0].label, "2");
        assert!(render.rows[0].full_text().contains("level 2: high -> yes\n"));
    }

    #[test]
    fn per_question_skips_are_typed_and_ordered() {
        let record = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {
              "bad": {"type": "choice", "instructions": "i", "criteria": {"only": "x"}},
              "mislabeled": {"type": "noul", "instructions": "i",
                "criteria": {"true": "t", "false": "f"}},
              "no_instr": {"type": "score", "instructions": "  ",
                "criteria": ["a", "b"]},
              "good": {"type": "choice", "instructions": "go",
                "criteria": {"x": "1", "y": "2"}}
            }},
            "target": {"mislabeled": {"type": "noul", "label": "maybe"},
              "no_instr": {"type": "score", "label": 0},
              "good": {"type": "choice", "label": "y"}}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.rows.len(), 1);
        assert_eq!(
            render.skips,
            vec![
                ("bad".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("mislabeled".to_owned(), MacjevSkip::LabelOutsideOptions),
                ("no_instr".to_owned(), MacjevSkip::EmptyInstructions),
            ]
        );
    }

    #[test]
    fn record_level_skips_and_errors() {
        let suite = r#"{"record_id": "s", "source": "suite:A-0000", "request": {"state": "x"}}"#;
        assert_eq!(render_record(suite, 24_000).unwrap(), MacjevOutcome::SuiteRow);
        let empty = r#"{"record_id": "e", "source": "src", "request": {"state": "   "}}"#;
        assert_eq!(render_record(empty, 24_000).unwrap(), MacjevOutcome::EmptyState);
        let long = r#"{"record_id": "l", "source": "src", "request": {"state": "abcdefgh"}}"#;
        assert_eq!(
            render_record(long, 4).unwrap(),
            MacjevOutcome::StateOverBudget { state_chars: 8 }
        );
        assert!(render_record("not json", 24_000).is_err());
        assert!(render_record("[1, 2]", 24_000).is_err());
    }

    #[test]
    fn unknown_question_families_skip() {
        let record = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {"prose": {"type": "essay", "instructions": "i",
            "criteria": {"a": "b"}}}}, "target": {}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.rows, [] as [MacjevRow; 0]);
        assert_eq!(render.skips, vec![("prose".to_owned(), MacjevSkip::UnrenderableCriteria)]);
    }

    #[test]
    fn display_impls_are_stable() {
        assert_eq!(MacjevKind::Choice.to_string(), "choice");
        assert_eq!(MacjevKind::Noul.to_string(), "noul");
        assert_eq!(MacjevKind::Score.to_string(), "score");
        assert_eq!(MacjevSkip::UnrenderableCriteria.to_string(), "unrenderable criteria");
        assert_eq!(MacjevSkip::LabelOutsideOptions.to_string(), "target label outside options");
        assert_eq!(MacjevSkip::EmptyInstructions.to_string(), "empty instructions");
        let error = render_record("[1, 2]", 24_000).unwrap_err();
        assert_eq!(error.to_string(), "macjev.invalid_record: record line is not a JSON object");
    }

    #[test]
    fn state_semantics_follow_the_prep_exactly() {
        let wrapper_state =
            choice_record(r#"{"a": "b"}"#, r#""a""#, r#"{"text": "  hi  ", "keep": 1}"#);
        let MacjevOutcome::Rendered(render) = render_record(&wrapper_state, 24_000).unwrap() else {
            panic!("record renders");
        };
        // A non-blank `text` key wins and is stripped; sibling keys are
        // dropped, exactly as the prep's early return does.
        assert_eq!(render.state, "hi");

        // Non-string, non-dict states, an empty dict, and a missing or
        // non-object request all strip to nothing -> EmptyState.
        for state in ["42", "true", "[1]", "null", "{}"] {
            let record = choice_record(r#"{"a": "b"}"#, r#""a""#, state);
            assert_eq!(
                render_record(&record, 24_000).unwrap(),
                MacjevOutcome::EmptyState,
                "state {state}"
            );
        }
        for record in [
            r#"{"record_id": "r", "source": "s"}"#,
            r#"{"record_id": "r", "source": "s", "request": {"questions": {}}}"#,
            r#"{"record_id": "r", "source": "s", "request": []}"#,
        ] {
            assert_eq!(
                render_record(record, 24_000).unwrap(),
                MacjevOutcome::EmptyState,
                "record {record}"
            );
        }

        // Negative integers and floats survive the dump through the
        // signed visitor (1.5 round-trips identically in both languages;
        // exponent forms are a documented boundary).
        let numbers = choice_record(r#"{"a": "b"}"#, r#""a""#, r#"{"delta": -5, "ratio": 1.5}"#);
        let MacjevOutcome::Rendered(render) = render_record(&numbers, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.state, r#"{"delta": -5, "ratio": 1.5}"#);
    }

    #[test]
    fn non_string_shapes_and_missing_keys_skip() {
        let record = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {
              "no_type": {"instructions": "i", "criteria": {"a": "x", "b": "y"}},
              "typed": {"type": 7, "instructions": "i",
                "criteria": {"a": "x", "b": "y"}},
              "desc_num": {"type": "choice", "instructions": "i",
                "criteria": {"a": "x", "b": 5}},
              "short_score": {"type": "score", "instructions": "i",
                "criteria": ["only"]},
              "level_num": {"type": "score", "instructions": "i",
                "criteria": ["low", 5]},
              "no_instr_key": {"type": "score", "criteria": ["a", "b"]}
            }},
            "target": {"no_instr_key": {"type": "score", "label": 0}}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(render.rows, [] as [MacjevRow; 0]);
        assert_eq!(
            render.skips,
            vec![
                ("no_type".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("typed".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("desc_num".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("short_score".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("level_num".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("no_instr_key".to_owned(), MacjevSkip::EmptyInstructions),
            ]
        );
    }

    #[test]
    fn noul_non_string_sides_skip_and_empty_criteria_render_blank() {
        // A truthy non-string side would python-`str()`-coerce; per the
        // documented boundary it skips instead of diverging silently.
        let skip_one = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {
              "num": {"type": "noul", "instructions": "i",
                "criteria": {"true": 5, "false": "f"}},
              "arr": {"type": "noul", "instructions": "i",
                "criteria": {"true": "t", "false": [1]}}
            }},
            "target": {}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&skip_one, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert_eq!(
            render.skips,
            vec![
                ("num".to_owned(), MacjevSkip::UnrenderableCriteria),
                ("arr".to_owned(), MacjevSkip::UnrenderableCriteria),
            ]
        );

        // Missing criteria entirely: both sides blank, still rendered.
        let blank = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
            "questions": {"q": {"type": "noul", "instructions": "i"}}},
            "target": {"q": {"type": "noul", "label": "false"}}}"#
            .replace('\n', "");
        let MacjevOutcome::Rendered(render) = render_record(&blank, 24_000).unwrap() else {
            panic!("record renders");
        };
        assert!(render.rows[0].full_text().contains("false: -> yes\ntrue: -> no\n"));
    }

    #[test]
    fn non_string_target_labels_skip() {
        for label in ["null", "-1", "1.5"] {
            let record = r#"{"record_id": "r1", "source": "src", "request": {"state": "s",
                "questions": {"q": {"type": "score", "instructions": "i",
                "criteria": ["low", "high"]}}},
                "target": {"q": {"type": "score", "label": @label@}}}"#
                .replace("@label@", label)
                .replace('\n', "");
            let MacjevOutcome::Rendered(render) = render_record(&record, 24_000).unwrap() else {
                panic!("record renders");
            };
            assert_eq!(
                render.skips,
                vec![("q".to_owned(), MacjevSkip::LabelOutsideOptions)],
                "label {label}"
            );
        }
    }

    #[test]
    fn a_record_without_questions_renders_an_empty_frame() {
        // `request.questions` absent (or not an object): the state still
        // renders, with no rows and no skips -- the `if let` fall-through
        // in `render_record`. The let-else stays on one line so the
        // refusal arm adds no never-executed line.
        let record =
            r#"{"record_id": "r1", "source": "src", "request": {"state": "the db is down"}}"#;
        let outcome = render_record(record, 24_000).unwrap();
        let MacjevOutcome::Rendered(render) = outcome else { panic!("not rendered: {outcome:?}") };
        assert_eq!(render.state, "the db is down");
        assert!(render.rows.is_empty());
        assert!(render.skips.is_empty());
    }
}
