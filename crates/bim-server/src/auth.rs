//! Accounts and bearer-token sessions: argon2 passwords, sha256-hashed random tokens, rate-limited
//! login/register, and emailed single-use password-reset links.

use crate::error::{ApiError, ApiJson};
use crate::ratelimit::ClientIp;
use crate::AppState;
use argon2::password_hash::{rand_core::OsRng, PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::Json;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Session lifetime.
pub const SESSION_DAYS: i64 = 30;
/// Password-reset link lifetime.
pub const RESET_MINUTES: i32 = 60;
const MIN_PASSWORD: usize = 8;
/// Upper bound keeps argon2 work per request bounded.
const MAX_PASSWORD: usize = 1024;

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
}

#[derive(Deserialize)]
pub struct Credentials {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub struct AuthResponse {
    user: User,
    token: String,
}

/// The authenticated caller, resolved from `Authorization: Bearer <token>`.
pub struct AuthUser {
    pub user: User,
    token_hash: Vec<u8>,
}

/// sha256 of the token as sent by the client; this is what `sessions` stores.
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Trims and lowercases; rejects anything not shaped like `local@domain.tld`.
pub fn normalize_email(raw: &str) -> Result<String, ApiError> {
    let email = raw.trim().to_lowercase();
    let ok = email.len() <= 254
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
        && match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && !domain.contains('@')
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                    && !domain.contains("..")
            }
            None => false,
        };
    if ok {
        Ok(email)
    } else {
        Err(ApiError::BadRequest("invalid email".into()))
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(ApiError::Unauthorized("missing bearer token"))?;
        let token_hash = hash_token(token);
        let row: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT u.id, u.email FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(&token_hash)
        .fetch_optional(&state.db)
        .await?;
        let (id, email) = row.ok_or(ApiError::Unauthorized("invalid or expired token"))?;
        Ok(AuthUser {
            user: User { id, email },
            token_hash,
        })
    }
}

/// 32 random bytes, hex-encoded.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Rejects passwords outside `MIN_PASSWORD..=MAX_PASSWORD` characters.
fn check_password(password: &str) -> Result<(), ApiError> {
    let n = password.chars().count();
    if n < MIN_PASSWORD {
        return Err(ApiError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD} characters"
        )));
    }
    if n > MAX_PASSWORD {
        return Err(ApiError::BadRequest("password too long".into()));
    }
    Ok(())
}

/// Creates a session for `user_id` and returns the plaintext token (shown once).
async fn new_session(state: &AppState, user_id: Uuid) -> Result<String, ApiError> {
    let token = random_token();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, expires_at)
         VALUES ($1, $2, now() + make_interval(days => $3))",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(SESSION_DAYS as i32)
    .execute(&state.db)
    .await?;
    Ok(token)
}

async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| ApiError::Internal(format!("argon2: {e}")))
    })
    .await
    .map_err(|e| ApiError::Internal(format!("join: {e}")))?
}

async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        let parsed =
            PasswordHash::new(&hash).map_err(|e| ApiError::Internal(format!("argon2: {e}")))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("join: {e}")))?
}

/// `POST /api/auth/register` -> 201 `{user, token}`; 409 if the email is taken.
pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(c): ApiJson<Credentials>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    state
        .limiter
        .hit("register_ip", &ip, state.limits.register_ip)?;
    let email = normalize_email(&c.email)?;
    check_password(&c.password)?;
    let hash = hash_password(c.password).await?;
    let id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)
         ON CONFLICT (email) DO NOTHING",
    )
    .bind(id)
    .bind(&email)
    .bind(&hash)
    .execute(&state.db)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::Conflict("email already registered"));
    }
    let token = new_session(&state, id).await?;
    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            user: User { id, email },
            token,
        }),
    ))
}

