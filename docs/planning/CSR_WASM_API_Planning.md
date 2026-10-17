# OpenCodifier Decision Engine — WASM / Client-Side Runtime

## Objective

Extend OpenCodifier so that the **deterministic Decision Engine itself** can compile to WebAssembly and execute entirely inside a browser without requiring:

- a Cloudflare Worker
- a Cloudflare Pages Function
- an HTTP server
- an external API
- an LLM
- a native executable
- server-side computation

The existing native Rust engine must remain the canonical implementation.

The new WASM target must be a **thin adapter around the existing engine**, not a second implementation of the decision logic.

The resulting architecture should allow:

```text
                    OpenCodifier Engine
                           │
             ┌─────────────┼─────────────┐
             │             │             │
           Native         WASM          HTTP
             │             │             │
            CLI         Browser       Axum
             │             │             │
          local CPU    local CPU     server CPU
```

The WASM implementation should execute the exact same deterministic decision pipeline as the native implementation.

The browser deployment should be capable of being hosted entirely as static files on Cloudflare Pages.

No Cloudflare Worker should be required for the Decision Engine.

---

# 1. Current Architecture

First inspect the repository rather than assuming the architecture.

Repository:

`https://github.com/aliasfoxkde/OpenCodifier`

Current project characteristics include:

- `opencodifier-core`
- `opencodifier-schema`
- `opencodifier-engine`
- `opencodifier-runtime`
- `opencodifier-model`
- `opencodifier-cli`
- `opencodifier-http`
- `opencodifier-mcp`

The current engine exposes `EngineHandle` as the primary engine facade.

The repository already deliberately keeps the core synchronous and trait-based, which is important for WASM portability.

The deterministic engine already supports, among other things:

- DAG execution
- bounded graph nodes
- deterministic rules
- exact decision caching
- metadata narrowing
- BM25 narrowing
- calibration
- relational reasoning
- focus extraction
- zero-ML execution

The existing HTTP interface must remain intact.

Do not redesign the existing architecture merely to accommodate WASM.

---

# 2. Architectural Principle

## One Engine, Multiple Runtimes

The most important requirement is:

> There must be exactly one implementation of the Decision Engine.

Do NOT create:

```text
native-engine.rs
wasm-engine.rs
browser-engine.ts
```

with duplicated decision logic.

Instead:

```text
                 opencodifier-engine
                         │
                  EngineHandle
                         │
          ┌──────────────┼──────────────┐
          │              │              │
       CLI/native       WASM           HTTP
```

The WASM crate should primarily perform:

```text
JavaScript
   ↓
WASM adapter
   ↓
canonical Rust types
   ↓
EngineHandle
   ↓
deterministic engine
   ↓
canonical result
   ↓
WASM serialization
   ↓
JavaScript
```

---

# 3. Desired Browser Architecture

The final static deployment should look conceptually like:

```text
Cloudflare Pages
│
├── index.html
├── assets/
│   ├── app.js
│   ├── app.css
│   └── ...
│
├── wasm/
│   ├── opencodifier_wasm.js
│   ├── opencodifier_wasm_bg.wasm
│   └── ...
│
└── graphs/
    └── optional static graph/configuration files
```

The browser performs:

```text
load application
      ↓
load WASM
      ↓
instantiate EngineHandle
      ↓
load/compile decision graph
      ↓
receive decision request
      ↓
execute locally
      ↓
return decision
```

There should be no request to an OpenCodifier backend merely to perform a deterministic decision.

---

# 4. WASM Crate

Create a dedicated WASM boundary crate.

Preferred structure:

```text
crates/
    opencodifier-wasm/
        Cargo.toml
        src/
            lib.rs
```

If repository conventions suggest a different location, preserve the same conceptual separation.

The WASM crate should depend on:

```text
opencodifier-core
opencodifier-schema
opencodifier-engine
```

and only the minimum additional WASM binding/serialization dependencies required.

Do not introduce browser-specific dependencies into the engine.

The dependency direction must remain:

```text
WASM adapter
    ↓
engine
    ↓
core
```

Never:

```text
engine
    ↓
WASM
```

---

# 5. Keep the Engine WASM-Friendly

