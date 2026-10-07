//! C ABI for the zero-ML decision runtime (PLANNING.md §57, §68; D23).
//!
//! [`oc_engine_lexical_new`] hands out the same assembled engine the
//! WASM binding exposes — relational solver over BM25, graph executor,
//! exact-decision cache, traces — as a static library for hosts that
//! speak C: iOS, Android (via JNI), embedded FFI shells. The boundary is
//! the native schema's JSON, so a payload that decides here decides
//! identically through `POST /v1/decide` or in a browser: one wire
//! contract, every transport.
//!
//! # The boundary, structurally
//!
//! * **JSON in, JSON out.** Every entry point decodes through the
//!   validating `opencodifier-schema` adapter via
//!   [`opencodifier_engine::wire`], the same module the WASM binding
//!   calls — there is no second decoder to drift from.
//! * **Errors are values, never unwinds.** A refusal sets the
//!   thread-local last error ([`oc_last_error`]) as
//!   `{"code", "message"}` JSON and returns `NULL` — a stable `schema.*`,
//!   `engine.*`, or `graph.*` code, or this crate's own `ffi.*` codes for
//!   conditions only a C caller can cause (a null pointer) or only the
//!   boundary can see (a caught panic; the workspace forbids `panic!` in
//!   production code, so this is a tripwire, not a code path).
//! * **No threads spawned, no I/O, no network.** The crate's dependency
//!   list admits none of it; the engine is CPU-bound by design (D5) and
//!   the engine handle is `Send + Sync` (asserted by a test), so one
//!   engine may be shared across threads or each thread may own its own.
//! * **All input is hostile.** Malformed bytes are refused with typed
//!   codes, never a best-effort guess (PLANNING.md §73).
//!
//! # Memory contract
//!
//! Returned `char *` strings are heap allocations owned by the caller:
//! release each one with [`oc_string_free`]. An engine handle from
//! [`oc_engine_lexical_new`] is released with [`oc_engine_destroy`].
//! Every function tolerates `NULL` where a pointer is optional and
//! refuses it with `ffi.invalid_argument` where it is not. The string
//! [`oc_last_error`] returns is *not* caller-owned when a failure is
//! recorded: it points into thread-local storage and stays valid only
//! until the next `oc_*` call on the same thread. The empty string
//! returned on the never-failed path is a static.
//!
//! `include/ocffi.h` is the hand-written C header for this surface; the
//! integration test `tests/c_abi.rs` declares the same signatures and is
//! the check that they stay in step.
//!
//! # Example (from C)
//!
//! ```c
//! oc_engine *engine = oc_engine_lexical_new();
//! char *response = oc_decide(engine, request_json);
//! if (response == NULL) {
//!     fprintf(stderr, "%s\n", oc_last_error());  // {"code","message"}
//! } else {
//!     /* parse response */
//!     oc_string_free(response);
//! }
//! oc_engine_destroy(engine);
//! ```

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use opencodifier_engine::{
    EngineConfig, EngineHandle,
    wire::{WireFailure, decide_batch_impl, decide_impl, run_graph_impl, validate_graph_impl},
};

/// The opaque engine handle C callers move around. The zero-sized
/// private field makes it unconstructable and un-dereferenceable from C;
/// every real [`EngineHandle`] lives behind a `Box` this crate owns.
#[repr(C)]
pub struct OcEngine {
    _opaque: [u8; 0],
}

impl std::fmt::Debug for OcEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Opaque by contract: no field of the wrapped engine is
        // reachable from the type C callers hold.
        formatter.write_str("OcEngine")
    }
}

/// The crate version, so a host can pin behavior to the artifact it
/// linked — the same version every other surface reports.
const VERSION_BYTES: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

/// The never-failed value of [`oc_last_error`]: a static empty string,
/// so that path allocates nothing and leaks nothing.
const NO_ERROR: &CStr = c"";

