//! Compute-host parity run for the Julia-1 ONNX rung (task #92).
//!
//! Runs the shipped Rust contract ([`opencodifier_model::JuliaEncoding`])
//! and transport ([`opencodifier_runtime::JuliaOnnxBackend`]) against the
//! real export's weights and compares the logits with the frozen fixture
//! (recorded from the upstream PyTorch model). The python parity
//! (`julia1_parity.py`, 100/100 rows) proves the export matches
//! PyTorch; this proves the *Rust* path reproduces both.
//!
//! Usage (on the compute host):
//!
//! ```text
//! ORT_DYLIB_PATH=<libonnxruntime.so> cargo run --release -- \
//!     <model.onnx> <julia-reference-trimmed.json>
//! ```
//!
//! Paths default to the compute-host eval layout when omitted.

use std::collections::BTreeMap;
use std::process::ExitCode;

use opencodifier_model::{
    BpeTokenizer, JULIA_FIXTURE_TOKENIZER, JuliaEncoding, JuliaKind, JuliaOption, JuliaQuestion,
    MAX_LENGTH,
};
use opencodifier_runtime::{InferenceBackend, JuliaOnnxBackend, LOGITS_OUTPUT};

const DEFAULT_MODEL: &str = "models/onnx/julia-1/model.onnx";
const DEFAULT_FIXTURE: &str =
    "../../crates/opencodifier-model/src/julia/fixtures/julia-reference-trimmed.json";

#[derive(serde::Deserialize)]
struct FixtureCase {
    id: String,
    kind: String,
    head_text: String,
    option_texts: Vec<String>,
    state_text: String,
    ids: Vec<i64>,
    markers: Vec<usize>,
    logits: Vec<f64>,
}

#[derive(serde::Deserialize)]
struct Fixture {
    cases: Vec<FixtureCase>,
}

fn kind_of(name: &str) -> JuliaKind {
    match name {
        "score" => JuliaKind::Score,
        "noul" => JuliaKind::Noul,
        _ => JuliaKind::Choice,
    }
}

fn question_of(case: &FixtureCase) -> Result<JuliaQuestion, Box<dyn std::error::Error>> {
    let prefix = format!("{} question: ", kind_of(&case.kind).as_str());
    let question = case
        .head_text
        .strip_prefix(&prefix)
        .ok_or(format!("case {}: head_text lacks the `{prefix}` prefix", case.id))?;
    Ok(JuliaQuestion::new(
        case.id.clone(),
        case.state_text.clone(),
        question,
        kind_of(&case.kind),
        case.option_texts
            .iter()
            .map(|text| JuliaOption { key: text.clone(), text: text.clone() })
            .collect(),
    )?)
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().unwrap_or_else(|| DEFAULT_MODEL.to_owned());
    let fixture_path = args.next().unwrap_or_else(|| DEFAULT_FIXTURE.to_owned());

    let fixture: Fixture =
        serde_json::from_str(&std::fs::read_to_string(&fixture_path)?)?;
    let tokenizer = BpeTokenizer::from_tokenizer_json(JULIA_FIXTURE_TOKENIZER)?;
    let backend = JuliaOnnxBackend::from_file(std::path::Path::new(&model_path))?;

    let mut matched = 0usize;
    let mut max_error = 0.0f64;
    for case in &fixture.cases {
        let question = question_of(case)?;
        let encoding = JuliaEncoding::encode(&question, &tokenizer, MAX_LENGTH)?;
        if encoding.ids != case.ids || encoding.marker_pos != case.markers {
            return Err(format!(
                "case {}: the Rust encode diverges from the fixture ({} ids vs {})",
                case.id,
                encoding.ids.len(),
                case.ids.len()
            )
            .into());
        }
        encoding.validate()?;

        let outputs: BTreeMap<String, opencodifier_runtime::DenseTensor> =
            backend.infer(&encoding.to_tensors()?)?;
        let logits = &outputs[LOGITS_OUTPUT];
        if logits.data().len() != case.logits.len() {
            return Err(format!(
                "case {}: the graph produced {} logits, the fixture records {}",
                case.id,
                logits.data().len(),
                case.logits.len()
            )
            .into());
        }
        let error = logits
            .data()
            .iter()
            .zip(&case.logits)
            .map(|(got, want)| (f64::from(*got) - want).abs())
            .fold(0.0f64, f64::max);
        let arg_got = logits
            .data()
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .unwrap_or_default();
        let arg_want = case
            .logits
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .unwrap_or_default();
        let agree = arg_got == arg_want;
        if agree {
            matched += 1;
        }
        max_error = max_error.max(error);
        println!(
            "{}: {} options, {} tokens, argmax {} (fixture {}), max |Δlogit| {error:.3e}",
            case.id,
            case.logits.len(),
            case.ids.len(),
            arg_got,
            arg_want
        );
    }

    println!("rust-side parity: {matched}/{} argmax, max |Δlogit| {max_error:.3e}", fixture.cases.len());
    if matched != fixture.cases.len() {
        return Err("argmax disagreement against the real weights".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("julia-parity-rs: {error}");
            ExitCode::FAILURE
        }
    }
}