Audit `opencodifier-engine` and its transitive dependencies for WASM incompatibilities.

The engine must not require:

- Tokio
- filesystem access
- sockets
- environment variables
- OS threads
- native dynamic libraries
- native-only synchronization primitives
- process spawning
- OS-specific APIs

for its deterministic path.

Do not unnecessarily rewrite working code.

Where platform-specific behavior exists, isolate it behind traits or adapters.

The core decision path must remain synchronous.

---

# 6. Public WASM API

Expose a deliberately small API.

Do NOT expose the entire internal Rust object graph to JavaScript.

Prefer a small stable interface such as:

```text
OpenCodifier
```

or:

```text
DecisionEngine
```

with operations conceptually equivalent to:

```text
create()
load_graph(...)
validate_graph(...)
decide(...)
decide_batch(...)
health()
```

The exact names should follow existing OpenCodifier terminology.

The most important operation is:

```text
decide(input)
```

The API should accept JSON-compatible input at the boundary.

Example:

```javascript
const result = engine.decide(request);
```

The WASM layer converts this into canonical OpenCodifier IR and calls the same `EngineHandle::decide(...)` used elsewhere.

---

# 7. JSON Boundary

Keep JSON at the WASM boundary initially.

Do not attempt to expose every Rust struct through dozens of generated bindings.

Preferred:

```text
JavaScript object
        ↓
JSON/string/serialized boundary
        ↓
Rust canonical IR
        ↓
EngineHandle
        ↓
canonical result
        ↓
JSON-compatible result
        ↓
JavaScript object
```

The internal engine should continue using strongly typed Rust structures.

The JSON boundary is only an interoperability mechanism.

If benchmarking later demonstrates that JSON serialization is materially significant, introduce a second optimized binary/typed API rather than prematurely complicating V1.

---

# 8. Preserve the Canonical Decision Contract

The WASM result must be semantically identical to native execution.

At minimum preserve:

- answer
- answer type
- distribution
- calibrated confidence
- abstention/escalation state
- execution trace where requested
- decision metrics
- error codes
- trace version
- schema version

Do not invent a browser-specific result format.

If the native canonical IR changes, the WASM interface should follow it.

---

# 9. Determinism Requirement

This is a critical requirement.

For the same:

```text
graph
+
request
+
configuration
```

native Rust and WASM execution must produce equivalent results.

Create cross-runtime fixtures.

For example:

```text
fixtures/
    wasm/
        choice.json
        score.json
        boolean.json
        abstain.json
        relational.json
        cache.json
        bm25.json
        invalid.json
```

For every fixture:

```text
native result
==
wasm result
```

where equality is defined according to the canonical representation.

If floating-point serialization creates harmless representation differences, define a precise comparison rule rather than weakening determinism.

---

# 10. Native-vs-WASM Conformance Harness

Create a test harness that executes the same fixture against:

```text
native EngineHandle
```

and:

```text
WASM EngineHandle
```

and compares:

```text
decision
distribution
confidence
abstention
trace
error behavior
```

The goal is not merely:

> "WASM doesn't crash."

The goal is:

> "WASM is another execution target for the same deterministic engine."

---

# 11. Batch API

Add a batch operation where practical:

```text
decide_batch(requests)
```

This is especially important because OpenCodifier is intended to operate at extremely low latency.

A browser application should be able to avoid repeated WASM boundary crossings:

```text
bad:

JS
 ↓
WASM
 ↓
JS
 ↓
WASM
 ↓
JS
 ↓
WASM
```

Prefer:

```text
JS
 ↓
WASM
 ↓
[decision 1
 decision 2
 decision 3
 ...]
 ↓
JS
```

This should be an optimization, not a second decision implementation.

---

# 12. Web Worker Support

Do not require a Web Worker for basic WASM functionality.

However, provide the architecture necessary for the browser application to run the engine inside a Worker.

Preferred browser architecture:

```text
Main Thread
    │
    │ postMessage()
    ▼
Web Worker
    │
    ▼
OpenCodifier WASM
    │
    ▼
EngineHandle
```

This keeps expensive graph evaluation, BM25 processing, large requests, and batch decisions away from the UI thread.

The engine itself must remain unaware that it is running in a Worker.

