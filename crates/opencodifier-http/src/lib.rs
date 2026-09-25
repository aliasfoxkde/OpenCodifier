//! The axum `/v1` HTTP surface for the `OpenCodifier` decision runtime
//! (PLANNING.md §36, §73; DECISIONS.md D12).
//!
//! A thin shell over a synchronous core: HTTP never decides anything. Each
//! endpoint normalizes its payload through
//! [`opencodifier_schema::WireFormat`], executes through
//! [`opencodifier_engine::EngineHandle`] — the same facade the CLI and MCP
//! surfaces use, so no endpoint can drift into its own pipeline — and
//! projects the typed result back onto the wire. There is no async runtime
//! in the engine, and no decision logic here.
//!
//! # Surface
//!
//! | Route | Method | Purpose |
//! |---|---|---|
//! | `/v1/decide` | `POST` | native decision request → decision response |
//! | `/v1/graph/validate` | `POST` | graph document → validity verdict |
//! | `/v1/healthz` | `GET` | liveness plus the engine identity |
//!
//! Outcomes are orthogonal to status codes. Abstention is a successful
//! decision — the runtime correctly refusing to answer under weak evidence
//! — so it is `200` with an `outcome` field, never an error status
//! (PLANNING.md §5). `400` means the caller sent something the canonical IR
//! cannot represent (`schema.*`, `graph.*` codes); `500` means the engine
//! itself failed (`engine.*`, `ir.*` codes). Every error body is exactly
//! `{"error":{"code":...,"message":...}}`, carrying the stable code of the
//! crate that owns the failure.
//!
//! # Security posture
//!
//! Local-first by default. [`ServerConfig::new`] refuses any non-loopback
//! bind address unless [`ServerConfig::allow_remote`] is set, so
//! `opencodifier serve` cannot start listening on an external interface by
//! accident. Request bodies are hostile input: capped at
//! [`MAX_BODY_BYTES`] before parsing, never echoed
//! back, and never allowed to influence policy, thresholds, graph
//! structure, or paths.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//!
//! use opencodifier_engine::{EngineConfig, EngineHandle};
//! use opencodifier_http::ServerConfig;
//!
//! // The deterministic, zero-ML engine: useful with no model on the machine.
//! let handle = Arc::new(
//!     EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap(),
//! );
//!
//! // Loopback only, unless the operator opts in.
//! let config = ServerConfig::new("127.0.0.1:0".parse().unwrap()).unwrap();
//! assert!(config.bind.ip().is_loopback());
//!
//! // The same router `serve` runs, so tests and production cannot diverge.
//! let app = opencodifier_http::router(Arc::clone(&handle));
//! let _ = (config, app);
//!
//! // Production entry point:
//! // opencodifier_http::serve(config, handle).await?;
//! ```

pub mod config;
pub mod error;
pub mod routes;

use std::io::Write as _;
use std::sync::Arc;

use opencodifier_engine::EngineHandle;

pub use crate::config::ServerConfig;
pub use crate::error::{ErrorBody, ErrorDetail, HttpError};
pub use crate::routes::{MAX_BODY_BYTES, router};

/// Alias used throughout the crate for fallible surface operations.
pub type HttpResult<T> = Result<T, HttpError>;

/// Binds `config.bind`, serves [`router`] over `handle`, and returns when
/// the server shuts down on `Ctrl-C`.
///
/// The bound address is written to `stderr` (the workspace bans the
/// `println!`/`eprintln!` macros, not the stream) so an operator binding an
/// ephemeral port can see where the runtime actually is.
///
/// # Errors
///
/// [`HttpError::RemoteBindForbidden`] when `config.bind` is not loopback
/// and `allow_remote` is `false`; [`HttpError::Bind`] when the listener
/// cannot be bound; [`HttpError::Serve`] when the serve loop fails.
pub async fn serve(config: ServerConfig, handle: Arc<EngineHandle>) -> HttpResult<()> {
    serve_with_shutdown(config, handle, shutdown_signal()).await
}

/// [`serve`] with the shutdown future supplied by the caller: identical
/// serving behaviour, but tests and embedders can end the loop with a
/// signal of their own instead of a process interrupt.
///
/// # Errors
///
/// [`HttpError::RemoteBindForbidden`] when `config.bind` is not loopback
/// and `allow_remote` is `false`; [`HttpError::Bind`] when the listener
/// cannot be bound; [`HttpError::Serve`] when the serve loop fails.
pub async fn serve_with_shutdown(
    config: ServerConfig,
    handle: Arc<EngineHandle>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> HttpResult<()> {
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|source| HttpError::Bind { addr: config.bind, source })?;
    let bound =
        listener.local_addr().map_err(|source| HttpError::Bind { addr: config.bind, source })?;

    {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "opencodifier-http listening on http://{bound}");
        let _ = stderr.flush();
    }

    axum::serve(listener, router(handle))
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(HttpError::from)
}

/// Resolves when the operator interrupts the process (`Ctrl-C`).
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use opencodifier_engine::EngineConfig;

    fn lexical_handle() -> Arc<EngineHandle> {
        Arc::new(EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap())
    }

    #[tokio::test]
    async fn serve_binds_loopback_and_stays_up() {
        let config = ServerConfig::new("127.0.0.1:0".parse().unwrap()).unwrap();
        let task = tokio::spawn(serve(config, lexical_handle()));

        // Yield long enough for a bind failure to surface: a refused or
        // busy address ends the task immediately, a healthy one keeps it
        // parked in the accept loop.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(!task.is_finished(), "serve ended before shutdown");
        task.abort();
    }

    #[test]
    fn serve_refuses_remote_binds_before_touching_the_socket() {
        let error = ServerConfig::new("0.0.0.0:0".parse().unwrap()).unwrap_err();
        assert_eq!(error.code(), "http.remote_bind_forbidden");
    }

    #[tokio::test]
    async fn serve_reports_a_bind_conflict_as_a_typed_error() {
        // Hold a port with a std listener so the server's bind cannot win.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        let config = ServerConfig::new(addr).unwrap();

        let error = serve(config, lexical_handle()).await.unwrap_err();
        drop(probe);

        assert!(matches!(error, HttpError::Bind { .. }), "expected Bind, got {error:?}");
    }

    #[tokio::test]
    async fn serve_with_shutdown_answers_then_ends_cleanly() {
        // Discover a free port, release it, and let the server claim it.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let config = ServerConfig::new(addr).unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve_with_shutdown(config, lexical_handle(), async move {
            let _ = shutdown_rx.await;
        }));

        // Wait until the listener accepts, then exercise the live surface.
        let mut connected = false;
        for _ in 0..200 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                connected = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(connected, "server never began accepting connections");

        let health = reqwest::get(format!("http://{addr}/v1/healthz")).await.unwrap();
        assert_eq!(health.status(), reqwest::StatusCode::OK);

        shutdown_tx.send(()).expect("server task still running");
        let served = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
        assert!(served.expect("shutdown deadline").unwrap().is_ok());
    }
}
