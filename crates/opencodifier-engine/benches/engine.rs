//! Benchmarks for the engine's hot paths (PLANNING.md §66).
//!
//! Two measurements matter here:
//!
//! * **Cache-key construction** — the gate every request passes through.
//!   It is SHA-256 over canonical bytes, so it has to be cheap enough to
//!   run per request, and this is the number that proves it.
//! * **A full `decide` through the lexical classifier** — the whole
//!   graph: waves, rules, narrowing, BM25, softmax, confidence gate,
//!   trace. This is the "no model" ceiling the engine promises
//!   (PLANNING.md §73).
//!
//! Both are measured against the built-in pipeline, which is what the base
//! binary ships with. Run with `cargo bench -p opencodifier-engine`.

// A benchmark is a test target: fixed inputs, so unwrapping the validated
// constructors is the honest way to keep the measured body readable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, RequestMetadata,
    State,
};
use opencodifier_engine::{
    CacheKeyBuilder, DecisionEngine, EngineConfig, EngineIdentity, LexicalClassifier,
    MockClassifier, SystemClock,
};

/// A representative choice request: four candidates and a state text with
/// enough substance for BM25 to have something to chew on.
fn request() -> DecisionRequest {
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
    DecisionRequest::new(
        State::from_text(
            "Summarize this research paper across many sources and compare the findings",
        ),
        vec![question],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap()
}

/// Cache-key construction: canonicalization plus SHA-256.
fn bench_cache_key(criterion: &mut Criterion) {
    let request = request();
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("cache_key", |b| {
        b.iter(|| {
            let canonical = CacheKeyBuilder::normalized_request(&request);
            CacheKeyBuilder::build(&canonical, &EngineIdentity::builtin()).unwrap()
        });
    });
    group.finish();
}

/// The whole decision graph on a cold cache, on four threads.
///
/// The cache is cleared inside the timed body: that is the point — the
/// measurement is a decision that cannot be answered from memory.
fn bench_decide_full(criterion: &mut Criterion) {
    let request = request();
    let engine = Arc::new(uncached_engine());
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("decide_full_pipeline", |b| {
        b.iter_batched(
            || (),
            |()| {
                engine.cache().clear().unwrap();
                engine.decide(&request).unwrap()
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

/// The second lookup of the same request: canonicalize, hash, hit.
fn bench_decide_cache_hit(criterion: &mut Criterion) {
    let request = request();
    let engine = uncached_engine();
    engine.decide(&request).unwrap();
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("decide_cache_hit", |b| {
        b.iter(|| engine.decide(&request).unwrap());
    });
    group.finish();
}

/// A mock classifier isolates the executor's fixed cost from BM25's.
fn bench_executor_only(criterion: &mut Criterion) {
    let request = request();
    let config = EngineConfig::with_default_pipeline().unwrap().with_parallelism(4);
    let engine = Arc::new(
        DecisionEngine::new(
            config,
            Arc::new(SystemClock),
            Arc::new(MockClassifier::new("bench/mock")),
            None,
        )
        .unwrap(),
    );
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("decide_mock_classifier", |b| {
        b.iter_batched(
            || (),
            |()| {
                engine.cache().clear().unwrap();
                engine.decide(&request).unwrap()
            },
            BatchSize::PerIteration,
        );
    });
    group.finish();
}

/// An engine whose cache starts empty, so a decision cannot be served from
/// memory.
fn uncached_engine() -> DecisionEngine {
    let config = EngineConfig::with_default_pipeline().unwrap().with_parallelism(4);
    DecisionEngine::new(config, Arc::new(SystemClock), Arc::new(LexicalClassifier::new()), None)
        .unwrap()
}

criterion_group!(
    benches,
    bench_cache_key,
    bench_decide_full,
    bench_decide_cache_hit,
    bench_executor_only
);
criterion_main!(benches);
