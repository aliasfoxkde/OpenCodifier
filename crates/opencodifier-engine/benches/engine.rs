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

use std::fmt::Write as _;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, FactValue,
    Limits, RequestMetadata, State,
};
use opencodifier_engine::{
    Action, Bm25Index, CacheKeyBuilder, Condition, DecisionEngine, EmbeddingReranker, EngineConfig,
    EngineIdentity, LexicalClassifier, MockClassifier, Reranker, Rule, RuleEngine, RuleSet,
    SystemClock,
};
use opencodifier_runtime::EmbeddingBackend;
use opencodifier_schema::{Native, WireFormat};

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

/// Deterministic hash embedding: no model, no I/O — isolates the Rust-side
/// cosine/scoring cost of the embedding rung (D9: "backend-declared"; the
/// declared backend here is free, so the measurement is engine overhead
/// only).
#[derive(Debug)]
struct HashEmbedding;

impl EmbeddingBackend for HashEmbedding {
    fn model_id(&self) -> &'static str {
        "bench/hash-256"
    }

    fn dims(&self) -> usize {
        256
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, opencodifier_runtime::RuntimeError> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0f32; 256];
                for (position, token) in text.split_whitespace().enumerate() {
                    let slot = token.len().wrapping_mul(31).wrapping_add(position) % 256;
                    vector[slot] += 1.0;
                }
                vector
            })
            .collect())
    }
}

/// D9 row "Normalize (schema → IR)": wire JSON through the Native codec's
/// inbound path — serde parse, unknown-field rejection, IR re-validation.
fn bench_normalize(criterion: &mut Criterion) {
    let payload = serde_json::to_value(request()).unwrap();
    let limits = Limits::default();
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("normalize_native_decode", |b| {
        b.iter(|| Native.decode_request(&payload, &limits).unwrap());
    });
    group.finish();
}

/// D9 row "Rule match + cache hit path" at scale: 200 rules (half fire)
/// over a state carrying 50 facts. The 100 µs budget owns the whole
/// evaluate pass.
fn bench_rule_match_scale(criterion: &mut Criterion) {
    let rules: Vec<Rule> = (0..200)
        .map(|index| Rule {
            name: None,
            when: if index % 2 == 0 {
                Condition::FactExists { fact: format!("key{}", index % 50) }
            } else {
                Condition::FactEquals {
                    fact: format!("absent{index}"),
                    value: FactValue::float(f64::from(index)).unwrap(),
                }
            },
            then: vec![Action::SetFact {
                fact: format!("out{index}"),
                value: FactValue::float(f64::from(index)).unwrap(),
            }],
        })
        .collect();
    let engine = RuleEngine::new(RuleSet { rules }).unwrap();
    let mut state = State::from_text("rule-match scale fixture");
    for index in 0..50 {
        state = state.with_fact(format!("key{index}"), FactValue::float(f64::from(index)).unwrap());
    }
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("rule_match_200", |b| {
        b.iter(|| engine.evaluate(&state, &[]));
    });
    group.finish();
}

/// 256 candidate blurbs with overlapping vocabulary, so BM25 has real
/// term-frequency structure to score against.
fn blurb(index: usize) -> String {
    let topics = ["local", "cloud", "fast", "large", "cheap", "research", "code", "chat"];
    let mut text = String::new();
    for step in 0..12 {
        text.push_str(topics[(index + step * 7) % topics.len()]);
        text.push_str("-model ");
    }
    let _ = write!(text, "candidate-{index} ");
    text.push_str(match index % 4 {
        0 => "answers short factual requests",
        1 => "handles long context research summaries",
        2 => "edits code with quick turnaround",
        _ => "chats with balanced cost and quality",
    });
    text
}

/// D9 row "BM25 narrowing, 256 candidates" — both halves of the cost:
/// index construction (the part rebuilt twice per question today; the B3
/// target) and scoring the query against the built index (the steady state).
fn bench_bm25_256(criterion: &mut Criterion) {
    let documents: Vec<String> = (0..256).map(blurb).collect();
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("bm25_index_build_256", |b| {
        b.iter(|| Bm25Index::new(documents.iter().map(String::as_str)));
    });
    let index = Bm25Index::new(documents.iter().map(String::as_str));
    group.bench_function("bm25_score_all_256", |b| {
        b.iter(|| index.score_all("fast local code model for quick edits candidate-7"));
    });
    group.finish();
}

/// D9 row "Embedding scoring (supplied backend)": the embedding rung's
/// rerank pass over 256 candidates with a zero-cost backend, so the
/// number is the engine's own scoring overhead (query + candidate
/// embeddings are produced inside the timed body — that is the rung's
/// real per-request shape).
fn bench_embedding_scoring(criterion: &mut Criterion) {
    let candidates: Vec<Candidate> = (0..256)
        .map(|index| Candidate::new(format!("candidate-{index}"), blurb(index)).unwrap())
        .collect();
    let reranker = EmbeddingReranker::new(Arc::new(HashEmbedding));
    let mut group = criterion.benchmark_group("engine");
    group.bench_function("embedding_rerank_256", |b| {
        b.iter(|| reranker.rerank("fast local code model for quick edits", &candidates).unwrap());
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_cache_key,
    bench_decide_full,
    bench_decide_cache_hit,
    bench_executor_only,
    bench_normalize,
    bench_rule_match_scale,
    bench_bm25_256,
    bench_embedding_scoring
);
criterion_main!(benches);
