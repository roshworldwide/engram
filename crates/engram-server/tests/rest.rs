//! In-process REST integration test (3c): the SRE flow over HTTP — record an
//! observation, an action it caused, and a derived belief; then query the belief
//! and trace provenance.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use engram_query::Engine;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::tempdir;
use tower::ServiceExt;

async fn send(app: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(if body.is_null() {
            Body::empty()
        } else {
            Body::from(serde_json::to_vec(&body).unwrap())
        })
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn rest_sre_flow() {
    let dir = tempdir().unwrap();
    let engine = Arc::new(Engine::open(dir.path()).unwrap());
    let app = engram_server::router(engine);

    // Health.
    let (status, _) = send(&app, "GET", "/health", Value::Null).await;
    assert_eq!(status, StatusCode::OK);

    // Observation.
    let (status, v) = send(
        &app,
        "POST",
        "/memories/episodic",
        json!({"agent":1,"session":1,"valid_time_ms":1000,
               "event_type":"Observation","payload":"metric Y = 95% > 80%"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let obs = v["id"].as_str().unwrap().to_string();

    // Action caused by the observation.
    let (status, v) = send(
        &app,
        "POST",
        "/memories/episodic",
        json!({"agent":1,"session":1,"valid_time_ms":1001,
               "event_type":"Action","payload":"restart service X","cause_ids":[obs]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let action = v["id"].as_str().unwrap().to_string();

    // Belief derived from the observation.
    let (status, _) = send(
        &app,
        "POST",
        "/memories/semantic",
        json!({"agent":1,"subject":"service-x","predicate":"health","object":"unhealthy",
               "valid_from_ms":1000,"confidence":0.9,"provenance_ids":[obs]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Query the current belief.
    let (status, v) = send(
        &app,
        "GET",
        "/memories/semantic?subject=service-x&predicate=health",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["object"], "unhealthy");
    assert!(v["confidence"].as_f64().unwrap() > 0.0);

    // Fetch the action event back.
    let (status, v) = send(
        &app,
        "GET",
        &format!("/memories/episodic/{action}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["payload"], "restart service X");

    // Provenance: why did the agent restart X? -> the observation.
    let (status, v) = send(
        &app,
        "GET",
        &format!("/memories/provenance/{action}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["provenance"][0], obs);

    // Missing event -> 404; bad id -> 400.
    let (status, _) = send(
        &app,
        "GET",
        "/memories/episodic/00000000000000000000000000",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&app, "GET", "/memories/episodic/not-an-id", Value::Null).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
