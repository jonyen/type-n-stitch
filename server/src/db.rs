//! SQLite connection pool and migrations. The database holds metadata and
//! the operation log; media files stay on disk under `DATA_DIR`.

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

/// Open (creating if needed) the database at `url` and apply migrations.
pub async fn open(url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Unix seconds, the timestamp format used by every table.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db", dir.path().display());
        let pool = open(&url).await.unwrap();
        let tables: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&pool)
                .await
                .unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        for expected in [
            "users",
            "sessions",
            "projects",
            "project_members",
            "edit_ops",
            "project_sources",
        ] {
            assert!(
                names.contains(&expected),
                "missing table {expected} in {names:?}"
            );
        }
    }

    /// A database from before sources: every project gets its own media as
    /// source 0 when 0005 runs.
    #[tokio::test]
    async fn sources_backfill_gives_every_project_its_first_source() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db", dir.path().display());
        let options = SqliteConnectOptions::from_str(&url)
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .unwrap();
        let migrator = sqlx::migrate!("./migrations");
        migrator.run_to(4, &pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, display_name, color, created_at)
             VALUES ('u', 'u@example.com', 'h', 'U', '#000000', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, media) in [("p1", "m1"), ("p2", "m2")] {
            sqlx::query(
                "INSERT INTO projects (id, media_id, owner_id, title, created_at)
                 VALUES (?, ?, 'u', 'T', 0)",
            )
            .bind(id)
            .bind(media)
            .execute(&pool)
            .await
            .unwrap();
        }
        migrator.run(&pool).await.unwrap();
        let rows: Vec<(String, i64, String, f64, f64)> = sqlx::query_as(
            "SELECT project_id, position, media_id, start_at, duration
             FROM project_sources ORDER BY project_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("p1".into(), 0, "m1".into(), 0.0, 0.0),
                ("p2".into(), 0, "m2".into(), 0.0, 0.0),
            ]
        );
    }

    #[tokio::test]
    async fn open_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/t.db", dir.path().display());
        open(&url).await.unwrap();
        open(&url).await.unwrap();
    }
}
