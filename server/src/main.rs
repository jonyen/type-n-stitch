//! type-n-stitch HTTP server: a thin shell around the `engine` crate that
//! stores uploads, shells out to ffmpeg / whisper-cli / VoiceStudio, and
//! serves `data/` back to the client.

mod config;
mod error;
mod library;
mod media;
mod routes;
mod tts;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

/// Shared server state: read-only config plus the in-flight export jobs.
pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
    pub jobs: Mutex<HashMap<String, routes::ExportJob>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env();
    tokio::fs::create_dir_all(&config.data_dir).await?;
    tracing::info!(
        data_dir = %config.data_dir.display(),
        model = %config.whisper_model.display(),
        tts = %config.tts_base_url,
        "starting"
    );

    let state = Arc::new(AppState {
        http: reqwest::Client::new(),
        config,
        jobs: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/library", get(library::list))
        .route("/api/library/{slug}", post(library::open))
        .route("/api/media", post(routes::upload))
        .route("/api/media/{id}/transcribe", post(routes::transcribe))
        .route("/api/media/{id}/suggest", post(routes::suggest))
        .route("/api/media/{id}/thumbnails", post(routes::thumbnails))
        .route("/api/media/{id}/speakers", post(routes::speakers))
        .route("/api/media/{id}/overdub", post(routes::overdub))
        .route("/api/media/{id}/export", post(routes::export))
        .route(
            "/api/media/{id}/export/{job}/progress",
            get(routes::export_progress),
        )
        .nest_service("/data", ServeDir::new(&state.config.data_dir))
        .nest_service(
            "/library",
            ServeDir::new(state.config.samples_dir.join("library")),
        )
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    tokio::spawn(library::warm(state.clone()));

    let addr = format!("127.0.0.1:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
