//! The escalation report: the engine's partial work handed to a host
//! model when it refuses to answer alone (`ESCALATION_LANES_PLAN.md` §2.2,
//! task #189).
//!
//! Four hard rules govern everything in this module:
//!
//! 1. **No chain of thought.** [`EscalationPayload::trace`] is the
//!    deterministic execution trace ([`TraceEntry`]), never model
//!    reasoning — the repo's own trace contract, carried verbatim.
//! 2. **Bounded.** The report rides another model's context, so
//!    [`EscalationReport::fit_to`] shrinks it to a byte budget by a
//!    fixed cut order and marks `truncated` — never silently.
//! 3. **Self-contained.** Question, state summary, evidence, and
//!    guidance together answer "what was asked and what does the engine
//!    already know" without a second round trip.
//! 4. **Deterministic.** Same request + same engine version →
//!    byte-identical report: every field is a pure function of the
//!    decision, and the trace detail map is sorted ([`TraceEntry`]).

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, is_unit_interval};
use crate::ids::CandidateId;
use crate::trace::TraceEntry;

/// Default byte budget for a serialized escalation report: large enough
/// for the fixed skeleton plus evidence on every decision the ladder
/// can produce, small enough to sit inside any model's context without
/// negotiation. Callers with a narrower budget pass it to
/// [`EscalationReport::fit_to`] explicitly.
pub const DEFAULT_ESCALATION_BUDGET_BYTES: usize = 65_536;

/// Why the engine escalated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EscalationReason {
    /// Calibrated confidence landed in the escalation band
    /// (`abstain_below <= c < escalate_below`) — the band of
    /// `ESCALATION_LANES_PLAN.md` §2.1.
    BelowConfidence,
}

/// Which gate measured the escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EscalationGateKind {
    /// The confidence band (`threshold` = `escalate_below`, `observed` =
    /// the calibrated confidence that landed below it).
    Confidence,
}

/// The action the report asks of its reader.
///
/// The report never acts itself: the *client* owns the host model
/// (§2.3 — the delegate surface), so the ask is declarative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EscalationAction {
    /// Answer the question from the payload; say `unknown` if the state
    /// does not settle it.
    AskTheHostModel,
}

/// The gate that fired: what kind, at what threshold, observed at what
/// value. Carried so the reader can distinguish "barely outside the
/// band" from "nowhere near deciding" without re-deriving anything.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EscalationGate {
    /// Which gate kind fired.
    pub kind: EscalationGateKind,
    /// The configured threshold the observation fell outside of.
    pub threshold: f64,
    /// The observed value (the calibrated confidence for the
    /// confidence gate).
    pub observed: f64,
}

/// Which engine produced the report — the same provenance the cache
/// key decorates with (`EngineIdentity` in the engine crate), flattened
/// so the reader can tell what it is being handed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineProvenance {
    /// Identifier of the deciding rung (e.g. `builtin-lexical-v2`).
    pub model_id: String,
    /// Version of the executed decision graph.
    pub graph_version: u64,
    /// Version of the calibration mapping in force.
    pub calibration_version: u64,
    /// Engine semver.
    pub engine_semver: String,
}

/// The question as the host model must see it: kind + text, no second
/// lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationQuestion {
    /// The question kind (`choice`, `boolean`, `score`, ...).
    pub kind: String,
    /// The question text, verbatim.
    pub text: String,
}

/// How much state stood behind the decision — enough for the reader to
/// judge whether the state can settle the question, without shipping
/// the state itself twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSummary {
    /// Bytes of state text.
    pub text_bytes: usize,
    /// Facts present in the state.
    pub fact_count: usize,
    /// Images present in the state.
    pub image_count: usize,
}

/// One candidate with why the engine put it on its side of the line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateEvidence {
    /// The candidate.
    pub id: CandidateId,
    /// Deterministic reason (rule matched, metadata filter, ...) —
    /// engine output, never model reasoning.
    pub why: String,
}

/// An answer value in the IR's typed form — untagged on the wire, so a
/// boolean serializes as `false` and a choice as its candidate id
/// string: exactly the flat shapes `ESCALATION_LANES_PLAN.md` §2.2 shows,
/// without free-form JSON in the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PartialAnswerValue {
    /// A boolean answer.
    Boolean(bool),
    /// A text answer: a choice candidate id or a score level label.
    Text(String),
}

/// The engine's current best answer, explicitly *not accepted*: kept
/// with its probability so the host model can build on it instead of
/// starting cold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartialAnswer {
    /// The answer value.
    pub value: PartialAnswerValue,
    /// Its probability, in `[0, 1]`.
    pub probability: f64,
}

