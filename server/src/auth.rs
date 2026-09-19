//! Accounts and sessions: argon2id passwords, an opaque session token in an
//! HttpOnly cookie, and the `CurrentUser` extractor every private route uses.

use std::sync::{Arc, LazyLock};

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::response::{AppendHeaders, IntoResponse};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::AppState;

pub const COOKIE_NAME: &str = "tns_session";
const SESSION_SECS: i64 = 30 * 24 * 60 * 60;

/// Avatar colours, assigned round-robin at registration.
const COLORS: &[&str] = &[
    "#e0575b", "#e08f3c", "#c9a227", "#4caf6e", "#3c8fd1", "#7c5cd6", "#d15ca7", "#3fb3b3",
];

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    /// Whether this account acts on behalf of another user, e.g. an agent
    /// authenticating with an API token minted by a human. Derived from
    /// `owner_id` by [`User::finish`], never set directly; kept as an
    /// explicit serialised field so clients don't need to infer it.
    #[serde(default)]
    #[sqlx(default)]
    pub bot: bool,
}

impl User {
    /// Whether this account acts on behalf of another user.
    pub fn is_bot(&self) -> bool {
        self.owner_id.is_some()
    }

    /// Fills `bot` from `owner_id`. Every place a `User` is built or fetched
    /// from the database must route through this so the two never diverge.
    pub fn finish(mut self) -> Self {
        self.bot = self.owner_id.is_some();
        self
    }
}

/// The signed-in user, or a 401.
pub struct CurrentUser(pub User);

impl FromRequestParts<Arc<AppState>> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, AppError> {
        if let Some(bearer) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        {
            return crate::tokens::verify(&state.db, bearer.trim())
                .await?
                .map(CurrentUser)
                .ok_or_else(AppError::unauthorized);
        }
        let token = session_token(parts).ok_or_else(AppError::unauthorized)?;
        let user: Option<User> = sqlx::query_as(
            "SELECT u.id, u.email, u.display_name, u.color, u.owner_id
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.id = ? AND s.expires_at > ?",
        )
        .bind(&token)
        .bind(now())
        .fetch_optional(&state.db)
        .await?;
        user.map(User::finish)
            .map(CurrentUser)
            .ok_or_else(AppError::unauthorized)
    }
}

/// The session token from the `Cookie` header, if present.
fn session_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .find_map(|kv| {
            kv.strip_prefix(COOKIE_NAME)
                .and_then(|rest| rest.strip_prefix('='))
        })
        .map(str::to_owned)
}

fn set_cookie(token: &str, max_age: i64) -> [(axum::http::HeaderName, String); 1] {
    [(
        SET_COOKIE,
        format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}"),
    )]
}

fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hashing password: {e}").into())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// A hash of a password nobody will type, computed once at first use so
/// `login` can run a real argon2id verification even when the email isn't
/// registered. Without this, the "no such user" and "wrong password" 401s
/// are byte-identical but take different amounts of time — a timing side
/// channel that lets an attacker enumerate registered emails.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("not-a-real-password").expect("hashing the dummy password"));

pub async fn count_users(db: &SqlitePool) -> AppResult<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(db)
        .await?;
    Ok(n)
}

pub async fn create_user(
    db: &SqlitePool,
    email: &str,
    password: &str,
    display_name: &str,
) -> AppResult<User> {
    let email = email.trim().to_ascii_lowercase();
    if !email.contains('@') {
        return Err(AppError::bad_request("enter a valid email address"));
    }
    if password.len() < 8 {
        return Err(AppError::bad_request(
            "password must be at least 8 characters",
        ));
    }
    let display_name = display_name.trim();
    let display_name = if display_name.is_empty() {
        email.clone()
    } else {
        display_name.to_owned()
    };
    let n = count_users(db).await?;
    let color = COLORS[(n as usize) % COLORS.len()];
    let user = User {
        id: Uuid::new_v4().to_string(),
        email,
        display_name,
        color: color.to_owned(),
        owner_id: None,
        bot: false,
    }
    .finish();
    let inserted = sqlx::query(
        "INSERT INTO users (id, email, password_hash, display_name, color, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.email)
    .bind(hash_password(password)?)
    .bind(&user.display_name)
    .bind(&user.color)
    .bind(now())
    .execute(db)
    .await;
    match inserted {
        Ok(_) => Ok(user),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(AppError::bad_request("that email is already registered"))
        }
        Err(e) => Err(e.into()),
    }
}

