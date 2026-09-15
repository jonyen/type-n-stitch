//! type-n-stitch HTTP server: a thin shell around the `engine` crate that
//! stores uploads, shells out to ffmpeg / whisper-cli / VoiceStudio, and
//! serves `data/` back to the client.

mod config;
mod error;
mod media;
mod routes;
mod tts;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

/// Shared, read-only server state.
pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
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
    });

    let app = Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/media", post(routes::upload))
        .route("/api/media/{id}/transcribe", post(routes::transcribe))
        .route("/api/media/{id}/overdub", post(routes::overdub))
        .route("/api/media/{id}/export", post(routes::export))
        .nest_service("/data", ServeDir::new(&state.config.data_dir))
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
