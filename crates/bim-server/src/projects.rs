//! Project storage (JSONB) with owner/editor/viewer sharing and optimistic concurrency.

use crate::auth::{normalize_email, AuthUser};
use crate::error::{ApiError, ApiJson};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::types::Json as SqlJson;
use uuid::Uuid;

const MAX_NAME: usize = 200;
const NOT_FOUND: ApiError = ApiError::NotFound("project not found");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Owner,
    Editor,
    Viewer,
}

impl Role {
    fn parse(s: &str) -> Option<Role> {
        match s {
            "owner" => Some(Role::Owner),
            "editor" => Some(Role::Editor),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Viewer => "viewer",
        }
    }
}

#[derive(Serialize)]
pub struct ProjectMeta {
    id: Uuid,
    name: String,
    role: &'static str,
    updated_at: String,
}

#[derive(Serialize)]
pub struct ProjectFull {
    #[serde(flatten)]
    meta: ProjectMeta,
    data: Value,
}

#[derive(Deserialize)]
pub struct CreateProject {
    name: String,
    data: Value,
}

#[derive(Deserialize)]
pub struct UpdateProject {
    name: Option<String>,
    data: Option<Value>,
    updated_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Share {
    email: String,
    role: String,
}

/// Microsecond RFC 3339 (Postgres precision), so clients can echo it back exactly.
fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn meta(id: Uuid, name: String, role: Role, updated_at: DateTime<Utc>) -> ProjectMeta {
    ProjectMeta {
        id,
        name,
        role: role.as_str(),
        updated_at: stamp(updated_at),
    }
}

/// Unknown or unparsable ids are a plain 404, like projects the caller cannot see.
fn project_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| NOT_FOUND)
}

fn check_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ApiError::BadRequest(format!(
            "name must be 1-{MAX_NAME} characters"
        )));
    }
    Ok(name.to_string())
}

/// Project data must be an object that loads as a `bim_core::Project`.
fn check_data(data: &Value) -> Result<(), ApiError> {
    if !data.is_object() {
        return Err(ApiError::Unprocessable(
            "data must be a project object".into(),
        ));
    }
    bim_core::Project::from_json(&data.to_string())
        .map(|_| ())
        .map_err(|e| ApiError::Unprocessable(format!("invalid project data: {e}")))
}

/// The caller's role on a project; 404 when it does not exist or is not shared with them.
async fn role_of(state: &AppState, project: Uuid, user: Uuid) -> Result<Role, ApiError> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT CASE WHEN p.owner_id = $2 THEN 'owner' ELSE s.role END
         FROM projects p
         LEFT JOIN project_shares s ON s.project_id = p.id AND s.user_id = $2
         WHERE p.id = $1",
    )
    .bind(project)
    .bind(user)
    .fetch_optional(&state.db)
    .await?;
    row.and_then(|(r,)| r)
        .as_deref()
        .and_then(Role::parse)
        .ok_or(NOT_FOUND)
}

async fn require_owner(state: &AppState, project: Uuid, user: Uuid) -> Result<(), ApiError> {
    match role_of(state, project, user).await? {
        Role::Owner => Ok(()),
        _ => Err(ApiError::Forbidden),
    }
}

/// `GET /api/projects` -> owned and shared projects, most recently updated first.
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ProjectMeta>>, ApiError> {
    let rows: Vec<(Uuid, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT p.id, p.name, CASE WHEN p.owner_id = $1 THEN 'owner' ELSE s.role END, p.updated_at
         FROM projects p
         LEFT JOIN project_shares s ON s.project_id = p.id AND s.user_id = $1
         WHERE p.owner_id = $1 OR s.user_id IS NOT NULL
         ORDER BY p.updated_at DESC, p.id",
    )
    .bind(auth.user.id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .filter_map(|(id, name, role, t)| Some(meta(id, name, Role::parse(&role)?, t)))
            .collect(),
    ))
}

/// `POST /api/projects` -> 201 with the new project's metadata.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(body): ApiJson<CreateProject>,
) -> Result<(StatusCode, Json<ProjectMeta>), ApiError> {
    let name = check_name(&body.name)?;
    check_data(&body.data)?;
    let id = Uuid::new_v4();
    let updated_at: DateTime<Utc> = sqlx::query_scalar(
        "INSERT INTO projects (id, owner_id, name, data) VALUES ($1, $2, $3, $4)
         RETURNING updated_at",
    )
    .bind(id)
    .bind(auth.user.id)
    .bind(&name)
    .bind(SqlJson(&body.data))
    .fetch_one(&state.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(meta(id, name, Role::Owner, updated_at)),
    ))
}