thread_local! {
    /// The most recent failure on this thread, as `{"code","message"}`
    /// JSON. `None` is the never-failed state.
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// A failure at the C boundary itself: conditions no Rust caller can
/// reach. Wire refusals pass through as [`WireFailure`]; the `ffi.*`
/// codes are this crate's own vocabulary, documented in `include/ocffi.h`.
enum Boundary {
    /// A decode, decide, or encode refusal from the shared wire module.
    Wire(WireFailure),
    /// A required pointer argument was null, or a string argument was
    /// not UTF-8. Code `ffi.invalid_argument`.
    Argument(String),
    /// A panic escaped the engine despite the workspace's `panic` deny —
    /// caught here so it never unwinds into C. Code `ffi.panic`.
    Panic(String),
}

impl Boundary {
    /// The stable code the JSON error document carries.
    fn code(&self) -> String {
        match self {
            Self::Wire(failure) => failure.code(),
            Self::Argument(_) => "ffi.invalid_argument".to_owned(),
            Self::Panic(_) => "ffi.panic".to_owned(),
        }
    }

    /// The human-readable half of the error document.
    fn message(&self) -> String {
        match self {
            Self::Wire(failure) => failure.to_string(),
            Self::Argument(detail) | Self::Panic(detail) => detail.clone(),
        }
    }

    /// The error document, serialized exactly once. `Value::to_string`
    /// cannot fail, and the result cannot contain a raw NUL —
    /// `serde_json` escapes control characters — so [`CString::new`]
    /// below only ever sees terminable text.
    fn error_json(&self) -> String {
        serde_json::json!({ "code": self.code(), "message": self.message() }).to_string()
    }
}

/// Records `failure` as this thread's last error. Hostile input is
/// assumed to reach every string here, so a NUL — which the JSON
/// escaping already rules out — is replaced rather than panicking on.
fn set_last_error(failure: &Boundary) {
    let document = CString::new(failure.error_json().replace('\0', "\u{FFFD}"))
        .unwrap_or_else(|_| CString::default());
    LAST_ERROR.with(|slot| slot.borrow_mut().replace(document));
}

/// Runs `operation` inside the panic guard: a payload that escapes the
/// engine becomes an `ffi.panic` refusal, never an unwind into C.
fn guarded<T>(operation: impl FnOnce() -> Result<T, Boundary>) -> Result<T, Boundary> {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        Err(Boundary::Panic(detail))
    })
}

/// The `char *` a successful call hands across the boundary, owned by
/// the caller until [`oc_string_free`].
fn into_cstring(text: &str) -> *mut c_char {
    CString::new(text.replace('\0', "\u{FFFD}")).unwrap_or_else(|_| CString::default()).into_raw()
}

/// The `String` behind a C string argument, refused with
/// `ffi.invalid_argument` when the pointer is null or the bytes are not
/// UTF-8.
fn argument_str(name: &'static str, pointer: *const c_char) -> Result<String, Boundary> {
    if pointer.is_null() {
        return Err(Boundary::Argument(format!("{name} must not be NULL")));
    }
    // The caller's contract is a NUL-terminated string; reading to the
    // terminator is what every entry point here is declared to do.
    let bytes = unsafe { CStr::from_ptr(pointer) };
    bytes
        .to_str()
        .map(str::to_owned)
        .map_err(|error| Boundary::Argument(format!("{name} is not valid UTF-8: {error}")))
}

/// The engine behind a handle argument, refused with
/// `ffi.invalid_argument` when it is null.
fn engine_of(handle: *const OcEngine) -> Result<&'static EngineHandle, Boundary> {
    if handle.is_null() {
        return Err(Boundary::Argument("engine must not be NULL".to_owned()));
    }
    // The handle was minted by [`oc_engine_lexical_new`] as
    // `Box::into_raw` of an `EngineHandle`; only this crate constructs
    // one, so the cast back is the ownership contract, not a guess.
    Ok(unsafe { &*handle.cast::<EngineHandle>() })
}

/// Runs `operation` and turns every failure class into the boundary's
/// error contract: a string the caller owns on success, `NULL` plus the
/// thread-local last error on refusal. Every entry point that produces a
/// string walks this shape, so the contract cannot drift between them.
fn respond(operation: impl FnOnce() -> Result<String, Boundary>) -> *mut c_char {
    match guarded(operation) {
        Ok(text) => into_cstring(&text),
        Err(failure) => {
            set_last_error(&failure);
            ptr::null_mut()
        }
    }
}

/// The crate version as a static string. Never freed; valid for the
/// life of the process.
#[unsafe(no_mangle)]
pub extern "C" fn oc_version() -> *const c_char {
    VERSION_BYTES.as_ptr().cast::<c_char>()
}

/// Assembles the zero-ML engine: the built-in pipeline under the
/// relational solver over BM25 — the same constructor the WASM binding
/// and `POST /v1/decide` run behind.
///
/// Returns the new handle, or `NULL` with the last error set. The handle
/// is released with [`oc_engine_destroy`].
///
/// # Safety
///
/// Safe to call with no arguments; the returned handle's contract begins
/// at the return.
#[unsafe(no_mangle)]
pub extern "C" fn oc_engine_lexical_new() -> *mut OcEngine {
    match guarded(|| {
        EngineConfig::with_default_pipeline()
            .map_err(|error| Boundary::Wire(error.into()))
            .and_then(|config| {
                EngineHandle::lexical(config).map_err(|error| Boundary::Wire(error.into()))
            })
            .map(|handle| Box::into_raw(Box::new(handle)).cast::<OcEngine>())
    }) {
        Ok(handle) => handle,
        Err(failure) => {
            set_last_error(&failure);
            ptr::null_mut()
        }
    }
}