---

# 13. Engine Initialization

Avoid unnecessary initialization cost.

The WASM runtime should support:

```text
instantiate
    ↓
initialize engine
    ↓
reuse engine
    ↓
many decisions
```

Do not recreate the engine for every request.

Bad:

```javascript
function decide(input) {
    const engine = new Engine();
    return engine.decide(input);
}
```

Preferred:

```javascript
const engine = await OpenCodifier.create();

engine.decide(a);
engine.decide(b);
engine.decide(c);
```

This is especially important for the existing cache.

---

# 14. Graph Loading

The WASM runtime should support loading a decision graph from:

```text
JSON
```

without requiring a server.

Potential flow:

```text
fetch("/graphs/example.json")
        ↓
browser
        ↓
WASM
        ↓
validate_graph()
        ↓
EngineHandle
```

The graph can therefore be:

- bundled with the application
- fetched from static Pages assets
- loaded from local user files
- generated dynamically
- supplied directly by JavaScript

Do not make graph loading dependent upon HTTP server functionality.

---

# 15. Local User Data

The browser runtime should never require user data to leave the browser for deterministic execution.

Example:

```text
User data
   ↓
JavaScript
   ↓
WASM
   ↓
Decision
```

No telemetry.

No implicit network request.

No API key.

No external service.

No analytics dependency.

The browser implementation should preserve OpenCodifier's existing local/offline/private design commitment.

---

# 16. No Cloudflare Worker

Do not create:

```text
functions/
_worker.js
worker.ts
/v1/decide proxy
```

for the browser decision engine.

Cloudflare Pages should simply distribute:

```text
HTML
JS
CSS
WASM
static graphs/data
```

The computation occurs on the client.

A future Worker/server API may exist separately for users who explicitly want remote execution, but it is not part of this implementation.

---

# 17. Optional Remote API Compatibility

Do not remove or alter:

```text
POST /v1/decide
POST /v1/graph/validate
GET /v1/healthz
```

The existing HTTP server remains useful for:

- native deployments
- servers
- containers
- internal infrastructure
- remote clients
- integrations that cannot run WASM

The important architectural change is that:

```text
HTTP API
```

and:

```text
WASM API
```

become two interfaces over the same engine.

---

# 18. Package the WASM Interface for Normal JavaScript

Produce a browser-consumable package.

Prefer a generated package along the lines of:

```text
@opencodifier/wasm
```

or an appropriate repository-specific name.

The generated interface should support:

```javascript
import init, { DecisionEngine } from "@opencodifier/wasm";

await init();

const engine = new DecisionEngine(...);

const result = engine.decide(...);
```

The package must not require Node.js.

It must work in:

- Chrome
- Edge
- Firefox
- Safari

to the extent supported by the selected WASM features.

Avoid requiring WASI unless there is a compelling reason.

The primary target is browser WASM.

---

# 19. Size Optimization

This is an explicit objective.

The deterministic engine is valuable partly because it is small and fast.

Measure the final:

```text
.wasm size
compressed .wasm size
JS glue size
initialization time
first decision latency
steady-state decision latency
batch throughput
```

Build the release WASM with aggressive size optimization.

Investigate:

```text
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

where compatible with the project's requirements.

Then run:

```text
wasm-opt
```

if appropriate.

Do not blindly optimize at the expense of execution speed.

Produce at least two measurements:

```text
size-optimized
speed-optimized
```

and select the appropriate production profile based on actual benchmark results.

---

# 20. Dependency Audit

The WASM build must not accidentally pull in large native/server dependencies.

Inspect:

```text
cargo tree
```

for the WASM target.

Look specifically for:

- tokio
- axum
- reqwest
- native TLS
- ONNX Runtime
- filesystem crates
- server frameworks
- unnecessary serialization frameworks
- native model runtimes

The deterministic WASM target should contain only what is necessary for:

```text
core
schema
engine
deterministic runtime
WASM adapter
```

Do not include model weights.

Do not include LLM infrastructure.

Do not include the HTTP server.

Do not include MCP.

---

# 21. Feature-Gate Non-WASM Components

If the existing dependency graph requires feature flags, establish a clean feature model.

Conceptually:

```text
default
    = native/server capabilities