/// `GET /api/projects/{id}` -> metadata plus `data`, for any role.
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<ProjectFull>, ApiError> {
    let id = project_id(&id)?;
    let role = role_of(&state, id, auth.user.id).await?;
    let row: Option<(String, DateTime<Utc>, SqlJson<Value>)> =
        sqlx::query_as("SELECT name, updated_at, data FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (name, t, SqlJson(data)) = row.ok_or(NOT_FOUND)?;
    Ok(Json(ProjectFull {
        meta: meta(id, name, role, t),
        data,
    }))
}

/// `PUT /api/projects/{id}` (owner or editor). A stale `updated_at` is a 409.
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateProject>,
) -> Result<Json<ProjectMeta>, ApiError> {
    let id = project_id(&id)?;
    let role = role_of(&state, id, auth.user.id).await?;
    if role == Role::Viewer {
        return Err(ApiError::Forbidden);
    }
    let name = body.name.as_deref().map(check_name).transpose()?;
    if let Some(data) = &body.data {
        check_data(data)?;
    }
    let expected = body
        .updated_at
        .as_deref()
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|_| ApiError::BadRequest("updated_at must be RFC 3339".into()))
        })
        .transpose()?;
    // The timestamp always moves forward, so every write yields a fresh version.
    let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        "UPDATE projects SET
             name = COALESCE($2, name),
             data = COALESCE($3, data),
             updated_at = GREATEST(now(), updated_at + interval '1 microsecond')
         WHERE id = $1 AND ($4::timestamptz IS NULL OR updated_at = $4)
         RETURNING name, updated_at",
    )
    .bind(id)
    .bind(name)
    .bind(body.data.as_ref().map(SqlJson))
    .bind(expected)
    .fetch_optional(&state.db)
    .await?;
    let (name, t) = row.ok_or(ApiError::Conflict(
        "project was modified since updated_at; reload and retry",
    ))?;
    Ok(Json(meta(id, name, role, t)))
}

/// `DELETE /api/projects/{id}` (owner) -> 204.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = project_id(&id)?;
    require_owner(&state, id, auth.user.id).await?;
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/projects/{id}/shares` (owner) -> `[{email, role}]` sorted by email.
pub async fn list_shares(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<Share>>, ApiError> {
    let id = project_id(&id)?;
    require_owner(&state, id, auth.user.id).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT u.email, s.role FROM project_shares s JOIN users u ON u.id = s.user_id
         WHERE s.project_id = $1 ORDER BY u.email",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(email, role)| Share { email, role })
            .collect(),
    ))
}

/// `PUT /api/projects/{id}/shares` (owner) -> 204; adds or changes a share. 404 if no such user;
/// 409 if that account has not verified its email (when this server sends email).
pub async fn put_share(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<Share>,
) -> Result<StatusCode, ApiError> {
    let id = project_id(&id)?;
    require_owner(&state, id, auth.user.id).await?;
    if !matches!(Role::parse(&body.role), Some(Role::Editor | Role::Viewer)) {
        return Err(ApiError::BadRequest(
            "role must be \"editor\" or \"viewer\"".into(),
        ));
    }
    let email = normalize_email(&body.email)?;
    let user: Option<(Uuid, bool)> =
        sqlx::query_as("SELECT id, email_verified_at IS NOT NULL FROM users WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;
    let (user, verified) = user.ok_or(ApiError::NotFound("no user with that email"))?;
    // an unverified account may belong to someone who registered another person's address
    if !verified && crate::account::email_enabled(&state).is_some() {
        return Err(ApiError::Conflict(
            "that account has not verified its email yet; ask them to use the link in their welcome email",
        ));
    }
    if user == auth.user.id {
        return Err(ApiError::BadRequest(
            "cannot share a project with its owner".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO project_shares (project_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(id)
    .bind(user)
    .bind(&body.role)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/projects/{id}/shares/{email}` (owner) -> 204, whether or not it was shared.
pub async fn delete_share(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, email)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = project_id(&id)?;
    require_owner(&state, id, auth.user.id).await?;
    sqlx::query(
        "DELETE FROM project_shares
         WHERE project_id = $1 AND user_id = (SELECT id FROM users WHERE email = $2)",
    )
    .bind(id)
    .bind(email.trim().to_lowercase())
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
