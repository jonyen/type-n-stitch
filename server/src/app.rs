//! The axum router, built once for `main` and once per test.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::{auth, library, ops, projects, routes, AppState};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/auth/setup", get(auth::setup))
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/projects", get(projects::list).post(routes::upload))
        .route("/api/projects/{id}", get(ops::get_project))
        .route("/api/projects/{id}/ops", post(ops::submit))
        .route(
            "/api/projects/{id}/members",
            get(projects::members).post(projects::add_member),
        )
        .route(
            "/api/projects/{id}/members/{user_id}",
            axum::routing::delete(projects::remove_member),
        )
        .route("/api/projects/{id}/transcribe", post(routes::transcribe))
        .route("/api/projects/{id}/suggest", post(routes::suggest))
        .route("/api/projects/{id}/thumbnails", post(routes::thumbnails))
        .route("/api/projects/{id}/speakers", post(routes::speakers))
        .route("/api/projects/{id}/overdub", post(routes::overdub))
        .route("/api/projects/{id}/export", post(routes::export))
        .route(
            "/api/projects/{id}/export/{job}/progress",
            get(routes::export_progress),
        )
        .route("/api/library", get(library::list))
        .route("/api/library/{slug}", post(library::open))
        .nest_service("/data", ServeDir::new(&state.config.data_dir))
        .nest_service(
            "/library",
            ServeDir::new(state.config.samples_dir.join("library")),
        )
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