/// The evidence bundle. All variable-length parts are subject to
/// [`EscalationReport::fit_to`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationPayload {
    /// What was asked.
    pub question: EscalationQuestion,
    /// How much state stood behind the decision.
    pub state_summary: StateSummary,
    /// Candidates the engine kept, with reasons.
    pub candidates_ruled_in: Vec<CandidateEvidence>,
    /// Candidates the engine eliminated, with reasons.
    pub candidates_ruled_out: Vec<CandidateEvidence>,
    /// The unaccepted best answer, when one exists.
    pub partial_answer: Option<PartialAnswer>,
    /// The deterministic execution trace (rule 1: engine trace, never
    /// chain of thought).
    pub trace: Vec<TraceEntry>,
    /// Fixed handoff instruction for the host model.
    pub guidance: String,
    /// `true` when [`EscalationReport::fit_to`] found the report over
    /// budget — it then cuts variable parts from the tail (explicit,
    /// never silent; rule 2). A degenerate report with nothing cuttable
    /// keeps its content and the marker anyway: the flag means "this
    /// report was too big for its budget", which is the fact a reader
    /// needs.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

/// Serde's `skip_serializing_if` requires the `&bool` signature; the
/// reference is what the derive calls.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// The complete handoff document (the `escalation` block of the native
/// response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationReport {
    /// Why the engine escalated.
    pub reason: EscalationReason,
    /// The gate that fired.
    pub gate: EscalationGate,
    /// What the report asks of its reader.
    pub requested_action: EscalationAction,
    /// Which engine produced it.
    pub engine: EngineProvenance,
    /// The evidence bundle.
    pub payload: EscalationPayload,
}

impl EscalationReport {
    /// Checks the report's invariants: gate coherence and range,
    /// non-empty strings, probabilities in `[0, 1]`.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidEscalation`] when a field is out of range or
    /// empty — the report is about to cross a wire into another
    /// model's context, so a malformed one is refused, not repaired.
    pub fn validate(&self) -> CoreResult<()> {
        let invalid = |reason: String| CoreError::InvalidEscalation { reason };
        if !is_unit_interval(self.gate.threshold) {
            return Err(invalid(format!(
                "gate threshold must be a finite value in [0, 1], got {}",
                self.gate.threshold
            )));
        }
        if !is_unit_interval(self.gate.observed) {
            return Err(invalid(format!(
                "gate observation must be a finite value in [0, 1], got {}",
                self.gate.observed
            )));
        }
        // The confidence band is the only reason today; coherence holds
        // trivially, and future reasons must match their gate kind.
        if matches!(self.reason, EscalationReason::BelowConfidence)
            && !matches!(self.gate.kind, EscalationGateKind::Confidence)
        {
            return Err(invalid(
                "reason `below_confidence` requires gate kind `confidence`".to_owned(),
            ));
        }
        if self.payload.question.kind.is_empty() {
            return Err(invalid("question kind must not be empty".to_owned()));
        }
        if self.payload.question.text.is_empty() {
            return Err(invalid("question text must not be empty".to_owned()));
        }
        if self.payload.guidance.is_empty() {
            return Err(invalid("guidance must not be empty".to_owned()));
        }
        if self.engine.model_id.is_empty() {
            return Err(invalid("engine model_id must not be empty".to_owned()));
        }
        if self.engine.engine_semver.is_empty() {
            return Err(invalid("engine engine_semver must not be empty".to_owned()));
        }
        for evidence in
            self.payload.candidates_ruled_in.iter().chain(&self.payload.candidates_ruled_out)
        {
            if evidence.why.is_empty() {
                return Err(invalid(format!(
                    "evidence for candidate `{}` must say why",
                    evidence.id
                )));
            }
        }
        if let Some(partial) = &self.payload.partial_answer
            && !is_unit_interval(partial.probability)
        {
            return Err(invalid(format!(
                "partial answer probability must be a finite value in [0, 1], got {}",
                partial.probability
            )));
        }
        Ok(())
    }

