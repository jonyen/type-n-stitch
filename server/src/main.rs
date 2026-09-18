//! type-n-stitch HTTP server: a thin shell around the `engine` crate that
//! stores uploads, shells out to ffmpeg / whisper-cli / VoiceStudio, and
//! serves `data/` back to the client.

mod app;
mod auth;
mod config;
mod db;
mod error;
mod library;
mod media;
mod projects;
mod routes;
#[cfg(test)]
mod test_util;
mod tts;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing_subscriber::EnvFilter;

use crate::config::Config;

/// Shared server state: read-only config plus the in-flight export jobs.
pub struct AppState {
    pub config: Config,
    pub http: reqwest::Client,
    pub jobs: Mutex<HashMap<String, routes::ExportJob>>,
    pub db: sqlx::SqlitePool,
    /// In-memory fold cache for collaborative editing sessions; wired up in Task 5.
    #[allow(dead_code)]
    pub folds: Mutex<HashMap<String, (i64, engine::ProjectDoc)>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env();
    tokio::fs::create_dir_all(&config.data_dir).await?;
    let db = db::open(&config.database_url).await?;
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
        db,
        folds: Mutex::new(HashMap::new()),
    });

    let app = app::router(state.clone());

    tokio::spawn(library::warm(state.clone()));

    let addr = format!("127.0.0.1:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
