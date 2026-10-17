//! Compute-host parity run for the Kai-0.6B ONNX rung (task #92).
//!
//! Closes the boundary D34 left open for Kai: the full Rust path —
//! request text ([`opencodifier_model::KaiQuestion`]), the reference HF
//! tokenizer, collate, the [`opencodifier_runtime::KaiOnnxBackend`]
//! graph execution on the real quantized export, and the contract's own
//! `probabilities` (softmax + fitted score bias) — against the
//! export-baked `ref.json` distributions. The python checker
//! (`kai_parity_check.py`) feeds the same fixture's pre-tokenized ids;
//! this runner rebuilds the ids from text first and asserts they agree.
//!
//! Usage (on the compute host):
//!
//! ```text
//! ORT_DYLIB_PATH=<libonnxruntime.so> cargo run --release -- \
//!     <model_quantized.onnx> <ref.json> <tokenizer.json>
//! ```

use std::collections::BTreeMap;
use std::process::ExitCode;

use opencodifier_model::{
    HfTokenizer, KaiEncoding, KaiOption, KaiPayload, KaiQuestion, MAX_INPUT_TOKENS, score_batch,
};
use opencodifier_runtime::KaiOnnxBackend;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct RefEntry {
    id: String,
    request: RefRequest,
    cases: Vec<RefCase>,
}

#[derive(Deserialize)]
struct RefRequest {
    state: Value,
    questions: BTreeMap<String, QuestionSpec>,
}

#[derive(Deserialize)]
struct QuestionSpec {
    instructions: String,
    criteria: Option<Value>,
}

#[derive(Deserialize)]
struct RefCase {
    question: String,
    kind: String,
    keys: Vec<String>,
    ids: Vec<i64>,
    option_pos: Vec<usize>,
    answer_pos: usize,
    answer: Value,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().ok_or("usage: kai-parity-rs <model.onnx> <ref.json> <tokenizer.json>")?;
    let ref_path = args.next().ok_or("missing <ref.json>")?;
    let tokenizer_path = args.next().ok_or("missing <tokenizer.json>")?;

    let entries: Vec<RefEntry> = serde_json::from_str(&std::fs::read_to_string(&ref_path)?)?;
    let tokenizer = HfTokenizer::from_tokenizer_json(&std::fs::read_to_string(&tokenizer_path)?)?;
    let backend = KaiOnnxBackend::from_file(std::path::Path::new(&model_path))?;

    let mut rows = 0usize;
    let mut argmax_match = 0usize;
    let mut ids_match = 0usize;
    let mut max_delta = 0.0f64;
    let mut worst = String::new();

    for entry in &entries {
        let state = match &entry.request.state {
            Value::String(text) => KaiPayload::Text(text.clone()),
            other => KaiPayload::Structured(other.clone()),
        };
        for case in &entry.cases {
            let spec = entry.request.questions.get(&case.question).ok_or(format!(
                "{}/{}: no question spec",
                entry.id, case.question
            ))?;
            let kind = match case.kind.as_str() {
                "choice" => opencodifier_model::KaiKind::Choice,
                "noul" => opencodifier_model::KaiKind::Noul,
                _ => opencodifier_model::KaiKind::Score,
            };
            let instructions = KaiPayload::Text(spec.instructions.clone());
            let question = match kind {
                opencodifier_model::KaiKind::Noul => {
                    KaiQuestion::noul(&case.question, state.clone(), instructions)
                }
                _ => {
                    let criteria = spec.criteria.as_ref().ok_or(format!(
                        "{}/{}: {} without criteria",
                        entry.id, case.question, case.kind
                    ))?;
                    let options = case
                        .keys
                        .iter()
                        .enumerate()
                        .map(|(index, key)| {
                            let description = if kind == opencodifier_model::KaiKind::Score {
                                criteria.get(index).and_then(Value::as_str)
                            } else {
                                criteria.get(key).and_then(Value::as_str)
                            };
                            match description {
                                Some(text) => KaiOption::described(key, text),
                                None => KaiOption::bare(key),
                            }
                        })
                        .collect();
                    KaiQuestion::new(&case.question, state.clone(), instructions, kind, options)?
                }
            };
            let encoding = KaiEncoding::encode(&question, &tokenizer, MAX_INPUT_TOKENS)?;

            let label = format!("{}/{}", entry.id, case.question);
            if encoding.ids == case.ids
                && encoding.option_pos == case.option_pos
                && encoding.answer_pos == case.answer_pos
            {
                ids_match += 1;
            } else {
                return Err(format!(
                    "{label}: the Rust encode diverges from the fixture ({} ids vs {})",
                    encoding.ids.len(),
                    case.ids.len()
                )
                .into());
            }

            let distribution = score_batch(&backend, std::slice::from_ref(&&encoding))?
                .pop()
                .ok_or("score_batch returned no rows")?;

            let want: BTreeMap<String, f64> = if case.kind == "noul" {
                let p_true = case.answer["noul"].as_f64().ok_or(format!("{label}: bad noul"))?;
                BTreeMap::from([("false".to_owned(), 1.0 - p_true), ("true".to_owned(), p_true)])
            } else {
                let probs = case.answer["probabilities"]
                    .as_object()
                    .ok_or(format!("{label}: bad probabilities"))?;
                probs
                    .iter()
                    .map(|(key, value)| {
                        Ok((key.clone(), value.as_f64().ok_or(format!("{label}: bad prob"))?))
                    })
                    .collect::<Result<BTreeMap<String, f64>, Box<dyn std::error::Error>>>()?
            };

            let mut delta = 0.0f64;
            for (index, key) in case.keys.iter().enumerate() {
                let want_p = want.get(key).ok_or(format!("{label}: missing ref key {key}"))?;
                delta = delta.max((distribution[index] - want_p).abs());
            }
            if delta > max_delta {
                max_delta = delta;
                worst = label.clone();
            }
            let arg_got = case
                .keys
                .iter()
                .zip(&distribution)
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(key, _)| key.clone());
            let arg_want = want
                .iter()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(key, _)| key.clone());
            rows += 1;
            if arg_got == arg_want {
                argmax_match += 1;
            }
            println!("{label}: {} keys, max |Δp| {delta:.4}", case.keys.len());
        }
    }

    println!(
        "rust-side parity: {ids_match}/{rows} encodes byte-identical, {argmax_match}/{rows} argmax, max |Δp| {max_delta:.4} (worst {worst})"
    );
    if argmax_match != rows || ids_match != rows {
        return Err("disagreement against the real weights".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("kai-parity-rs: {error}");
            ExitCode::FAILURE
        }
    }
}