wasm
    = browser-safe deterministic engine
```

The WASM build must fail clearly if a native-only dependency accidentally enters the target.

Do not use broad conditional compilation throughout the engine unless necessary.

Prefer isolating platform-specific functionality at crate boundaries.

---

# 22. Browser Demo

Add a minimal static browser demonstration.

Do not redesign the existing OpenCodifier website.

Create a small developer/demo surface that demonstrates:

```text
Decision Engine
────────────────────────

Input
[ JSON ]

Graph
[ JSON ]

[ Decide ]

Result
[ decision ]

Latency
0.XX ms

Engine
WASM / local

Network
none
```

The important demonstration should visibly communicate:

> The decision happened locally in the browser.

If practical, include:

```text
Network: 0 requests
Execution: WASM
LLM: none
Server: none
```

These should be factual runtime measurements/status indicators rather than marketing claims.

---

# 23. Benchmarking

Create dedicated WASM benchmarks.

At minimum measure:

### Initialization

```text
WASM download size
WASM instantiate time
Engine initialization time
```

### Decision

Measure:

```text
p50
p95
p99
```

for:

- exact rule
- cache hit
- metadata match
- lexical/BM25
- relational
- abstention
- larger graph
- batch execution

### Boundary overhead

Separate:

```text
JavaScript → WASM
```

from:

```text
actual engine execution
```

Do not report boundary-inclusive latency as engine latency without clearly labeling it.

---

# 24. Performance Goal

The project already has measured deterministic performance in the millisecond range, including a builtin lexical engine benchmark around the existing low-millisecond range. Do not replace those existing measurements with vague "instant" claims.

Establish separate WASM measurements.

The target should be:

```text
steady-state deterministic decision:
sub-10 ms in ordinary browser workloads
```

where practical.

Do not hard-code a claim that all devices will achieve this.

Report actual:

```text
hardware
browser
graph
request
p50/p95/p99
```

for benchmark results.

---

# 25. Cache Behavior

The existing exact-decision cache must work in WASM.

Do not replace it with browser storage.

The first V1 cache should simply be:

```text
in-memory WASM/Rust cache
```

with the existing engine's semantics.

Do not introduce:

- IndexedDB
- localStorage
- OPFS
- Cache API

into the decision engine itself.

Those can be added later at the application layer if needed.

---

# 26. Persistence Boundary

Keep persistence outside the engine.

The engine should remain:

```text
input
 ↓
decision
```

The browser application may optionally provide:

```text
IndexedDB
OPFS
Cache API
```

for:

- graphs
- configuration
- user-created rules
- history

but the engine must not require them.

This keeps the WASM runtime portable to:

- browser
- Web Worker
- WASI
- embedded environments
- native applications

where possible.

---

# 27. Error Handling

WASM must never panic for normal malformed user input.

Convert errors into stable machine-readable errors.

Preserve existing OpenCodifier error codes.

For example:

```json
{
  "error": {
    "code": "engine.invalid_graph",
    "message": "..."
  }
}
```

Do not expose Rust panic strings as the public API.

Malformed:

- graph
- request
- schema
- configuration

must produce controlled errors.

---

# 28. Security

Treat all browser input as untrusted.

Maintain the existing bounded execution model.

Verify that WASM cannot be coerced into:

- unbounded graph traversal
- pathological regex execution
- unbounded memory growth
- recursive stack exhaustion
- enormous request allocation
- cache exhaustion

The existing engine's limits should remain active in WASM.

Do not relax server-side limits simply because the browser is local.

---

# 29. No Hidden Network Access

The WASM engine itself must not perform network access.

The following should remain outside the engine:

```text
fetch()
XMLHttpRequest
WebSocket
WebRTC
```

The engine receives data and produces decisions.

This should make it possible to run the engine offline after the static assets are downloaded.

---

# 30. Optional PWA Layer

Do not make PWA functionality a prerequisite for the WASM engine.

The architecture should support:

```text
Cloudflare Pages
   ↓
normal web app
   ↓
WASM
```

first.

Then optionally:

```text
PWA
   ↓
service worker
   ↓
cache WASM + graphs
   ↓
