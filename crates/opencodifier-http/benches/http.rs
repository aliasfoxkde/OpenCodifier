//! HTTP-overhead benchmark (D9 row: "HTTP overhead added by
//! `opencodifier-http`", budget 2 ms p99).
//!
//! A real listener on an ephemeral port serves the production router; the
//! timed body is one full POST /v1/decide round trip over the loopback
//! socket with a real engine behind it — wire JSON → schema decode →
//! engine → encode → HTTP. Measured with a real socket rather than
//! `tower::ServiceExt::oneshot` so the number includes what a client
//! actually experiences. Run with
//! `cargo bench -p opencodifier-http`.

// A benchmark is a test target: fixed inputs, so unwrapping the validated
// constructors is the honest way to keep the measured body readable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use criterion::{Criterion, criterion_group, criterion_main};
use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, RequestMetadata,
    State,
};
use opencodifier_engine::{EngineConfig, EngineHandle};
use opencodifier_schema::WireFormat;
use opencodifier_schema::native::Native;

/// The same representative choice request the engine bench uses.
fn request_body() -> String {
    let question = DecisionQuestion::Choice(
        ChoiceQuestion::new(
            "model",
            "Which model should answer this request?",
            vec![
                Candidate::new("local-small", "small local coding model for quick edits").unwrap(),
                Candidate::new("local-large", "large local model with long context").unwrap(),
                Candidate::new("cloud-large", "cloud research model for long context").unwrap(),
                Candidate::new("cloud-fast", "fast cloud model for short answers").unwrap(),
            ],
        )
        .unwrap(),
    );
    let request = DecisionRequest::new(
        State::from_text(
            "Summarize this research paper across many sources and compare the findings",
        ),
        vec![question],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    Native.encode_request(&request).unwrap().to_string()
}

fn bench_http_overhead(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let listener =
        runtime.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() });
    let addr = listener.local_addr().unwrap();
    let handle = lexical_handle();
    runtime.block_on(async move {
        tokio::spawn(async move {
            axum::serve(listener, opencodifier_http::router(handle)).await.unwrap();
        });
    });
    let client = reqwest::Client::builder().build().unwrap();
    let url = format!("http://{addr}/v1/decide");
    let body = request_body();

    // The engine is not the subject here — the deterministic lexical path
    // is fast enough that the socket + decode + encode stack dominates.
    let mut group = criterion.benchmark_group("http");
    group.bench_function("decide_round_trip_4_candidates", |b| {
        b.iter(|| {
            runtime.block_on(async {
                client
                    .post(&url)
                    .header("content-type", "application/json")
                    .body(body.clone())
                    .send()
                    .await
                    .unwrap()
                    .status()
            });
        });
    });
    group.finish();
}

fn lexical_handle() -> std::sync::Arc<EngineHandle> {
    std::sync::Arc::new(
        EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap(),
    )
}

criterion_group!(benches, bench_http_overhead);
criterion_main!(benches);
