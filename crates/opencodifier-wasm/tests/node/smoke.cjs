// Node smoke test for the wasm-pack artifact (PLANNING.md §57; D23).
//
// Loads the REAL artifact in `pkg/` — not a mock — and proves the wire
// contract the browser gets: native payload in, native response out,
// hostile input refused with the same typed codes the HTTP surface
// reports. Run via `just check-wasm` (not part of `just ci`; the Node
// toolchain is not a CI dependency).
//
// It also runs the §34 conformance fixtures from `fixtures/conformance/`
// — the same files the crate's native test asserts on — so "wasm is
// another execution target for the same deterministic engine" is checked
// against recorded decisions, not just against the absence of a crash.

"use strict";

const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const bindings = require("../../pkg/opencodifier_wasm.js");

// The §34 fixtures: request fixtures decide, the graph fixture validates.
const CONFORMANCE_DIR = path.join(__dirname, "..", "..", "..", "..", "fixtures", "conformance");
const DECIDE_FIXTURES = ["choice-lexical-accept", "relational-proof", "abstain-no-evidence",
    "invalid-request"];
const GRAPH_FIXTURES = ["invalid-graph"];

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

// One §34 fixture document: {"request": ..., "expected": ...} (or
// {"graph": ..., "expected": ...} for the graph cases).
function load(name) {
    return JSON.parse(fs.readFileSync(path.join(CONFORMANCE_DIR, `${name}.json`), "utf8"));
}

// Bit-ordered distance between two doubles, negatives folded so the
// ordering stays monotonic — the JS twin of the crate's
// `within_one_ulp`. The zero-ML stack is deterministic within a build,
// but not bit-identical across build targets: the same BM25 fold over
// the same query lands one ulp apart on x86-64 (which recorded the
// fixture) and on wasm32 (which just answered), measured, not assumed.
function ulpApart(left, right) {
    const ordered = (value) => {
        const view = new DataView(new ArrayBuffer(8));
        view.setFloat64(0, value);
        const raw = view.getBigUint64(0);
        return (raw >> 63n) === 1n ? ~raw : raw | (1n << 63n);
    };
    const [leftBits, rightBits] = [ordered(left), ordered(right)];
    return (leftBits > rightBits ? leftBits - rightBits : rightBits - leftBits) <= 1n;
}

// The first place two wire documents disagree, or null when they agree
// canonically: same structure, same text, same booleans, and float
// leaves within one ulp (see `ulpApart`). Both sides are `JSON.parse`d
// wire documents — a fixture's whitespace and key order are free; its
// structure, text, and decisions are exact.
function canonicalDifference(actual, expected, at) {
    if (typeof actual === "number" && typeof expected === "number") {
        return actual === expected || ulpApart(actual, expected)
            ? null : `${at}: ${actual} vs ${expected}`;
    }
    if (typeof actual === "string" && typeof expected === "string") {
        return actual === expected ? null : `${at}: "${actual}" vs "${expected}"`;
    }
    if (typeof actual === "boolean" && typeof expected === "boolean") {
        return actual === expected ? null : `${at}: ${actual} vs ${expected}`;
    }
    if (actual === null && expected === null) return null;
    if (Array.isArray(actual) && Array.isArray(expected)) {
        if (actual.length !== expected.length) {
            return `${at}: ${actual.length} entries vs ${expected.length}`;
        }
        for (const [index, item] of actual.entries()) {
            const difference = canonicalDifference(item, expected[index], `${at}[${index}]`);
            if (difference !== null) return difference;
        }
        return null;
    }
    if (actual !== null && expected !== null && typeof actual === "object" &&
        typeof expected === "object") {
        for (const [key, value] of Object.entries(actual)) {
            if (!(key in expected)) return `${at}.${key}: unexpected`;
            const difference = canonicalDifference(value, expected[key], `${at}.${key}`);
            if (difference !== null) return difference;
        }
        for (const key of Object.keys(expected)) {
            if (!(key in actual)) return `${at}.${key}: missing`;
        }
        return null;
    }
    return `${at}: ${JSON.stringify(actual)} vs ${JSON.stringify(expected)}`;
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

    // §34 conformance: the wasm artifact answers the same fixtures the
    // native engine answers. Comparison is canonical, never lexical: both
    // sides are JSON.parse'd (Node parses correctly rounded per
    // ECMA-262; the artifact serializes each double to its shortest
    // round-tripping form), so a fixture's whitespace and key order are
    // free while its structure, text, and decisions are exact — and a
    // float leaf may move one ulp, the cross-target BM25 drift
    // `ulpApart` documents.
    const assertCanonical = (actual, expected, label) => {
        const difference = canonicalDifference(actual, expected, label);
        assert.strictEqual(difference, null, difference ?? undefined);
    };

    for (const name of DECIDE_FIXTURES) {
        const document = load(name);
        let actual;
        let refused = false;
        try {
            actual = JSON.parse(engine.decide(JSON.stringify(document.request)));
        } catch (thrown) {
            // A refused fixture: the thrown error value is the same
            // {"code", "message"} object the fixture records.
            actual = JSON.parse(String(thrown));
            refused = true;
        }
        assertCanonical(actual, document.expected,
            `${name} (${refused ? "refused" : "decided"})`);
    }

    for (const name of GRAPH_FIXTURES) {
        const document = load(name);
        // errorOf already hands back the decoded {"code", "message"} object.
        assertCanonical(errorOf(() => engine.validate_graph(JSON.stringify(document.graph))),
            document.expected, name);
    }

    // The batch is one boundary crossing for many decisions, with the
    // batch wire shape `POST /v1/batch` uses.
    const batch = JSON.parse(engine.decide_batch(JSON.stringify({
        requests: [JSON.parse(REQUEST), JSON.parse(REQUEST)],
    })));
    assert.strictEqual(batch.count, 2, JSON.stringify(batch).slice(0, 200));
    assert.strictEqual(batch.results.length, 2);
    assert.ok(batch.results[0].response, "the first item answers");
    assert.strictEqual(batch.results[1].response.metrics.cache_hit, true,
        "the repeated item is a cache hit");
    const mixed = JSON.parse(engine.decide_batch(JSON.stringify({
        requests: [JSON.parse(REQUEST), {}],
    })));
    assert.strictEqual(mixed.count, 2);
    assert.ok(mixed.results[0].response, "the good item still answers");
    assert.ok(mixed.results[1].error.code.startsWith("schema."),
        "the bad item reports its own schema refusal");
    assert.strictEqual(
        errorOf(() => engine.decide_batch(JSON.stringify({
            requests: Array.from({ length: 17 }, () => JSON.parse(REQUEST)),
        }))).code,
        "schema.limit_exceeded", "the batch ceiling holds");
    assert.strictEqual(errorOf(() => engine.decide_batch("not json")).code,
        "schema.invalid_json");

    console.log("wasm smoke: all assertions passed");
}

main();