offline OpenCodifier
```

The PWA should be an application-layer enhancement.

---

# 31. Offline Test

Create an offline test demonstrating:

1. Load application.
2. Load WASM.
3. Load graph.
4. Disable network.
5. Execute decisions.
6. Receive identical results.

This should establish that the actual decision engine is not dependent upon Cloudflare.

---

# 32. Cloudflare Pages Deployment

Add a reproducible WASM build/deployment path.

Conceptually:

```text
cargo build WASM
        ↓
optimize
        ↓
copy generated JS/WASM
        ↓
frontend build
        ↓
static output
        ↓
Cloudflare Pages
```

No Worker deployment.

No server process.

No runtime compute.

The exact Pages configuration should follow the existing site's build system rather than introducing a second frontend framework.

---

# 33. CI

Add CI coverage for:

```text
cargo fmt
cargo clippy
cargo test
cargo deny
native build
WASM build
WASM tests
browser integration test
WASM size report
```

The WASM build must be a required gate once stable.

Do not allow the browser target to silently drift from native behavior.

---

# 34. Cross-Runtime Golden Tests

This is one of the most important pieces.

Create a fixture suite:

```text
fixtures/conformance/
```

Run each fixture against:

```text
native
WASM
```

and compare.

Include:

```text
choice
score
boolean
abstain
exact match
range
set membership
contains
regex
metadata narrowing
BM25
relational
cache hit
multi-node DAG
invalid graph
invalid request
empty candidates
boundary values
maximum graph size
```

The same fixtures should remain usable as the engine evolves.

---

# 35. Do Not Add LLM Functionality

This task is explicitly about the deterministic Decision Engine.

Do not:

- add an LLM
- add model weights
- add embeddings
- add ONNX Runtime
- add inference to the browser
- add remote model calls

The current architecture already distinguishes deterministic execution from the optional model layer.

Preserve that separation.

The WASM V1 target should demonstrate:

```text
OpenCodifier
=
deterministic decision engine
+
WASM
=
local browser decision runtime
```

---

# 36. Do Not Turn the WASM Build Into a Generic Web Framework

Avoid:

- server-side routing
- REST endpoint emulation
- Express-like abstractions
- browser database requirements
- UI dependencies in the engine
- React dependencies in Rust
- frontend logic inside `opencodifier-engine`

The WASM crate is an adapter.

Keep it small.

---

# 37. Recommended Crate Boundary

Target architecture:

```text
crates/
├── opencodifier-core
│
├── opencodifier-schema
│
├── opencodifier-engine
│
├── opencodifier-runtime
│
├── opencodifier-model
│
├── opencodifier-cli
│
├── opencodifier-http
│
├── opencodifier-mcp
│
└── opencodifier-wasm
```

Dependency graph:

```text
                         core
                          │
             ┌────────────┴────────────┐
             │                         │
          schema                    engine
                                       │
                               ┌───────┴───────┐
                               │               │
                              CLI             WASM
                               │               │
                             HTTP              │
                             MCP               │
```

The exact dependency graph may differ according to the existing implementation.

Do not force this structure if repository inspection reveals a better existing boundary.

---

# 38. API Design Example

The JavaScript API should feel approximately like:

```javascript
import init, { DecisionEngine } from "@opencodifier/wasm";

await init();

const engine = new DecisionEngine();

engine.load_graph(graph);

const result = engine.decide(request);

console.log(result);
```

Optional:

```javascript
const results = engine.decide_batch(requests);
```

Optional:

```javascript
const validation = engine.validate_graph(graph);
```

The exact API should follow the native API and canonical IR rather than inventing a new conceptual model.

---

# 39. Worker API Example

The browser-facing application should eventually be able to do:

```javascript
const worker = new Worker("/opencodifier-worker.js", {
    type: "module"
});

worker.postMessage({
    type: "decide",
    request
});

