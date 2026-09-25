//! Typed errors for the HTTP surface (PLANNING.md §36, §73).
//!
//! Every failure the surface reports carries a stable machine-readable
//! code. Codes are relayed verbatim from the crate that owns the failure —
//! `schema.*` from wire normalization, `graph.*` / `engine.*` from the
//! engine — and only three codes are this crate's own
//! (`http.remote_bind_forbidden`, `http.bind_failed`, `http.serve_failed`),
//! because only the bind policy is HTTP-specific. Callers branch on codes,
//! never on prose.
//!
//! The wire shape of an error is fixed and minimal:
//!
//! ```json
//! {"error":{"code":"schema.invalid_json","message":"..."}}
//! ```
//!
//! Messages are the typed `Display` of the underlying error. No stack, no
//! internal path, no echo of the offending payload: request bodies are
//! hostile input and are never reflected back.

use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use opencodifier_engine::EngineError;
use opencodifier_schema::SchemaError;
use serde::Serialize;

/// The single `error` envelope every failure response uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorBody {
    /// The error envelope, keyed so the shape is always
    /// `{"error":{"code":..,"message":..}}`.
    pub error: ErrorDetail,
}

impl ErrorBody {
    /// Builds an envelope from a stable code and a typed message.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { error: ErrorDetail { code: code.into(), message: message.into() } }
    }
}

/// The code/message pair inside [`ErrorBody`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    /// Stable machine-readable code (`schema.*`, `graph.*`, `engine.*`,
    /// `http.*`).
    pub code: String,
    /// Human-readable, typed message. Never carries internal state.
    pub message: String,
}

/// Everything that can fail on the HTTP surface.
///
/// Request-scoped variants ([`HttpError::Schema`], [`HttpError::Engine`],
/// [`HttpError::Graph`]) become HTTP responses. Server-scoped variants
/// ([`HttpError::RemoteBindForbidden`], [`HttpError::Bind`],
/// [`HttpError::Serve`]) are returned from
/// [`serve`](crate::serve) and never produced by a handler; they still
/// render as the same envelope if converted, so no path can leak an
/// unformatted error.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// The payload is not JSON, or the native adapter refused it. A client
    /// input error, never a server fault.
    #[error("{0}")]
    Schema(
        /// The refusing adapter error, carrying its `schema.*` code.
        #[from]
        SchemaError,
    ),

    /// The engine failed or refused while deciding a valid request.
    #[error("{0}")]
    Engine(
        /// The engine error, carrying its `engine.*` / `graph.*` code.
        #[from]
        EngineError,
    ),

    /// A graph document failed validation. This is an input error — a bad
    /// graph is a caller bug, not a server fault — so it maps to `400`
    /// even though the code is the engine's.
    #[error("{0}")]
    Graph(
        /// The validation error, carrying its `graph.*` code.
        EngineError,
    ),

    /// A non-loopback bind address was requested without
    /// [`ServerConfig::allow_remote`](crate::ServerConfig::allow_remote).
    ///
    /// This is the SECURITY.md gate: the decision runtime is local-first
    /// and must not start listening on an external interface by accident.
    #[error(
        "refusing to bind {0}: non-loopback bind addresses require `allow_remote` (SECURITY.md)"
    )]
    RemoteBindForbidden(
        /// The rejected bind address.
        SocketAddr,
    ),

    /// The listener could not be bound (port in use, permission denied).
    #[error("failed to bind {addr}: {source}")]
    Bind {
        /// The address that could not be bound.
        addr: SocketAddr,
        /// The OS error.
        source: std::io::Error,
    },

    /// The serve loop itself failed after binding.
    #[error("serve failed: {0}")]
    Serve(
        /// The I/O error that ended the serve loop.
        std::io::Error,
    ),
}

impl HttpError {
    /// Stable machine-readable code for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema(inner) => inner.code(),
            Self::Engine(inner) | Self::Graph(inner) => inner.code(),
            Self::RemoteBindForbidden(_) => "http.remote_bind_forbidden",
            Self::Bind { .. } => "http.bind_failed",
            Self::Serve(_) => "http.serve_failed",
        }
    }

    /// The HTTP status this failure maps onto.
    ///
    /// Decode, normalization, and validation failures are `400` — bad
    /// input, not a broken server. Only a genuine engine fault or a
    /// server-side failure is `500`.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Schema(_) | Self::Graph(_) => StatusCode::BAD_REQUEST,
            Self::Engine(_) | Self::RemoteBindForbidden(_) | Self::Bind { .. } | Self::Serve(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// The exact response body for this failure.
    #[must_use]
    pub fn body(&self) -> ErrorBody {
        ErrorBody::new(self.code(), self.to_string())
    }
}

impl From<std::io::Error> for HttpError {
    fn from(source: std::io::Error) -> Self {
        Self::Serve(source)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status(), axum::Json(self.body())).into_response()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use opencodifier_core::CoreError;

    #[test]
    fn schema_errors_map_to_400_with_their_own_code() {
        let error = HttpError::from(SchemaError::Json("eof while parsing".into()));
        assert_eq!(error.code(), "schema.invalid_json");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn graph_validation_is_a_400_even_though_the_engine_owns_the_code() {
        let cycle = EngineError::Cycle { node: "a".into() };
        let error = HttpError::Graph(cycle);
        assert_eq!(error.code(), "graph.cycle");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn engine_faults_map_to_500() {
        let error = HttpError::Engine(EngineError::from(CoreError::EmptyQuestions));
        assert_eq!(error.code(), "ir.invalid");
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn remote_bind_is_forbidden_with_a_stable_code() {
        let addr: SocketAddr = "0.0.0.0:8177".parse().unwrap();
        let error = HttpError::RemoteBindForbidden(addr);
        assert_eq!(error.code(), "http.remote_bind_forbidden");
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(error.to_string().contains("allow_remote"));
    }

    #[test]
    fn error_body_is_exactly_the_envelope() {
        let body = HttpError::from(SchemaError::Json("trailing comma".into())).body();
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "schema.invalid_json",
                    "message": "payload is not valid JSON: trailing comma",
                }
            })
        );
    }

    #[test]
    fn io_errors_become_serve_faults() {
        let error = HttpError::from(std::io::Error::other("closed"));
        assert_eq!(error.code(), "http.serve_failed");
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
    #[test]
    fn bind_failures_carry_their_own_code() {
        let error = HttpError::Bind {
            addr: "127.0.0.1:1".parse().unwrap(),
            source: std::io::Error::other("address already in use"),
        };
        assert_eq!(error.code(), "http.bind_failed");
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(error.to_string().contains("failed to bind"), "{error}");
    }

    #[test]
    fn serve_loop_failures_carry_their_own_code() {
        let error = HttpError::Serve(std::io::Error::other("closed"));
        assert_eq!(error.code(), "http.serve_failed");
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
