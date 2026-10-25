//! End-to-end feedback-loop tests (#193): `POST /v1/feedback` appends
//! `{p_true, label, source, digest}` rows to the evidence JSONL in the
//! exact format `ladder fit-boolean` reads — the loop closes with no new
//! mathematics.
//!
//! Like the other e2e suites: real engine, real listener, ephemeral
//! port; assertions on the wire contract and on the file the sink wrote.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use opencodifier_engine::{EngineConfig, EngineHandle};
use reqwest::StatusCode;
use serde_json::json;

/// Spawns the router with a feedback sink pointed at a temp file.
async fn spawn_with_evidence(path: std::path::PathBuf) -> String {
    let handle =
        Arc::new(EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = opencodifier_http::router_with_feedback(handle, Some(path));
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn feedback_appends_a_fit_boolean_readable_row() {
    let dir = tempfile::tempdir().unwrap();
    let evidence = dir.path().join("evidence.jsonl");
    let base = spawn_with_evidence(evidence.clone()).await;

    let response = reqwest::Client::new()
        .post(format!("{base}/v1/feedback"))
        .json(&json!({"p_true": 0.82, "label": true, "source": "human", "digest": "abc"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{}", response.text().await.unwrap());

    let rows = std::fs::read_to_string(&evidence).unwrap();
    let row: serde_json::Value = serde_json::from_str(rows.lines().next().unwrap()).unwrap();
    assert_eq!(row["p_true"], 0.82);
    assert_eq!(row["label"], true);
    assert_eq!(row["source"], "human");

    // The exact half the fitter consumes: p_true + label survive a
    // round-trip through the fitter's own row type.
    let mut fitted = 0usize;
    for line in rows.lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let _p = row["p_true"].as_f64().unwrap();
        let _l = row["label"].as_bool().unwrap();
        fitted += 1;
    }
    assert_eq!(fitted, 1);
}

#[tokio::test]
async fn feedback_appends_grow_the_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let evidence = dir.path().join("evidence.jsonl");
    let base = spawn_with_evidence(evidence.clone()).await;
    let client = reqwest::Client::new();
    for p in [0.1, 0.5, 0.9] {
        let response = client
            .post(format!("{base}/v1/feedback"))
            .json(&json!({"p_true": p, "label": false, "source": "outcome"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }
    let rows = std::fs::read_to_string(&evidence).unwrap();
    assert_eq!(rows.lines().count(), 3);
}

#[tokio::test]
async fn out_of_range_p_true_is_a_400() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_with_evidence(dir.path().join("e.jsonl")).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/feedback"))
        .json(&json!({"p_true": 1.5, "label": true, "source": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_source_is_a_400() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_with_evidence(dir.path().join("e.jsonl")).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/feedback"))
        .json(&json!({"p_true": 0.5, "label": true, "source": "vibes"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn without_a_sink_the_route_is_a_loud_404() {
    let handle = std::sync::Arc::new(
        EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = opencodifier_http::router(handle);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}");
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/feedback"))
        .json(&json!({"p_true": 0.5, "label": true, "source": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "http.feedback_disabled");
}

use std::sync::Arc;