/// `POST /api/auth/login` -> `{user, token}`; 401 on bad credentials, 429 when rate limited.
pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(c): ApiJson<Credentials>,
) -> Result<Json<AuthResponse>, ApiError> {
    const BAD: ApiError = ApiError::Unauthorized("invalid email or password");
    let lim = state.limits;
    state.limiter.hit("login_ip", &ip, lim.login_ip)?;
    let Ok(email) = normalize_email(&c.email) else {
        return Err(BAD);
    };
    state
        .limiter
        .check("login_email", &email, lim.login_email_failures)?;
    let fail = || {
        // only failures count per email, so the real owner can still sign in
        let _ = state
            .limiter
            .hit("login_email", &email, lim.login_email_failures);
        BAD
    };
    if c.password.chars().count() > MAX_PASSWORD {
        return Err(fail());
    }
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;
    let Some((id, hash)) = row else {
        return Err(fail());
    };
    if !verify_password(c.password, hash).await? {
        return Err(fail());
    }
    // Opportunistic cleanup keeps the sessions table from growing without bound.
    sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    let token = new_session(&state, id).await?;
    Ok(Json(AuthResponse {
        user: User { id, email },
        token,
    }))
}

/// `POST /api/auth/logout` -> 204; revokes the calling token only.
pub async fn logout(State(state): State<AppState>, auth: AuthUser) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(&auth.token_hash)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ResetRequest {
    email: String,
}

#[derive(Deserialize)]
pub struct ResetConfirm {
    token: String,
    password: String,
}

/// `POST /api/auth/password-reset {email}` -> 202 whether or not the account exists (no
/// enumeration); if it does, mails a single-use link `<PUBLIC_URL>/#reset=<token>` valid for
/// `RESET_MINUTES`. 503 when email is not configured, 429 when rate limited.
pub async fn reset_request(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(r): ApiJson<ResetRequest>,
) -> Result<StatusCode, ApiError> {
    const OFF: ApiError = ApiError::Unavailable("password reset is not configured on this server");
    let (Some(mailer), Some(base)) = (state.mailer.clone(), state.public_url.clone()) else {
        return Err(OFF);
    };
    state.limiter.hit("reset_ip", &ip, state.limits.reset_ip)?;
    let email = normalize_email(&r.email)?;
    // per address, so nobody can flood an inbox; applies to unknown addresses too
    state
        .limiter
        .hit("reset_email", &email, state.limits.reset_email)?;
    sqlx::query("DELETE FROM password_resets WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;
    let user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;
    if let Some(user_id) = user {
        let token = random_token();
        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, expires_at)
             VALUES ($1, $2, now() + make_interval(mins => $3))",
        )
        .bind(hash_token(&token))
        .bind(user_id)
        .bind(RESET_MINUTES)
        .execute(&state.db)
        .await?;
        let body = format!(
            "Someone asked to reset the password for your buildsim account ({email}).\n\n\
             Choose a new password here (the link works once, for {RESET_MINUTES} minutes):\n\
             {base}/#reset={token}\n\n\
             If this wasn't you, ignore this email; your password is unchanged."
        );
        // sent in the background: the response must not wait on (or reveal) SMTP
        tokio::spawn(async move {
            if let Err(e) = mailer
                .send(&email, "Reset your buildsim password", &body)
                .await
            {
                tracing::error!("password-reset email failed: {e}");
            }
        });
    }
    Ok(StatusCode::ACCEPTED)
}

/// `POST /api/auth/password-reset/confirm {token, password}` -> 204. Uses up the token, sets the
/// password and signs the account out everywhere; 400 for an unknown, used or expired token.
pub async fn reset_confirm(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(r): ApiJson<ResetConfirm>,
) -> Result<StatusCode, ApiError> {
    state
        .limiter
        .hit("reset_confirm_ip", &ip, state.limits.reset_confirm_ip)?;
    check_password(&r.password)?;
    let hash = hash_password(r.password).await?;
    let mut tx = state.db.begin().await?;
    let user: Option<Uuid> = sqlx::query_scalar(
        "DELETE FROM password_resets WHERE token_hash = $1 AND expires_at > now()
         RETURNING user_id",
    )
    .bind(hash_token(r.token.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let user = user.ok_or(ApiError::BadRequest(
        "this reset link is invalid or has expired".into(),
    ))?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&hash)
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM password_resets WHERE user_id = $1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/me` -> `{id, email}`.
pub async fn me(auth: AuthUser) -> Json<User> {
    Json(auth.user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_shape() {
        assert_eq!(normalize_email(" A@B.co ").unwrap(), "a@b.co");
        for bad in [
            "", "a", "a@b", "@b.co", "a@@b.co", "a b@c.co", "a@.co", "a@b.", "a@b..co",
        ] {
            assert!(normalize_email(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn token_hash_is_sha256() {
        assert_eq!(hash_token("x").len(), 32);
        assert_ne!(hash_token("x"), hash_token("y"));
    }
}
