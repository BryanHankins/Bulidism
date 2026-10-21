//! buildsim backend: accounts, project storage and sharing over a JSON API (Axum + Postgres).
//!
//! All routes live under `/api`; see `router`. The binary (`main.rs`) reads its config from env.

pub mod auth;
pub mod db;
pub mod error;
pub mod projects;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

pub use db::{connect, migrate};

/// Largest accepted request body (project JSON can be large).
pub const BODY_LIMIT: usize = 20 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

/// Runtime options beyond the database.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Allow cross-origin requests from exactly this origin.
    pub cors_origin: Option<String>,
    /// Serve this directory (the static web app) for non-`/api` paths.
    pub web_dir: Option<PathBuf>,
}

impl Options {
    /// Reads `CORS_ORIGIN` and `WEB_DIR`; empty values count as unset.
    pub fn from_env() -> Self {
        let var = |k| {
            std::env::var(k)
                .ok()
                .filter(|v: &String| !v.trim().is_empty())
        };
        Options {
            cors_origin: var("CORS_ORIGIN"),
            web_dir: var("WEB_DIR").map(PathBuf::from),
        }
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
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
        .route("/me", get(auth::me))
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
        .with_state(AppState { db });

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
