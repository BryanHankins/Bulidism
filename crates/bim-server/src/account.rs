//! Self-service account endpoints: email verification, data export and account deletion.
//!
//! Verification is enforced only where an unproven address could leak data: a project can be shared
//! only with a verified account (otherwise anyone could register someone else's address and receive
//! their colleagues' projects). Everything else works before verifying. Without email configured
//! nothing is enforced, since nobody could verify.

use crate::auth::{hash_token, random_token, verify_password, AuthUser};
use crate::error::{ApiError, ApiJson};
use crate::mail::{send_in_background, Mailer};
use crate::ratelimit::ClientIp;
use crate::AppState;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::types::Json as SqlJson;
use std::sync::Arc;
use uuid::Uuid;

/// Verification link lifetime.
pub const VERIFY_HOURS: i32 = 48;

/// The mailer and the link base, when this server can send email.
pub fn email_enabled(state: &AppState) -> Option<(Arc<dyn Mailer>, String)> {
    Some((state.mailer.clone()?, state.public_url.clone()?))
}

/// Mails `<PUBLIC_URL>/#verify=<token>` to a new account; a no-op without email.
pub async fn send_verification(state: &AppState, user: Uuid, email: &str) -> Result<(), ApiError> {
    let Some((mailer, base)) = email_enabled(state) else {
        return Ok(());
    };
    sqlx::query("DELETE FROM email_verifications WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    let token = random_token();
    sqlx::query(
        "INSERT INTO email_verifications (token_hash, user_id, expires_at)
         VALUES ($1, $2, now() + make_interval(hours => $3))",
    )
    .bind(hash_token(&token))
    .bind(user)
    .bind(VERIFY_HOURS)
    .execute(&state.db)
    .await?;
    let body = format!(
        "Welcome to buildsim. Confirm that {email} is your address (the link works for \
         {VERIFY_HOURS} hours):\n\
         {base}/#verify={token}\n\n\
         Until you do, other people cannot share projects with this account.\n\
         If you did not sign up, ignore this email."
    );
    send_in_background(
        mailer,
        email.to_owned(),
        "Confirm your buildsim email",
        body,
    );
    Ok(())
}

#[derive(Deserialize)]
pub struct VerifyBody {
    token: String,
}

/// `POST /api/auth/verify {token}` -> 204; marks the address verified. No session needed (the link
/// may be opened on another device). 400 for an unknown or expired token.
pub async fn verify(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(b): ApiJson<VerifyBody>,
) -> Result<StatusCode, ApiError> {
    state
        .limiter
        .hit("verify_ip", &ip, state.limits.reset_confirm_ip)?;
    let mut tx = state.db.begin().await?;
    let user: Option<Uuid> = sqlx::query_scalar(
        "DELETE FROM email_verifications WHERE token_hash = $1 AND expires_at > now()
         RETURNING user_id",
    )
    .bind(hash_token(b.token.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let user = user.ok_or(ApiError::BadRequest(
        "this verification link is invalid or has expired".into(),
    ))?;
    sqlx::query(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1",
    )
    .bind(user)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM email_verifications WHERE user_id = $1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/auth/verify/resend` (auth) -> 202 (sent) or 204 (already verified); 503 without email.
pub async fn resend(State(state): State<AppState>, auth: AuthUser) -> Result<StatusCode, ApiError> {
    if auth.user.email_verified {
        return Ok(StatusCode::NO_CONTENT);
    }
    if email_enabled(&state).is_none() {
        return Err(ApiError::Unavailable(
            "email is not configured on this server",
        ));
    }
    state.limiter.hit(
        "verify_resend",
        &auth.user.id.to_string(),
        state.limits.reset_email,
    )?;
    send_verification(&state, auth.user.id, &auth.user.email).await?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub struct DeleteBody {
    password: String,
}

/// `DELETE /api/me {password}` -> 204. Re-checks the password, then deletes the account with its
/// sessions, pending links, shares and every project it owns (also for the people they were shared
/// with). 400 on a wrong password (not 401, which would sign the client out).
pub async fn delete_account(
    State(state): State<AppState>,
    auth: AuthUser,
    ApiJson(b): ApiJson<DeleteBody>,
) -> Result<StatusCode, ApiError> {
    let key = auth.user.id.to_string();
    let lim = state.limits.login_email_failures;
    state.limiter.check("delete_password", &key, lim)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(auth.user.id)
        .fetch_one(&state.db)
        .await?;
    if b.password.chars().count() > 1024 || !verify_password(b.password, hash).await? {
        let _ = state.limiter.hit("delete_password", &key, lim);
        return Err(ApiError::BadRequest("password is incorrect".into()));
    }
    // ON DELETE CASCADE removes sessions, resets, verifications, owned projects and all shares
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(auth.user.id)
        .execute(&state.db)
        .await?;
    tracing::info!("account {} deleted by its owner", auth.user.id);
    Ok(StatusCode::NO_CONTENT)
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

type Owned = (Uuid, String, DateTime<Utc>, DateTime<Utc>, SqlJson<Value>);
type SharedWithMe = (Uuid, String, String, String, DateTime<Utc>, SqlJson<Value>);

/// `GET /api/me/export` (auth) -> a JSON download of everything stored about the caller: account
/// details, owned projects (with data and who they are shared with) and projects shared with them.
/// Never includes the password hash or session tokens.
pub async fn export(State(state): State<AppState>, auth: AuthUser) -> Result<Response, ApiError> {
    let id = auth.user.id;
    let (email, created, verified): (String, DateTime<Utc>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT email, created_at, email_verified_at FROM users WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let owned: Vec<Owned> = sqlx::query_as(
        "SELECT id, name, created_at, updated_at, data FROM projects
         WHERE owner_id = $1 ORDER BY created_at, id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let grants: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT s.project_id, u.email, s.role FROM project_shares s
         JOIN projects p ON p.id = s.project_id JOIN users u ON u.id = s.user_id
         WHERE p.owner_id = $1 ORDER BY u.email",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let shared: Vec<SharedWithMe> = sqlx::query_as(
        "SELECT p.id, p.name, s.role, o.email, p.updated_at, p.data FROM project_shares s
         JOIN projects p ON p.id = s.project_id JOIN users o ON o.id = p.owner_id
         WHERE s.user_id = $1 ORDER BY p.name, p.id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let now = Utc::now();
    let doc = json!({
        "format": "buildsim-account-export/1",
        "exported_at": ts(now),
        "account": {
            "id": id, "email": email, "created_at": ts(created),
            "email_verified_at": verified.map(ts),
        },
        "projects": owned.into_iter().map(|(pid, name, c, u, data)| json!({
            "id": pid, "name": name, "created_at": ts(c), "updated_at": ts(u),
            "shared_with": grants.iter().filter(|g| g.0 == pid)
                .map(|g| json!({"email": g.1, "role": g.2})).collect::<Vec<_>>(),
            "data": data.0,
        })).collect::<Vec<_>>(),
        "shared_with_me": shared.into_iter().map(|(pid, name, role, owner, u, data)| json!({
            "id": pid, "name": name, "role": role, "owner_email": owner,
            "updated_at": ts(u), "data": data.0,
        })).collect::<Vec<_>>(),
    });
    let file = format!("buildsim-export-{}.json", now.format("%Y-%m-%d"));
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        Json(doc),
    )
        .into_response())
}