/// Creates (or finds) the bot user an owner's agents act as. Idempotent:
/// repeat calls for the same owner return the same row. Bots have no
/// password (`password_hash = ''`) and can never log in — see `login`.
#[allow(dead_code)] // Wired up by the MCP agent endpoint; only tests call it until then.
pub async fn ensure_bot(db: &SqlitePool, owner: &User) -> AppResult<User> {
    let existing: Option<User> = sqlx::query_as(
        "SELECT id, email, display_name, color, owner_id FROM users WHERE owner_id = ?",
    )
    .bind(&owner.id)
    .fetch_optional(db)
    .await?;
    if let Some(bot) = existing {
        return Ok(bot.finish());
    }
    let color = if owner.color == "#7c5cd6" {
        "#3fb3b3"
    } else {
        "#7c5cd6"
    };
    let bot = User {
        id: Uuid::new_v4().to_string(),
        email: format!("agent+{}@local", owner.id),
        display_name: "Claude".to_owned(),
        color: color.to_owned(),
        owner_id: Some(owner.id.clone()),
        bot: false,
    }
    .finish();
    let inserted = sqlx::query(
        "INSERT INTO users (id, email, password_hash, display_name, color, created_at, owner_id)
         VALUES (?, ?, '', ?, ?, ?, ?)",
    )
    .bind(&bot.id)
    .bind(&bot.email)
    .bind(&bot.display_name)
    .bind(&bot.color)
    .bind(now())
    .bind(&owner.id)
    .execute(db)
    .await;
    match inserted {
        Ok(_) => Ok(bot),
        // Another call for the same owner won the race between our SELECT
        // and INSERT and already created the bot (same deterministic
        // email). Re-read it so both callers agree on one row instead of
        // one of them surfacing a bare 500.
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            let winner: Option<User> = sqlx::query_as(
                "SELECT id, email, display_name, color, owner_id FROM users WHERE owner_id = ?",
            )
            .bind(&owner.id)
            .fetch_optional(db)
            .await?;
            winner
                .map(User::finish)
                .ok_or_else(|| anyhow::anyhow!("bot insert conflicted but no bot row found").into())
        }
        Err(e) => Err(e.into()),
    }
}

async fn create_session(db: &SqlitePool, user_id: &str) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(user_id)
        .bind(now() + SESSION_SECS)
        .bind(now())
        .execute(db)
        .await?;
    Ok(id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    email: String,
    password: String,
    #[serde(default)]
    display_name: String,
}

/// `POST /api/auth/register` — create an account and sign in.
pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<impl IntoResponse> {
    let user = create_user(&state.db, &req.email, &req.password, &req.display_name).await?;
    if count_users(&state.db).await? == 1 {
        crate::projects::adopt_orphans(&state, &user.id).await?;
    }
    let token = create_session(&state.db, &user.id).await?;
    Ok((AppendHeaders(set_cookie(&token, SESSION_SECS)), Json(user)))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    email: String,
    password: String,
}

/// `POST /api/auth/login`.
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let row: Option<(String, String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, email, display_name, color, password_hash, owner_id FROM users WHERE email = ?",
    )
    .bind(req.email.trim().to_ascii_lowercase())
    .fetch_optional(&state.db)
    .await?;
    let Some((id, email, display_name, color, hash, owner_id)) = row else {
        // Run a real (deliberately slow) verification against a dummy hash
        // so this branch takes about as long as a genuine wrong-password
        // failure below — the two 401s must be indistinguishable in timing,
        // not just in body.
        verify_password(&req.password, &DUMMY_HASH);
        return Err(AppError::unauthorized());
    };
    if owner_id.is_some() {
        // Bots have no password of their own; they authenticate only via
        // the owner's API token. Reject before verifying, so a bot's
        // `password_hash = ''` never gets a chance at argon2 verification.
        return Err(AppError::unauthorized());
    }
    if !verify_password(&req.password, &hash) {
        return Err(AppError::unauthorized());
    }
    let token = create_session(&state.db, &id).await?;
    let user = User {
        id,
        email,
        display_name,
        color,
        owner_id,
        bot: false,
    }
    .finish();
    Ok((AppendHeaders(set_cookie(&token, SESSION_SECS)), Json(user)))
}

/// `POST /api/auth/logout` — forget the session and clear the cookie.
pub async fn logout(
    State(state): State<Arc<AppState>>,
    parts: Parts,
) -> AppResult<impl IntoResponse> {
    if let Some(token) = session_token(&parts) {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(token)
            .execute(&state.db)
            .await?;
    }
    Ok((
        AppendHeaders(set_cookie("", 0)),
        Json(json!({ "ok": true })),
    ))
}

/// `GET /api/me`.
pub async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}

