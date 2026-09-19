//! type-n-stitch HTTP server: a thin shell around the `engine` crate that
//! stores uploads, shells out to ffmpeg / whisper-cli / VoiceStudio, and
//! serves `data/` back to the client.

mod app;
mod auth;
mod bus;
mod config;
mod db;
mod error;
mod library;
mod mcp;
mod mcp_tools;
mod media;
mod ops;
mod projects;
mod routes;
#[cfg(test)]
mod test_util;
mod tokens;
mod tts;
mod ws;

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
    /// In-memory fold cache for collaborative editing sessions.
    pub folds: Mutex<HashMap<String, (i64, engine::ProjectDoc)>>,
    /// Fan-out for live edits and presence, one hub per project.
    pub bus: Arc<dyn bus::Bus>,
    /// The live MCP agents, one per token owner's bot.
    pub agents: mcp::Agents,
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
        bus: Arc::new(bus::LocalBus::new()),
        agents: mcp::Agents::default(),
    });

    let app = app::router(state.clone());

    if let (Some(email), Some(password)) = (&state.config.admin_email, &state.config.admin_password)
    {
        let admin = match auth::create_user(&state.db, email, password, "admin").await {
            Ok(user) => Some(user.id),
            Err(e) if e.status() == axum::http::StatusCode::BAD_REQUEST => {
                let row: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
                    .bind(email.trim().to_ascii_lowercase())
                    .fetch_optional(&state.db)
                    .await?;
                row.map(|r| r.0)
            }
            Err(e) => anyhow::bail!("creating admin user: {e:?}"),
        };
        if let Some(id) = admin {
            projects::adopt_orphans(&state, &id)
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
    }

    tokio::spawn(library::warm(state.clone()));
    // Nothing else notices an agent that simply stops calling, so a reaper
    // retires the idle ones and, with them, their place in the peer list.
    tokio::spawn(mcp::reap_idle_agents(state.clone()));

    let addr = format!("127.0.0.1:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
