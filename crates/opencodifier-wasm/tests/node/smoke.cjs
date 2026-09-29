// Node smoke test for the wasm-pack artifact (PLANNING.md §57; D23).
//
// Loads the REAL artifact in `pkg/` — not a mock — and proves the wire
// contract the browser gets: native payload in, native response out,
// hostile input refused with the same typed codes the HTTP surface
// reports. Run via `just check-wasm` (not part of `just ci`; the Node
// toolchain is not a CI dependency).

"use strict";

const assert = require("node:assert");
const bindings = require("../../pkg/opencodifier_wasm.js");

// The native-schema request, byte-shape what POST /v1/decide accepts
// (same fixture shape scripts/e2e_validate.py posts).
const REQUEST = JSON.stringify({
    state: { text: "The service is down and customers are affected.", facts: {} },
    questions: [{
        type: "choice",
        id: "decision",
        text: "Which component is responsible for the outage?",
        candidates: [
            { id: "database", description: "The database tier" },
            { id: "network", description: "The network path" },
        ],
    }],
    policy: { min_confidence: 0.8, verify_below: 0.65, abstain_below: 0.5, risk: "low" },
    metadata: {
        request_id: "wasm-smoke",
        limits: {
            max_input_bytes: 1048576,
            max_questions: 32,
            max_candidates: 256,
            max_graph_nodes: 128,
            max_execution_time: { secs: 10, nanos: 0 },
            max_retrieval_results: 64,
        },
    },
});

const VALID_GRAPH = JSON.stringify({
    version: 1,
    nodes: [
        { id: "normalize", kind: "normalize" },
        { id: "output", kind: "output", depends_on: ["normalize"] },
    ],
});

const CYCLIC_GRAPH = JSON.stringify({
    version: 1,
    nodes: [
        { id: "a", kind: "rule", depends_on: ["b"] },
        { id: "b", kind: "rule", depends_on: ["a"] },
    ],
});

const RETRIEVAL_GRAPH = JSON.stringify({
    version: 1,
    nodes: [
        { id: "normalize", kind: "normalize" },
        { id: "embed", kind: "embedding", depends_on: ["normalize"] },
        { id: "output", kind: "output", depends_on: ["embed"] },
    ],
});

// Decoded error payload of a thrown JsValue string: {"code", "message"}.
function errorOf(thunk) {
    try {
        thunk();
    } catch (thrown) {
        return JSON.parse(String(thrown));
    }
    throw new Error("expected the call to throw");
}

function main() {
    const engine = new bindings.WasmEngine();

    // Version + identity: what the page loaded is what decided (§57).
    assert.ok(bindings.opencodifier_version().length > 0, "version is non-empty");
    const identity = JSON.parse(engine.identity());
    for (const component of ["graph_version", "model_id", "calibration_version",
        "engine_semver", "embedding_model"]) {
        assert.ok(component in identity, `identity carries ${component}`);
    }
    assert.strictEqual(identity.embedding_model, "none", "zero-ML posture");

    // The round trip: one typed answer, in the candidate set.
    const response = JSON.parse(engine.decide(REQUEST));
    assert.strictEqual(response.answers.length, 1, JSON.stringify(response).slice(0, 200));
    const answer = response.answers[0];
    assert.ok(
        ["database", "network"].includes(answer.choice ?? answer.value ?? answer.candidate),
        `answer names a candidate: ${JSON.stringify(answer)}`,
    );

    // Determinism: a second identical request decides identically.
    assert.strictEqual(engine.decide(REQUEST), engine.decide(REQUEST));

    // Hostile input: typed refusal, never a panic across the boundary.
    assert.strictEqual(
        errorOf(() => engine.decide("not json")).code, "schema.invalid_json");
    assert.ok(
        errorOf(() => engine.decide("{}")).code.startsWith("schema."),
        "malformed shape refuses with a schema code");

    // Graph validation: a good document summarizes; a cycle refuses.
    assert.deepStrictEqual(JSON.parse(engine.validate_graph(VALID_GRAPH)),
        { version: 1, nodes: 2 });
    assert.strictEqual(
        errorOf(() => engine.validate_graph(CYCLIC_GRAPH)).code, "graph.cycle");

    // Client-supplied graph decides (D19 parity) and a graph needing a
    // backend is refused, exactly as natively (D21). The graph carries a
    // decision stage — a validate-only shape (normalize → output) answers
    // no questions and produces zero answers by contract.
    const decidingGraph = JSON.stringify({
        version: 1,
        nodes: [
            { id: "normalize", kind: "normalize" },
            { id: "choice", kind: "choice", depends_on: ["normalize"] },
            { id: "threshold", kind: "threshold", threshold: 0.5, depends_on: ["choice"] },
            { id: "output", kind: "output", depends_on: ["threshold"] },
        ],
    });
    const throughGraph = JSON.parse(engine.run_graph(decidingGraph, REQUEST));
    assert.strictEqual(throughGraph.answers.length, 1);
    assert.strictEqual(
        errorOf(() => engine.run_graph(RETRIEVAL_GRAPH, REQUEST)).code,
        "engine.missing_backend");

    console.log("wasm smoke: all assertions passed");
}

main();
