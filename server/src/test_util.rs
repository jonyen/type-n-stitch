//! Shared helpers for handler tests: a temp `AppState` and one-shot requests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::config::Config;
use crate::{app, bus, db, AppState};

/// A state whose data dir and database live in a fresh temp dir.
pub async fn state() -> (Arc<AppState>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env();
    config.data_dir = dir.path().join("data");
    config.database_url = format!("sqlite://{}/test.db", dir.path().display());
    config.admin_email = None;
    config.admin_password = None;
    tokio::fs::create_dir_all(&config.data_dir).await.unwrap();
    let db = db::open(&config.database_url).await.unwrap();
    let state = Arc::new(AppState {
        http: reqwest::Client::new(),
        config,
        jobs: Mutex::new(HashMap::new()),
        db,
        folds: Mutex::new(HashMap::new()),
        bus: Arc::new(bus::LocalBus::new()),
    });
    (state, dir)
}

pub fn app(state: &Arc<AppState>) -> Router {
    app::router(state.clone())
}

pub fn json_req(
    method: Method,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

pub async fn call(app: Router, req: Request<Body>) -> (StatusCode, Value, HeaderMap) {
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value, headers)
}

/// The `name=value` part of the session cookie a response set.
pub fn cookie_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::SET_COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .next()
        .map(str::to_owned)
}

/// Register `email` with password "pw-secret" and return its session cookie.
pub async fn register(state: &Arc<AppState>, email: &str) -> String {
    let (status, _, headers) = call(
        app(state),
        json_req(
            Method::POST,
            "/api/auth/register",
            None,
            Some(json!({ "email": email, "password": "pw-secret", "displayName": email })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    cookie_of(&headers).expect("register sets a cookie")
}

/// Serve the app on an ephemeral port; returns `http://127.0.0.1:PORT`.
pub async fn serve(state: &Arc<AppState>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = app(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}