    /// The report's serialized size in bytes.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
    }

    /// Shrinks the report to at most `max_bytes` of canonical JSON,
    /// cutting in one fixed order — trace tail, ruled-out tail, ruled-in
    /// tail, then guidance (at a UTF-8 char boundary) — so the same
    /// input always truncates to the same output (rule 2 + rule 4).
    /// Sets [`EscalationPayload::truncated`] when anything was cut.
    ///
    /// If `max_bytes` cannot fit even the fixed skeleton after every
    /// variable part is exhausted, the report keeps its smallest legal
    /// form and may still exceed the budget: pass a ceiling above a few
    /// hundred bytes (the skeleton's size) — `Limits`-family budgets
    /// always are.
    pub fn fit_to(&mut self, max_bytes: usize) {
        if self.serialized_len() <= max_bytes {
            return;
        }
        self.payload.truncated = true;
        while self.serialized_len() > max_bytes && !self.payload.trace.is_empty() {
            self.payload.trace.pop();
        }
        while self.serialized_len() > max_bytes && !self.payload.candidates_ruled_out.is_empty() {
            self.payload.candidates_ruled_out.pop();
        }
        while self.serialized_len() > max_bytes && !self.payload.candidates_ruled_in.is_empty() {
            self.payload.candidates_ruled_in.pop();
        }
        if self.serialized_len() <= max_bytes {
            return;
        }
        // Guidance last: cut from the tail at a UTF-8 char boundary,
        // never to empty — `validate()` requires guidance, so an
        // impossible budget over-runs rather than producing an invalid
        // report (the doc contract above).
        loop {
            let len = self.payload.guidance.len();
            if len == 0 {
                break;
            }
            let mut cut = (len / 8).max(1);
            // Nearest boundary at or before `len - cut`; increasing
            // `cut` walks toward position 0, which is always a boundary,
            // so this terminates.
            while cut < len && !self.payload.guidance.is_char_boundary(len - cut) {
                cut += 1;
            }
            if cut >= len {
                // The only remaining cut would empty the guidance.
                break;
            }
            self.payload.guidance.truncate(len - cut);
            if self.serialized_len() <= max_bytes {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn sample() -> EscalationReport {
        EscalationReport {
            reason: EscalationReason::BelowConfidence,
            gate: EscalationGate {
                kind: EscalationGateKind::Confidence,
                threshold: 0.62,
                observed: 0.31,
            },
            requested_action: EscalationAction::AskTheHostModel,
            engine: EngineProvenance {
                model_id: "builtin-lexical-v2".to_owned(),
                graph_version: 7,
                calibration_version: 3,
                engine_semver: "0.6.1".to_owned(),
            },
            payload: EscalationPayload {
                question: EscalationQuestion {
                    kind: "boolean".to_owned(),
                    text: "Is the sky blue?".to_owned(),
                },
                state_summary: StateSummary { text_bytes: 41, fact_count: 3, image_count: 0 },
                candidates_ruled_in: vec![],
                candidates_ruled_out: vec![],
                partial_answer: Some(PartialAnswer {
                    value: PartialAnswerValue::Boolean(false),
                    probability: 0.31,
                }),
                trace: vec![],
                guidance: "Answer from the state above; say 'unknown' if the state does not \
                           settle it."
                    .to_owned(),
                truncated: false,
            },
        }
    }

    #[test]
    fn validates_the_shipped_shape() {
        sample().validate().expect("valid report");
    }

    #[test]
    fn rejects_out_of_range_gate_values() {
        let mut bad = sample();
        bad.gate.observed = 1.5;
        assert!(matches!(bad.validate(), Err(CoreError::InvalidEscalation { .. })));

        let mut bad = sample();
        bad.gate.threshold = f64::NAN;
        assert!(matches!(bad.validate(), Err(CoreError::InvalidEscalation { .. })));
    }

    #[test]
    fn rejects_empty_guidance_and_evidence_without_reason() {
        let mut bad = sample();
        bad.payload.guidance.clear();
        assert!(matches!(bad.validate(), Err(CoreError::InvalidEscalation { .. })));

        let mut bad = sample();
        bad.payload.candidates_ruled_out.push(CandidateEvidence {
            id: crate::ids::CandidateId::new("b").expect("valid id"),
            why: String::new(),
        });
        assert!(matches!(bad.validate(), Err(CoreError::InvalidEscalation { .. })));
    }

    #[test]
    fn truncated_marker_omitted_until_a_cut_is_made() {
        let mut report = sample();
        let plain = serde_json::to_string(&report).expect("serialize");
        assert!(!plain.contains("truncated"), "absent-on-default marker: {plain}");

        report.fit_to(16); // below the skeleton: forces the guidance cut path
        let cut = serde_json::to_string(&report).expect("serialize");
        assert!(cut.contains("\"truncated\":true"), "explicit marker: {cut}");
        assert!(report.payload.guidance.len() < sample().payload.guidance.len());
    }

    #[test]
    fn fit_to_is_deterministic() {
        let mut one = sample();
        let mut two = sample();
        // Give both a real payload to cut.
        for r in [&mut one, &mut two] {
            for i in 0..40 {
                r.payload.trace.push(TraceEntry::new(
                    "narrow",
                    [("out", crate::state::FactValue::Text(format!("n-{i}")))],
                ));
                r.payload.candidates_ruled_out.push(CandidateEvidence {
                    id: crate::ids::CandidateId::new(format!("c-{i}")).expect("valid id"),
                    why: format!("filtered by metadata: tier {i}"),
                });
            }
        }
        one.fit_to(2_048);
        two.fit_to(2_048);
        assert_eq!(one, two, "same input truncates to the same output");
        assert!(one.serialized_len() <= 2_048);
        assert!(one.payload.truncated);
        // A budget that already fits cuts nothing.
        let mut three = sample();
        three.fit_to(DEFAULT_ESCALATION_BUDGET_BYTES);
        assert!(!three.payload.truncated);
        assert_eq!(three, sample());
    }

    #[test]
    fn impossible_budget_keeps_the_report_valid() {
        // A budget no serialization can meet cuts every variable part
        // it may cut, keeps guidance non-empty (validate's floor), and
        // over-runs — never an invalid report, never a silent marker.
        let mut report = sample();
        report.fit_to(1);
        assert!(report.payload.truncated);
        assert_ne!(report.payload.guidance, "");
        report.validate().expect("still valid after an impossible cut");
    }
}
