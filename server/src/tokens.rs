//! Per-user API tokens for agents and scripts. Only a SHA-256 of the token is
//! stored; the plaintext is shown once at creation.

use std::sync::Arc;

use axum::extract::{Path as UrlPath, State};
use axum::Json;
use base64::Engine as _;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::{CurrentUser, User};
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::AppState;

pub const PREFIX: &str = "tns_";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub id: String,
    pub label: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

fn hash(plaintext: &str) -> String {
    Sha256::digest(plaintext.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub async fn mint(db: &SqlitePool, user_id: &str, label: &str) -> AppResult<(String, TokenInfo)> {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let plaintext = format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    );
    let info = TokenInfo {
        id: Uuid::new_v4().to_string(),
        label: label.trim().to_owned(),
        created_at: now(),
        last_used_at: None,
    };
    sqlx::query(
        "INSERT INTO api_tokens (id, user_id, token_hash, label, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&info.id)
    .bind(user_id)
    .bind(hash(&plaintext))
    .bind(&info.label)
    .bind(info.created_at)
    .execute(db)
    .await?;
    Ok((plaintext, info))
}

/// The owner of a live token, touching `last_used_at`; `None` for unknown or revoked.
pub async fn verify(db: &SqlitePool, plaintext: &str) -> AppResult<Option<User>> {
    if !plaintext.starts_with(PREFIX) {
        return Ok(None);
    }
    let user: Option<User> = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, u.color, u.owner_id FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?",
    )
    .bind(hash(plaintext))
    .fetch_optional(db)
    .await?;
    let user = user.map(User::finish);
    if user.is_some() {
        sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE token_hash = ?")
            .bind(now())
            .bind(hash(plaintext))
            .execute(db)
            .await?;
    }
    Ok(user)
}

pub async fn list(db: &SqlitePool, user_id: &str) -> AppResult<Vec<TokenInfo>> {
    Ok(sqlx::query_as(
        "SELECT id, label, created_at, last_used_at FROM api_tokens WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?)
}

pub async fn revoke(db: &SqlitePool, user_id: &str, id: &str) -> AppResult<bool> {
    let done = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(done.rows_affected() == 1)
}

#[derive(Deserialize)]
pub struct MintRequest {
    label: String,
}

/// `POST /api/tokens`
pub async fn create(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<MintRequest>,
) -> AppResult<Json<Value>> {
    if req.label.trim().is_empty() {
        return Err(AppError::bad_request("give the token a label"));
    }
    let (token, info) = mint(&state.db, &user.id, &req.label).await?;
    Ok(Json(
        json!({ "token": token, "id": info.id, "label": info.label, "createdAt": info.created_at }),
    ))
}

/// `GET /api/tokens`
pub async fn index(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> AppResult<Json<Vec<TokenInfo>>> {
    Ok(Json(list(&state.db, &user.id).await?))
}

/// `DELETE /api/tokens/:id`
pub async fn delete(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
    UrlPath(id): UrlPath<String>,
) -> AppResult<Json<Value>> {
    if revoke(&state.db, &user.id, &id).await? {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(AppError::not_found("no such token"))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use crate::test_util::{app, call, json_req, register, state, with_bearer};

    #[tokio::test]
    async fn mint_list_use_and_revoke() {
        let (state, _d) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/tokens",
                Some(&cookie),
                Some(json!({ "label": "laptop" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let token = body["token"].as_str().unwrap().to_owned();
        assert!(token.starts_with("tns_"));
        let id = body["id"].as_str().unwrap().to_owned();

        // Bearer works where the cookie would.
        let req = json_req(Method::GET, "/api/me", None, None);
        let req = with_bearer(req, &token);
        let (status, me, _) = call(app(&state), req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["email"], "ada@example.com");

        let (_, list, _) = call(
            app(&state),
            json_req(Method::GET, "/api/tokens", Some(&cookie), None),
        )
        .await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert!(
            list[0]["lastUsedAt"].is_number(),
            "use updates lastUsedAt: {list}"
        );
        assert!(list[0].get("token").is_none() && list[0].get("tokenHash").is_none());

        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::DELETE,
                &format!("/api/tokens/{id}"),
                Some(&cookie),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call(
            app(&state),
            with_bearer(json_req(Method::GET, "/api/me", None, None), &token),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn garbage_bearer_is_401_and_cannot_revoke_someone_elses_token() {
        let (state, _d) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let (status, _, _) = call(
            app(&state),
            with_bearer(json_req(Method::GET, "/api/me", None, None), "tns_nope"),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (_, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/tokens",
                Some(&ada),
                Some(json!({ "label": "x" })),
            ),
        )
        .await;
        let id = body["id"].as_str().unwrap();
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::DELETE,
                &format!("/api/tokens/{id}"),
                Some(&bob),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