/// Releases an engine from [`oc_engine_lexical_new`]. `NULL` is a no-op;
/// a handle is destroyed exactly once.
///
/// # Safety
///
/// `engine` must be null or a handle this crate returned that has not
/// been destroyed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_engine_destroy(engine: *mut OcEngine) {
    if engine.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(engine.cast::<EngineHandle>()) });
}

/// Frees a string this crate returned (`oc_decide`,
/// [`oc_decide_batch`], [`oc_validate_graph`], [`oc_run_graph`],
/// [`oc_identity`]). `NULL` is a no-op; a string is freed exactly once.
/// Strings from [`oc_last_error`] and [`oc_version`] must **not** be
/// passed here — they are not caller-owned.
///
/// # Safety
///
/// `string` must be null or a pointer this crate returned that has not
/// been freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_string_free(string: *mut c_char) {
    if string.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(string) });
}

/// Decides a native-schema request payload (what `POST /v1/decide`
/// accepts), returning the native-schema response JSON.
///
/// Returns the response document, or `NULL` with the last error set —
/// `schema.*` for a decode refusal, `engine.*` or `graph.*` for an
/// execution refusal.
///
/// # Safety
///
/// `engine` must be null (refused) or a live handle; `request_json` must
/// be null (refused) or a valid NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_decide(
    engine: *const OcEngine,
    request_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let handle = engine_of(engine)?;
        let request = argument_str("request_json", request_json)?;
        decide_impl(handle, &request).map_err(Boundary::Wire)
    })
}

/// Decides a batch envelope (`{"requests": [ ... ]}` — the
/// `POST /v1/batch` body) on the one engine: one boundary crossing for
/// the batch, one cache across it, every item answered in input order.
/// The ceiling is the engine's
/// [`MAX_BATCH`](opencodifier_engine::MAX_BATCH).
///
/// # Safety
///
/// Same contract as [`oc_decide`] for both pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_decide_batch(
    engine: *const OcEngine,
    requests_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let handle = engine_of(engine)?;
        let requests = argument_str("requests_json", requests_json)?;
        decide_batch_impl(handle, &requests).map_err(Boundary::Wire)
    })
}

/// Validates a decision graph without running it: the DAG contract
/// (ids, edges, single output, cycles, node limit) is enforced, and the
/// summary `{"version", "nodes"}` names what validated.
///
/// There is no engine argument on purpose — validation needs no runtime,
/// so a host can check a graph before paying for one.
///
/// # Safety
///
/// `graph_json` must be null (refused) or a valid NUL-terminated UTF-8
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_validate_graph(graph_json: *const c_char) -> *mut c_char {
    respond(|| {
        let graph = argument_str("graph_json", graph_json)?;
        let (version, nodes) = validate_graph_impl(&graph).map_err(Boundary::Wire)?;
        Ok(serde_json::json!({ "version": version, "nodes": nodes }).to_string())
    })
}

/// Decides a request through a client-supplied graph (D19 parity with
/// `POST /v1/graph/run`): the graph's cache identity is derived from its
/// own content, the engine is assembled fresh per call, and no state
/// survives the call.
///
/// # Safety
///
/// Both arguments follow the [`oc_decide`] contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_run_graph(
    graph_json: *const c_char,
    request_json: *const c_char,
) -> *mut c_char {
    respond(|| {
        let graph = argument_str("graph_json", graph_json)?;
        let request = argument_str("request_json", request_json)?;
        run_graph_impl(&graph, &request).map_err(Boundary::Wire)
    })
}

/// The engine identity decisions are cached under: graph, model,
/// calibration, engine, and embedding components, as JSON. Deterministic
/// per build, so a host can pin behavior to the artifact it linked.
///
/// # Safety
///
/// `engine` must be null (refused) or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn oc_identity(engine: *const OcEngine) -> *mut c_char {
    respond(|| {
        let identity = engine_of(engine)?.identity();
        Ok(serde_json::json!({
            "graph_version": identity.graph_version,
            "model_id": identity.model_id,
            "calibration_version": identity.calibration_version,
            "engine_semver": identity.engine_semver,
            "embedding_model": identity.embedding_model,
        })
        .to_string())
    })
}

/// The most recent failure on this thread, as `{"code","message"}` JSON,
/// or a static empty string if this thread has never seen a failure.
///
/// After a failure the pointer is **not** caller-owned: it points into
/// thread-local storage and is valid only until the next `oc_*` call on
/// the same thread. Copy it before calling on. Successes leave the
/// previous error in place — this reports the most recent *failure*, not
/// the most recent call. Never returns null; never free the result.
#[unsafe(no_mangle)]
pub extern "C" fn oc_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| {
        let borrow = slot.borrow();
        borrow.as_ref().map_or(NO_ERROR.as_ptr(), |document| document.as_ptr())
    })
}