worker.onmessage = ({ data }) => {
    console.log(data.result);
};
```

Again, the Worker is merely transport/isolation.

It must not contain decision logic.

---

# 40. V1 Scope

Implement only:

### Required

- dedicated WASM crate
- deterministic engine compilation to WASM
- browser-compatible WASM bindings
- graph loading
- `decide`
- `decide_batch`
- graph validation
- canonical result/error serialization
- native/WASM conformance tests
- browser demo
- static deployment
- WASM size measurement
- WASM latency benchmarks
- CI build/test
- offline execution validation

### Explicitly out of scope

- LLM inference
- browser model execution
- remote WASM execution
- Worker API
- authentication
- billing
- telemetry
- persistent browser database
- full PWA implementation
- redesign of the OpenCodifier site
- replacing the existing HTTP API

---

# 41. V2 Possibilities

Do not implement these now, but preserve architectural compatibility with:

```text
WASM package
    ↓
npm package
    ↓
browser applications
    ↓
Web Workers
    ↓
PWA/offline applications
    ↓
desktop applications
    ↓
embedded WASM
```

Potential future APIs:

```text
streaming batch decisions
shared graph instances
zero-copy typed input
binary serialization
SharedArrayBuffer
WASM threads
incremental graph compilation
persistent graph cache
```

Only pursue these after profiling demonstrates a real need.

---

# 42. Acceptance Criteria

The implementation is complete only when all of the following are true.

## Functional

- [ ] Existing native tests remain green.
- [ ] Existing HTTP behavior remains unchanged.
- [ ] WASM can instantiate in a normal browser.
- [ ] WASM can load a valid decision graph.
- [ ] WASM can execute `decide`.
- [ ] WASM can execute batch decisions.
- [ ] WASM can validate graphs.
- [ ] WASM returns canonical results.
- [ ] WASM returns canonical errors.
- [ ] WASM handles malformed input without crashing.

## Determinism

- [ ] Native and WASM produce equivalent decisions.
- [ ] Native and WASM pass the conformance fixture suite.
- [ ] Cache behavior remains deterministic.
- [ ] Abstention behavior remains identical.
- [ ] Execution limits remain enforced.

## Performance

- [ ] WASM initialization is measured.
- [ ] First-decision latency is measured.
- [ ] Warm-decision latency is measured.
- [ ] Batch throughput is measured.
- [ ] WASM binary size is measured.
- [ ] Compressed WASM size is measured.
- [ ] No unnecessary server dependencies enter the WASM artifact.

## Deployment

- [ ] Browser demo works from static hosting.
- [ ] Cloudflare Pages deployment requires no Worker.
- [ ] Decision execution produces no network request.
- [ ] Offline execution works after assets are loaded.

## Quality

- [ ] `cargo fmt` passes.
- [ ] `cargo clippy` passes under project policy.
- [ ] native tests pass.
- [ ] WASM tests pass.
- [ ] browser integration tests pass.
- [ ] dependency/security checks pass.
- [ ] documentation explains native vs WASM architecture.

---

# 43. Required Final Deliverable

At completion, provide:

1. Files added/modified.
2. Architecture diagram.
3. WASM build command.
4. Browser usage example.
5. Cloudflare Pages deployment instructions.
6. Native/WASM conformance results.
7. WASM size before/after optimization.
8. Initialization latency.
9. Cold decision latency.
10. Warm decision p50/p95/p99.
11. Batch throughput.
12. Any dependencies added and why.
13. Any platform limitations discovered.
14. Any deviations from this plan and why.

Do not claim performance numbers that were not measured.

---

# 44. Final Architectural Principle

The finished OpenCodifier should be able to exist as:

```text
                   OpenCodifier
                        │
                Deterministic Engine
                        │
        ┌───────────────┼────────────────┐
        │               │                │
      Native           WASM             HTTP
        │               │                │
       CLI           Browser          Server
        │               │                │
     Local CPU       Local CPU       Local CPU
```

The WASM target is not a replacement for the native runtime.

It is not a web server.

It is not an LLM.

It is not a cloud function.

It is simply:

> **The OpenCodifier Decision Engine compiled into a small, portable browser-executable module.**

The ideal result is that `opencodifier.pages.dev` can ship the engine as static assets, the browser downloads it once, and all deterministic decisions thereafter happen locally on the user's machine with no OpenCodifier compute infrastructure involved.

This should reinforce OpenCodifier's existing design commitments of deterministic-first execution, local/offline/private operation, and useful zero-ML decision making rather than changing those commitments.