//! Projects, membership and roles. A project is one media directory plus its
//! operation log; membership decides who may read, comment on or edit it.

use std::sync::Arc;

use axum::extract::{FromRequestParts, Path as UrlPath, RawPathParams, State};
use axum::http::request::Parts;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::{CurrentUser, User};
use crate::db::now;
use crate::error::{AppError, AppResult};
use crate::routes::{read_meta, Meta};
use crate::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Editor,
    Commenter,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Commenter => "commenter",
            Role::Viewer => "viewer",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "owner" => Some(Role::Owner),
            "editor" => Some(Role::Editor),
            "commenter" => Some(Role::Commenter),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    pub fn can_edit(self) -> bool {
        matches!(self, Role::Owner | Role::Editor)
    }

    pub fn can_manage(self) -> bool {
        self == Role::Owner
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub media_id: String,
    pub owner_id: String,
    pub title: String,
    pub created_at: i64,
}

/// A signed-in user's access to the project named by the `{id}` path
/// parameter. Rejections, in order: 401 (no session), 404 (no such project),
/// 403 (not a member).
pub struct ProjectAccess {
    pub user: User,
    pub project: Project,
    pub role: Role,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub title: String,
    pub role: Role,
    pub media: Meta,
    pub created_at: i64,
}

impl ProjectAccess {
    pub fn require_edit(&self) -> AppResult<()> {
        if self.role.can_edit() {
            Ok(())
        } else {
            Err(AppError::forbidden(
                "you can view this project but not edit it",
            ))
        }
    }

    pub fn require_manage(&self) -> AppResult<()> {
        if self.role.can_manage() {
            Ok(())
        } else {
            Err(AppError::forbidden("only the owner can manage members"))
        }
    }
}

impl FromRequestParts<Arc<AppState>> for ProjectAccess {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, AppError> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        let params = RawPathParams::from_request_parts(parts, state)
            .await
            .map_err(|e| AppError::bad_request(e.to_string()))?;
        let id = params
            .iter()
            .find(|(k, _)| *k == "id")
            .map(|(_, v)| v.to_owned())
            .ok_or_else(|| AppError::bad_request("missing project id"))?;
        let project = find_project(&state.db, &id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("no project with id {id}")))?;
        let role = member_role(&state.db, &project.id, &user.id)
            .await?
            .ok_or_else(|| AppError::forbidden("you are not a member of this project"))?;
        Ok(ProjectAccess {
            user,
            project,
            role,
        })
    }
}

pub async fn find_project(db: &SqlitePool, id: &str) -> AppResult<Option<Project>> {
    Ok(sqlx::query_as(
        "SELECT id, media_id, owner_id, title, created_at FROM projects WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

pub(crate) async fn member_role(
    db: &SqlitePool,
    project_id: &str,
    user_id: &str,
) -> AppResult<Option<Role>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT role FROM project_members WHERE project_id = ? AND user_id = ?")
            .bind(project_id)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(row.and_then(|(r,)| Role::parse(&r)))
}

/// A stored role string as a `Role`. An unrecognised value (a hand-edited row,
/// or a role from a newer build) is treated as the least privileged one.
fn role_or_viewer(role: &str) -> Role {
    Role::parse(role).unwrap_or_else(|| {
        tracing::warn!(role, "unknown role in project_members; treating as viewer");
        Role::Viewer
    })
}

pub async fn create_project(
    db: &SqlitePool,
    owner: &User,
    media_id: &str,
    title: &str,
) -> AppResult<Project> {
    let project = Project {
        id: Uuid::new_v4().to_string(),
        media_id: media_id.to_owned(),
        owner_id: owner.id.clone(),
        title: title.to_owned(),
        created_at: now(),
    };
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO projects (id, media_id, owner_id, title, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&project.id)
    .bind(&project.media_id)
    .bind(&project.owner_id)
    .bind(&project.title)
    .bind(project.created_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO project_members (project_id, user_id, role) VALUES (?, ?, 'owner')")
        .bind(&project.id)
        .bind(&owner.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(project)
}

pub async fn summary(state: &AppState, project: &Project, role: Role) -> AppResult<ProjectSummary> {
    let media = read_meta(&state.config.data_dir.join(&project.media_id)).await?;
    Ok(ProjectSummary {
        id: project.id.clone(),
        title: project.title.clone(),
        role,
        media,
        created_at: project.created_at,
    })
}

/// `GET /api/projects` — every project the user is a member of, newest first.
pub async fn list(
    State(state): State<Arc<AppState>>,
    CurrentUser(user): CurrentUser,
) -> AppResult<Json<Vec<ProjectSummary>>> {
    let rows: Vec<(String, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT p.id, p.media_id, p.owner_id, p.title, p.created_at, m.role
         FROM projects p JOIN project_members m ON m.project_id = p.id
         WHERE m.user_id = ? ORDER BY p.created_at DESC",
    )
    .bind(&user.id)
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, media_id, owner_id, title, created_at, role) in rows {
        let project = Project {
            id,
            media_id,
            owner_id,
            title,
            created_at,
        };
        let role = role_or_viewer(&role);
        // A project whose media directory vanished is skipped, not fatal.
        match summary(&state, &project, role).await {
            Ok(s) => out.push(s),
            Err(e) => {
                tracing::warn!(
                    project = project.id,
                    media = project.media_id,
                    "skipping project: {e:?}"
                );
            }
        }
    }
    Ok(Json(out))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    #[serde(flatten)]
    user: User,
    role: Role,
}

/// `GET /api/projects/:id/members`.
pub async fn members(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
) -> AppResult<Json<Vec<Member>>> {
    Ok(Json(list_members(&state.db, &access.project.id).await?))
}

async fn list_members(db: &SqlitePool, project_id: &str) -> AppResult<Vec<Member>> {
    let rows: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, u.color, u.owner_id, m.role
         FROM project_members m JOIN users u ON u.id = m.user_id
         WHERE m.project_id = ? ORDER BY m.role, u.display_name",
    )
    .bind(project_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, email, display_name, color, owner_id, role)| Member {
            user: User {
                id,
                email,
                display_name,
                color,
                owner_id,
            },
            role: role_or_viewer(&role),
        })
        .collect())
}

