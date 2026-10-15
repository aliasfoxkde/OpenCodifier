//! Compute-host parity run for the NLI verbalization verifier
//! (PLAN 24j): the serving scorer in this workspace reproduces the
//! frozen fixture (`crates/opencodifier-model/src/nli/fixture.json`)
//! through the real arm of record —
//! `MoritzLaurer/deberta-v3-base-zeroshot-v2.0`, its own
//! `onnx/model.onnx` — before the arm is trusted anywhere else.
//!
//! Usage (on the compute host):
//!
//! ```text
//! ORT_DYLIB_PATH=<libonnxruntime.so> cargo run --release -- <model_dir>
//! ```
//!
//! `<model_dir>` holds `onnx/model.onnx` + `tokenizer.json` (the
//! repository's published layout). This is a separate binary rather
//! than a `cargo test` case because ORT's load-dynamic teardown does
//! not survive the test harness's exit; the kai and julia contracts
//! ship their parity runs the same way.

use std::path::Path;
use std::process::ExitCode;

use opencodifier_model::NliScorer;
use opencodifier_model::nli;
use opencodifier_model::nli::serving::OnnxNliScorer;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(model_dir) = std::env::args_os().nth(1) else {
        return Err("usage: nli-parity-rs <model_dir>".into());
    };
    let model_dir = Path::new(&model_dir);
    let scorer = OnnxNliScorer::from_dir(model_dir)?;
    println!("scorer identity: {}", scorer.model_prefix());

    // Gate 1 — the frozen fixture, row by row, through the same
    // `verify_fixture` the contract exposes.
    let rows = nli::verify_fixture(&scorer)?;
    println!("fixture parity: {rows} rows reproduced within 1e-4");

    // Gate 2 — one end-to-end decision: a two-candidate choice where
    // the fixture's own first premise ranks A-0000 first, read through
    // the full verbalizer path (premise rendering, hypotheses,
    // renormalization).
    let fixture: serde_json::Value = serde_json::from_str(nli::FIXTURE)?;
    let first = &fixture["items"][0];
    let premise = first["premise"].as_str().ok_or("fixture item missing premise")?;
    let mut ranked: Vec<(String, f64)> = first["rows"]
        .as_array()
        .ok_or("fixture item missing rows")?
        .iter()
        .map(|row| {
            Ok((
                row["hypothesis"].as_str().ok_or("row missing hypothesis")?.to_owned(),
                row["entailment"].as_f64().ok_or("row missing entailment")?,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let masses = scorer.entailment_probabilities(
        premise,
        &ranked.iter().map(|(hypothesis, _)| hypothesis.clone()).collect::<Vec<_>>(),
    )?;
    let mut live: Vec<(String, f64)> =
        ranked.iter().map(|(hypothesis, _)| hypothesis.clone()).zip(masses).collect();
    live.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top_fixture = &ranked[0].0;
    let top_live = &live[0].0;
    if top_fixture != top_live {
        return Err(format!(
            "end-to-end argmax disagrees: fixture {top_fixture:?}, live {top_live:?}"
        )
        .into());
    }
    println!("end-to-end argmax: {top_live:?} (matches the fixture ranking)");

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nli-parity-rs: {error}");
            ExitCode::FAILURE
        }
    }
}