/// `GET /api/auth/setup` — whether the first account still needs creating.
pub async fn setup(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({ "needsSetup": count_users(&state.db).await? == 0 }),
    ))
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::ensure_bot;
    use crate::auth::User;
    use crate::test_util::{app, call, cookie_of, json_req, register, state};

    #[tokio::test]
    async fn register_sets_cookie_and_me_returns_user() {
        let (state, _dir) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (status, body, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&cookie), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["email"], "ada@example.com");
        assert_eq!(body["displayName"], "ada@example.com");
        assert!(body["color"].as_str().unwrap().starts_with('#'));
        assert!(body.get("passwordHash").is_none());
    }

    #[tokio::test]
    async fn me_without_cookie_is_401() {
        let (state, _dir) = state().await;
        let (status, body, _) =
            call(app(&state), json_req(Method::GET, "/api/me", None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "sign in first");
    }

    #[tokio::test]
    async fn duplicate_email_is_400_case_insensitive() {
        let (state, _dir) = state().await;
        register(&state, "ada@example.com").await;
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/register",
                None,
                Some(json!({ "email": "ADA@example.com", "password": "pw-secret", "displayName": "Ada" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn short_password_is_400() {
        let (state, _dir) = state().await;
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/register",
                None,
                Some(json!({ "email": "a@example.com", "password": "short", "displayName": "A" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn login_with_right_and_wrong_password() {
        let (state, _dir) = state().await;
        register(&state, "ada@example.com").await;
        let (status, _, headers) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/login",
                None,
                Some(json!({ "email": "ada@example.com", "password": "pw-secret" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(cookie_of(&headers).is_some());

        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/login",
                None,
                Some(json!({ "email": "ada@example.com", "password": "nope" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_with_unknown_email_is_401() {
        let (state, _dir) = state().await;
        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/login",
                None,
                Some(json!({ "email": "nobody@example.com", "password": "pw-secret" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, json!({ "error": "sign in first" }));
    }

    #[tokio::test]
    async fn logout_invalidates_the_session() {
        let (state, _dir) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (status, _, _) = call(
            app(&state),
            json_req(Method::POST, "/api/auth/logout", Some(&cookie), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&cookie), None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn expired_session_is_401() {
        let (state, _dir) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        sqlx::query("UPDATE sessions SET expires_at = 0")
            .execute(&state.db)
            .await
            .unwrap();
        let (status, _, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&cookie), None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn setup_reports_whether_any_user_exists() {
        let (state, _dir) = state().await;
        let (_, body, _) = call(
            app(&state),
            json_req(Method::GET, "/api/auth/setup", None, None),
        )
        .await;
        assert_eq!(body["needsSetup"], true);
        register(&state, "ada@example.com").await;
        let (_, body, _) = call(
            app(&state),
            json_req(Method::GET, "/api/auth/setup", None, None),
        )
        .await;
        assert_eq!(body["needsSetup"], false);
    }

    #[tokio::test]
    async fn ensure_bot_is_idempotent_and_bots_cannot_log_in() {
        let (state, _d) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (_, me, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&cookie), None),
        )
        .await;
        let owner: User = serde_json::from_value(me).unwrap();
        let a = ensure_bot(&state.db, &owner).await.unwrap();
        let b = ensure_bot(&state.db, &owner).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.display_name, "Claude");
        assert_eq!(a.owner_id.as_deref(), Some(owner.id.as_str()));
        assert_ne!(a.color, owner.color);
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                "/api/auth/login",
                None,
                Some(json!({ "email": a.email, "password": "anything-8" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// Several racing `ensure_bot` calls for the same owner all miss the
    /// `SELECT ... WHERE owner_id = ?` before any of them has inserted, so
    /// only one INSERT can win — the deterministic email makes the rest
    /// hit the unique constraint. Every caller must still get back the
    /// same bot row instead of one of them surfacing a bare 500.
    #[tokio::test]
    async fn ensure_bot_survives_a_concurrent_insert_race() {
        let (state, _d) = state().await;
        let cookie = register(&state, "ada@example.com").await;
        let (_, me, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&cookie), None),
        )
        .await;
        let owner: User = serde_json::from_value(me).unwrap();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let db = state.db.clone();
            let owner = owner.clone();
            handles.push(tokio::spawn(async move { ensure_bot(&db, &owner).await }));
        }
        let mut ids = Vec::new();
        for handle in handles {
            let bot = handle.await.unwrap().unwrap();
            ids.push(bot.id);
        }
        assert!(
            ids.iter().all(|id| *id == ids[0]),
            "every racer should agree on one bot: {ids:?}"
        );
    }
}