#[derive(Deserialize)]
pub struct AddMember {
    email: String,
    role: String,
}

/// `POST /api/projects/:id/members` — add or re-role a member by email.
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    Json(req): Json<AddMember>,
) -> AppResult<Json<Vec<Member>>> {
    access.require_manage()?;
    let role = Role::parse(&req.role)
        .filter(|r| *r != Role::Owner)
        .ok_or_else(|| AppError::bad_request("role must be editor, commenter or viewer"))?;
    let user: Option<(String,)> = sqlx::query_as("SELECT id FROM users WHERE email = ?")
        .bind(req.email.trim().to_ascii_lowercase())
        .fetch_optional(&state.db)
        .await?;
    let Some((user_id,)) = user else {
        return Err(AppError::bad_request("no account with that email"));
    };
    if user_id == access.project.owner_id {
        return Err(AppError::bad_request("the owner's role cannot change"));
    }
    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role) VALUES (?, ?, ?)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
    )
    .bind(&access.project.id)
    .bind(&user_id)
    .bind(role.as_str())
    .execute(&state.db)
    .await?;
    Ok(Json(list_members(&state.db, &access.project.id).await?))
}

/// `DELETE /api/projects/:id/members/:user_id`.
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    access: ProjectAccess,
    UrlPath((_, user_id)): UrlPath<(String, String)>,
) -> AppResult<Json<Value>> {
    access.require_manage()?;
    if user_id == access.project.owner_id {
        return Err(AppError::bad_request("the owner cannot be removed"));
    }
    sqlx::query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
        .bind(&access.project.id)
        .bind(&user_id)
        .execute(&state.db)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// Give every media directory that has no project yet to `owner_id`. Runs at
/// startup for the admin user and after the first registration.
pub async fn adopt_orphans(state: &Arc<AppState>, owner_id: &str) -> AppResult<()> {
    let owner: Option<User> =
        sqlx::query_as("SELECT id, email, display_name, color, owner_id FROM users WHERE id = ?")
            .bind(owner_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(owner) = owner else { return Ok(()) };
    let mut entries = tokio::fs::read_dir(&state.config.data_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let dir = entry.path();
        if !dir.join("meta.json").is_file() {
            continue;
        }
        let meta = match read_meta(&dir).await {
            Ok(meta) => meta,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), "skipping media dir: {e:?}");
                continue;
            }
        };
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects WHERE media_id = ?")
            .bind(&meta.id)
            .fetch_one(&state.db)
            .await?;
        if n == 0 {
            create_project(&state.db, &owner, &meta.id, &meta.filename).await?;
            tracing::info!(media = meta.id, "adopted orphan media into a project");
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::routes::Meta;
    use crate::AppState;

    /// A fake media item on disk: just `meta.json`, enough for project routes.
    pub async fn seed_media(state: &Arc<AppState>, duration: f64) -> String {
        let id = Uuid::new_v4().to_string();
        let dir = state.config.data_dir.join(&id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let meta = Meta {
            id: id.clone(),
            filename: "clip.mp4".into(),
            ext: "mp4".into(),
            duration,
            kind: engine::MediaKind::Video,
            url: format!("/data/{id}/source.mp4"),
            video: None,
        };
        tokio::fs::write(dir.join("meta.json"), serde_json::to_vec(&meta).unwrap())
            .await
            .unwrap();
        id
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use serde_json::json;

    use super::test_support::seed_media;
    use super::*;
    use crate::test_util::{app, call, json_req, register, state};

    async fn user_id(state: &Arc<AppState>, cookie: &str) -> String {
        let (_, me, _) = call(
            app(state),
            json_req(Method::GET, "/api/me", Some(cookie), None),
        )
        .await;
        me["id"].as_str().unwrap().to_owned()
    }

    async fn owned_project(state: &Arc<AppState>, cookie: &str) -> Project {
        let media_id = seed_media(state, 10.0).await;
        let (_, me, _) = call(
            app(state),
            json_req(Method::GET, "/api/me", Some(cookie), None),
        )
        .await;
        let owner: User = serde_json::from_value(me).unwrap();
        create_project(&state.db, &owner, &media_id, "Clip")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn list_shows_only_projects_the_user_belongs_to() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let (_, list, _) = call(
            app(&state),
            json_req(Method::GET, "/api/projects", Some(&ada), None),
        )
        .await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"], project.id);
        assert_eq!(list[0]["role"], "owner");
        assert_eq!(list[0]["media"]["duration"], 10.0);
        let (_, list, _) = call(
            app(&state),
            json_req(Method::GET, "/api/projects", Some(&bob), None),
        )
        .await;
        assert_eq!(list.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn members_route_distinguishes_401_404_403() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let uri = format!("/api/projects/{}/members", project.id);

        let (status, _, _) = call(app(&state), json_req(Method::GET, &uri, None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::GET,
                "/api/projects/does-not-exist/members",
                Some(&ada),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _, _) = call(app(&state), json_req(Method::GET, &uri, Some(&bob), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body, _) =
            call(app(&state), json_req(Method::GET, &uri, Some(&ada), None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn owner_adds_and_removes_a_member_by_email() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let bob = register(&state, "bob@example.com").await;
        let project = owned_project(&state, &ada).await;
        let uri = format!("/api/projects/{}/members", project.id);

        let (status, body, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &uri,
                Some(&ada),
                Some(json!({ "email": "bob@example.com", "role": "viewer" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, list, _) = call(
            app(&state),
            json_req(Method::GET, "/api/projects", Some(&bob), None),
        )
        .await;
        assert_eq!(list[0]["role"], "viewer");

        // A viewer cannot manage members.
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &uri,
                Some(&bob),
                Some(json!({ "email": "ada@example.com", "role": "editor" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let bob_id = user_id(&state, &bob).await;
        let (status, _, _) = call(
            app(&state),
            json_req(Method::DELETE, &format!("{uri}/{bob_id}"), Some(&ada), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call(app(&state), json_req(Method::GET, &uri, Some(&bob), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unknown_email_and_bad_role_are_400_and_owner_cannot_be_removed() {
        let (state, _dir) = state().await;
        let ada = register(&state, "ada@example.com").await;
        let project = owned_project(&state, &ada).await;
        let uri = format!("/api/projects/{}/members", project.id);
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &uri,
                Some(&ada),
                Some(json!({ "email": "zed@example.com", "role": "viewer" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _, _) = call(
            app(&state),
            json_req(
                Method::POST,
                &uri,
                Some(&ada),
                Some(json!({ "email": "ada@example.com", "role": "boss" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let ada_id = user_id(&state, &ada).await;
        let (status, _, _) = call(
            app(&state),
            json_req(Method::DELETE, &format!("{uri}/{ada_id}"), Some(&ada), None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn adopt_orphans_creates_a_project_per_media_dir_once() {
        let (state, _dir) = state().await;
        seed_media(&state, 4.0).await;
        seed_media(&state, 5.0).await;
        let ada = register(&state, "ada@example.com").await; // first user adopts
        let (_, list, _) = call(
            app(&state),
            json_req(Method::GET, "/api/projects", Some(&ada), None),
        )
        .await;
        assert_eq!(list.as_array().unwrap().len(), 2);
        let (_, me, _) = call(
            app(&state),
            json_req(Method::GET, "/api/me", Some(&ada), None),
        )
        .await;
        adopt_orphans(&state, me["id"].as_str().unwrap())
            .await
            .unwrap();
        let (_, list, _) = call(
            app(&state),
            json_req(Method::GET, "/api/projects", Some(&ada), None),
        )
        .await;
        assert_eq!(list.as_array().unwrap().len(), 2);
    }
}
