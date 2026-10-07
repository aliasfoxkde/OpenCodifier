/*
 * OpenCodifier C ABI — the zero-ML decision runtime as a static library.
 *
 * Hand-written companion to `crates/opencodifier-ffi` (no cbindgen in the
 * tree: the surface is nine functions, and the integration test
 * `tests/c_abi.rs` declares the same signatures, so the two cannot drift
 * silently — a signature change that skips the header breaks the test
 * build).
 *
 * Contract summary (crate docs carry the full version):
 *   - JSON in, JSON out: request payloads are the native schema, byte for
 *     byte what `POST /v1/decide` accepts.
 *   - NULL return + `oc_last_error()` on refusal; the error document is
 *     `{"code","message"}` with stable `schema.*` / `engine.*` /
 *     `graph.*` codes (or `ffi.*` for boundary-only conditions).
 *   - Returned `char *` strings are caller-owned: free with
 *     `oc_string_free`. `oc_last_error()` and `oc_version()` results are
 *     NOT caller-owned — never pass them to `oc_string_free`.
 *   - `oc_last_error()` is thread-local and valid only until the next
 *     `oc_*` call on the same thread; copy it first.
 *   - `oc_engine` is Send + Sync: share one handle across threads or own
 *     one per thread; decisions are deterministic either way.
 */
#ifndef OCODIFIER_FFI_H
#define OCODIFIER_FFI_H

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque engine handle: the zero-ML runtime (relational solver over
 * BM25, graph executor, exact-decision cache). */
typedef struct oc_engine oc_engine;

/* Crate version, static storage. Valid for the process lifetime. */
const char *oc_version(void);

/* Assembles the engine (built-in pipeline, lexical classification).
 * NULL + last error on failure. Release with oc_engine_destroy. */
oc_engine *oc_engine_lexical_new(void);

/* Releases an engine. NULL is a no-op. Destroy exactly once. */
void oc_engine_destroy(oc_engine *engine);

/* Decides one native-schema request; returns the response JSON
 * (caller-owned) or NULL + last error. */
char *oc_decide(oc_engine *engine, const char *request_json);

/* Decides a batch envelope {"requests": [...]} in input order on the one
 * engine (one cache across the batch). Returns
 * {"results": [...], "count": N} or NULL + last error. */
char *oc_decide_batch(oc_engine *engine, const char *requests_json);

/* Validates a graph without running it. Returns {"version": V,
 * "nodes": N} or NULL + last error (graph.* codes). */
char *oc_validate_graph(const char *graph_json);

/* Decides through a client-supplied graph; the engine is assembled
 * fresh per call and no state survives. Returns the response JSON or
 * NULL + last error. */
char *oc_run_graph(const char *graph_json, const char *request_json);

/* The cache-identity components (graph, model, calibration, engine,
 * embedding) as JSON. Caller-owned. */
char *oc_identity(oc_engine *engine);

/* The most recent failure on THIS thread as {"code","message"} JSON, or
 * "" if none. Not caller-owned; valid until the next oc_* call on this
 * thread. Never NULL. */
const char *oc_last_error(void);

/* Frees a string this crate returned. NULL is a no-op. Free exactly
 * once; never on oc_last_error/oc_version results. */
void oc_string_free(char *string);

#ifdef __cplusplus
}
#endif

#endif /* OCODIFIER_FFI_H */
