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

use crate::auth::User;
use crate::config::Config;
use crate::projects::{create_project, test_support::seed_media, Project};
use crate::{app, bus, db, AppState};

/// A state whose data dir and database live in a fresh temp dir.
pub async fn state() -> (Arc<AppState>, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::from_env();
    config.data_dir = dir.path().join("data");
    config.database_url = format!("sqlite://{}/test.db", dir.path().display());
    config.admin_email = None;
    config.admin_password = None;
    // Background transcription must never find a real whisper in tests: a
    // job fails fast instead, which is what the status tests rely on.
    config.whisper_bin = "type-n-stitch-no-whisper-in-tests".into();
    // Nor a real diarizer: speaker labels come from seeded caches only.
    config.diarize_bin = dir.path().join("no-diarizer-in-tests");
    tokio::fs::create_dir_all(&config.data_dir).await.unwrap();
    let db = db::open(&config.database_url).await.unwrap();
    let state = Arc::new(AppState {
        http: reqwest::Client::new(),
        config,
        jobs: Mutex::new(HashMap::new()),
        db,
        folds: Mutex::new(HashMap::new()),
        bus: Arc::new(bus::LocalBus::new()),
        agents: Default::default(),
        transcripts: Mutex::new(HashMap::new()),
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

/// Set an `Authorization: Bearer <token>` header on a request.
pub fn with_bearer(mut req: Request<Body>, token: &str) -> Request<Body> {
    req.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    req
}

/// The signed-in user behind a session cookie.
pub async fn me(state: &Arc<AppState>, cookie: &str) -> User {
    let (_, me, _) = call(
        app(state),
        json_req(Method::GET, "/api/me", Some(cookie), None),
    )
    .await;
    serde_json::from_value(me).unwrap()
}

/// A project called "Clip" owned by `cookie`'s user, over ten seconds of
/// fake media whose transcript cache is pre-seeded with the three words
/// "a b c" at 0, 1 and 2 seconds — so nothing ever shells out to whisper.
pub async fn owned_project(state: &Arc<AppState>, cookie: &str) -> Project {
    let media_id = seed_media(state, 10.0).await;
    seed_words(state, &media_id, &["a", "b", "c"]).await;
    let owner = me(state, cookie).await;
    create_project(&state.db, &owner, &media_id, "Clip")
        .await
        .unwrap()
}

/// Pre-seed `media_id`'s transcript cache with `texts`, one word a second
/// from 0 (each half a second long), so nothing shells out to whisper.
pub async fn seed_words(state: &Arc<AppState>, media_id: &str, texts: &[&str]) {
    let words: Vec<engine::Word> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| engine::Word {
            id: format!("w{i}"),
            text: (*text).to_owned(),
            start: i as f64,
            end: i as f64 + 0.5,
        })
        .collect();
    tokio::fs::write(
        state
            .config
            .data_dir
            .join(media_id)
            .join(crate::routes::WORDS_CACHE),
        serde_json::to_vec(&words).unwrap(),
    )
    .await
    .unwrap();
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

/// Add `email` to `project` with `role`, as the owner behind `cookie`.
pub async fn add_member(
    state: &Arc<AppState>,
    cookie: &str,
    project: &str,
    email: &str,
    role: &str,
) {
    let (status, body, _) = call(
        app(state),
        json_req(
            Method::POST,
            &format!("/api/projects/{project}/members"),
            Some(cookie),
            Some(json!({ "email": email, "role": role })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "add_member: {body}");
}
