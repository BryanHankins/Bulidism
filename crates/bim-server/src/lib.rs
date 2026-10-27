//! buildsim backend: accounts, project storage and sharing over a JSON API (Axum + Postgres).
//!
//! All routes live under `/api`; see `router`. The binary (`main.rs`) reads its config from env.

pub mod account;
pub mod auth;
pub mod db;
pub mod error;
pub mod mail;
pub mod projects;
pub mod ratelimit;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

pub use db::{connect, migrate};

/// Largest accepted request body (project JSON can be large).
pub const BODY_LIMIT: usize = 20 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub limiter: Arc<ratelimit::Limiter>,
    pub limits: ratelimit::RateLimits,
    pub trust_proxy: bool,
    pub mailer: Option<Arc<dyn mail::Mailer>>,
    pub public_url: Option<String>,
}

/// Runtime options beyond the database.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Allow cross-origin requests from exactly this origin.
    pub cors_origin: Option<String>,
    /// Serve this directory (the static web app) for non-`/api` paths.
    pub web_dir: Option<PathBuf>,
    /// Take the client IP from `X-Forwarded-For` (only behind a proxy that sets it).
    pub trust_proxy: bool,
    /// Base URL of the web app, for links in emails (no trailing slash).
    pub public_url: Option<String>,
    /// Sends password-reset email; reset is disabled (503) without it or `public_url`.
    pub mailer: Option<Arc<dyn mail::Mailer>>,
    pub limits: ratelimit::RateLimits,
}

impl Options {
    /// Reads `CORS_ORIGIN`, `WEB_DIR`, `TRUST_PROXY` (1/true), `PUBLIC_URL` (else Render's
    /// `RENDER_EXTERNAL_URL`) and `SMTP_URL` + `MAIL_FROM`; empty values count as unset.
    pub fn from_env() -> Self {
        let var = |k| {
            std::env::var(k)
                .ok()
                .filter(|v: &String| !v.trim().is_empty())
        };
        Options {
            cors_origin: var("CORS_ORIGIN"),
            web_dir: var("WEB_DIR").map(PathBuf::from),
            trust_proxy: var("TRUST_PROXY")
                .is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes")),
            public_url: var("PUBLIC_URL")
                .or_else(|| var("RENDER_EXTERNAL_URL"))
                .map(|u| u.trim().trim_end_matches('/').to_owned()),
            mailer: mail::SmtpMailer::from_env().map(|m| Arc::new(m) as Arc<dyn mail::Mailer>),
            limits: ratelimit::RateLimits::default(),
        }
    }
}

/// `GET /api/health` -> 200 `{ok: true, db: true}`, or 503 `{ok: false, db: false}` when the
/// database does not answer within 3 s (what Render's health check and the uptime workflow see).
async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let ping = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.db);
    match tokio::time::timeout(std::time::Duration::from_secs(3), ping).await {
        Ok(Ok(_)) => (StatusCode::OK, Json(json!({ "ok": true, "db": true }))),
        Ok(Err(e)) => {
            tracing::error!("health check: database error: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false, "db": false })),
            )
        }
        Err(_) => {
            tracing::error!("health check: database timed out");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false, "db": false })),
            )
        }
    }
}

async fn api_not_found() -> error::ApiError {
    error::ApiError::NotFound("not found")
}

/// The full application: `/api` routes, optional CORS and static file serving.
pub fn router(db: PgPool, opts: &Options) -> Router {
    use projects as p;
    let api = Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/password-reset", post(auth::reset_request))
        .route("/auth/password-reset/confirm", post(auth::reset_confirm))
        .route("/auth/verify", post(account::verify))
        .route("/auth/verify/resend", post(account::resend))
        .route("/me", get(auth::me).delete(account::delete_account))
        .route("/me/export", get(account::export))
        .route("/projects", get(p::list).post(p::create))
        .route(
            "/projects/{id}",
            get(p::get).put(p::update).delete(p::delete),
        )
        .route(
            "/projects/{id}/shares",
            get(p::list_shares).put(p::put_share),
        )
        .route("/projects/{id}/shares/{email}", delete(p::delete_share))
        .fallback(api_not_found)
        .with_state(AppState {
            db,
            limiter: Arc::default(),
            limits: opts.limits,
            trust_proxy: opts.trust_proxy,
            mailer: opts.mailer.clone(),
            public_url: opts.public_url.clone(),
        });

    let mut app = Router::new().nest("/api", api);
    if let Some(dir) = &opts.web_dir {
        app = app.fallback_service(ServeDir::new(dir));
    }
    let mut app = app
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(TraceLayer::new_for_http());
    if let Some(origin) = &opts.cors_origin {
        match HeaderValue::from_str(origin) {
            Ok(origin) => {
                app = app.layer(
                    CorsLayer::new()
                        .allow_origin(origin)
                        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                        .max_age(std::time::Duration::from_secs(3600)),
                );
            }
            Err(_) => tracing::warn!("ignoring invalid CORS_ORIGIN {origin:?}"),
        }
    }
    app
}
