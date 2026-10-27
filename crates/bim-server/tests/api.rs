//! End-to-end API tests against a real Postgres. Skipped (with a note) when `DATABASE_URL` is unset.
//! Every test uses fresh random emails, so tests can share one database and run in parallel.

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use bim_server::mail::{Mailer, SendFuture};
use bim_server::ratelimit::{Limit, RateLimits};
use bim_server::{auth::hash_token, connect, migrate, router, Options};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

async fn setup() -> Option<(Router, PgPool)> {
    setup_with(Options::default()).await
}

async fn setup_with(opts: Options) -> Option<(Router, PgPool)> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping bim-server DB test");
        return None;
    };
    let pool = connect(&url).await.expect("connect");
    migrate(&pool).await.expect("migrate");
    Some((router(pool.clone(), &opts), pool))
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_with(app, method, uri, token, body, &[]).await.0
}

/// `call` with extra request headers; also returns the response's Retry-After, if any.
async fn call_with(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> ((StatusCode, Value), Option<String>) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let retry = res
        .headers()
        .get(header::RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("non-JSON body: {}", String::from_utf8_lossy(&bytes)))
    };
    ((status, v), retry)
}

fn email() -> String {
    format!("u{}@example.com", uuid::Uuid::new_v4().simple())
}

fn project_data(name: &str) -> Value {
    serde_json::from_str(&bim_core::Project::new(name, "IN-2026").to_json().unwrap()).unwrap()
}

/// Registers a fresh user; returns (email, token).
async fn user(app: &Router) -> (String, String) {
    let e = email();
    let (s, v) = call(
        app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({"email": e, "password": "correct horse"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (e, v["token"].as_str().unwrap().to_string())
}

async fn new_project(app: &Router, token: &str, name: &str) -> Value {
    let (s, v) = call(
        app,
        Method::POST,
        "/api/projects",
        Some(token),
        Some(json!({"name": name, "data": project_data(name)})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn health() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (s, v) = call(&app, Method::GET, "/api/health", None, None).await;
    assert_eq!((s, v), (StatusCode::OK, json!({"ok": true, "db": true})));
    let (s, v) = call(&app, Method::GET, "/api/nope", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn register_login_logout() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let e = email();
    let creds = json!({"email": e.to_uppercase(), "password": "longenough"});
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(creds.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["user"]["email"], e, "email is normalised to lowercase");
    let id = v["user"]["id"].clone();
    let t1 = v["token"].as_str().unwrap().to_string();
    assert_eq!(t1.len(), 64, "32 random bytes, hex");

    // Duplicate, bad email, short password.
    let (s, _) = call(&app, Method::POST, "/api/auth/register", None, Some(creds)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({"email": "not-an-email", "password": "longenough"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({"email": email(), "password": "short"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Login: wrong password, unknown user, then success.
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": e, "password": "wrongpass"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(v["error"].is_string());
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": email(), "password": "longenough"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": e, "password": "longenough"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["user"]["id"], id);
    let t2 = v["token"].as_str().unwrap().to_string();
    assert_ne!(t1, t2);

    let (s, v) = call(&app, Method::GET, "/api/me", Some(&t2), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v,
        json!({"id": id, "email": e, "email_verified": false, "verification_required": false})
    );

    // Logout revokes only that token.
    let (s, _) = call(&app, Method::POST, "/api/auth/logout", Some(&t2), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&t2), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&t1), None).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn missing_invalid_and_expired_tokens() {
    let Some((app, pool)) = setup().await else {
        return;
    };
    let (s, v) = call(&app, Method::GET, "/api/me", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(v["error"].is_string());
    let (s, _) = call(&app, Method::GET, "/api/projects", Some("deadbeef"), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (_, token) = user(&app).await;
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&token), None).await;
    assert_eq!(s, StatusCode::OK);
    let expires: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token_hash = $1")
            .bind(hash_token(&token))
            .fetch_one(&pool)
            .await
            .unwrap();
    let days = (expires - chrono::Utc::now()).num_days();
    assert!(
        (29..=30).contains(&days),
        "session lasts 30 days, got {days}"
    );

    sqlx::query(
        "UPDATE sessions SET expires_at = now() - interval '1 second' WHERE token_hash = $1",
    )
    .bind(hash_token(&token))
    .execute(&pool)
    .await
    .unwrap();
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&token), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn project_crud() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (_, t) = user(&app).await;
    let (s, v) = call(&app, Method::GET, "/api/projects", Some(&t), None).await;
    assert_eq!((s, v), (StatusCode::OK, json!([])));

    let p = new_project(&app, &t, "Office").await;
    assert_eq!(p["name"], "Office");
    assert_eq!(p["role"], "owner");
    let id = p["id"].as_str().unwrap().to_string();
    let url = format!("/api/projects/{id}");

    let (s, v) = call(&app, Method::GET, &url, Some(&t), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["data"], project_data("Office"));
    assert_eq!(v["updated_at"], p["updated_at"]);

    let (s, v) = call(&app, Method::GET, "/api/projects", Some(&t), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!([p]));

    // Rename only, then replace data with the version we hold.
    let (s, v) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"name": "HQ"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "HQ");
    assert_eq!(v["role"], "owner");
    assert!(v.get("data").is_none());
    assert_ne!(v["updated_at"], p["updated_at"]);
    let (s, v2) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"data": project_data("HQ v2"), "updated_at": v["updated_at"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v2}");
    let (_, full) = call(&app, Method::GET, &url, Some(&t), None).await;
    assert_eq!(full["data"]["name"], "HQ v2");
    assert_eq!(full["name"], "HQ");
    assert_eq!(full["updated_at"], v2["updated_at"]);

    // Bad input.
    let (s, _) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"name": "  "})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        &app,
        Method::GET,
        "/api/projects/not-a-uuid",
        Some(&t),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _) = call(&app, Method::DELETE, &url, Some(&t), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, Method::GET, &url, Some(&t), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stale_updated_at_conflicts() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (_, t) = user(&app).await;
    let p = new_project(&app, &t, "Concurrent").await;
    let url = format!("/api/projects/{}", p["id"].as_str().unwrap());
    let stale = p["updated_at"].clone();

    let (s, _) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"name": "first", "updated_at": stale})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"name": "second", "updated_at": stale})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].is_string());
    let (_, full) = call(&app, Method::GET, &url, Some(&t), None).await;
    assert_eq!(full["name"], "first");

    let (s, _) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"name": "x", "updated_at": "yesterday"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_project_data_is_422() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (_, t) = user(&app).await;
    for data in [
        json!({"name": "no other fields"}),
        json!([1, 2, 3]),
        json!("text"),
        {
            let mut d = project_data("future");
            d["schema_version"] = json!(999);
            d
        },
    ] {
        let (s, v) = call(
            &app,
            Method::POST,
            "/api/projects",
            Some(&t),
            Some(json!({"name": "Bad", "data": data})),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert!(v["error"].is_string());
    }
    let p = new_project(&app, &t, "Good").await;
    let url = format!("/api/projects/{}", p["id"].as_str().unwrap());
    let (s, _) = call(
        &app,
        Method::PUT,
        &url,
        Some(&t),
        Some(json!({"data": {"elements": "nope"}})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn sharing_roles() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (_, owner) = user(&app).await;
    let (ed_email, editor) = user(&app).await;
    let (vw_email, viewer) = user(&app).await;
    let (_, stranger) = user(&app).await;
    let p = new_project(&app, &owner, "Shared").await;
    let id = p["id"].as_str().unwrap();
    let url = format!("/api/projects/{id}");
    let shares = format!("{url}/shares");

    for (e, role) in [(&ed_email, "editor"), (&vw_email, "viewer")] {
        let (s, v) = call(
            &app,
            Method::PUT,
            &shares,
            Some(&owner),
            Some(json!({"email": e, "role": role})),
        )
        .await;
        assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    }
    let (s, v) = call(&app, Method::GET, &shares, Some(&owner), None).await;
    assert_eq!(s, StatusCode::OK);
    let mut want = vec![
        json!({"email": ed_email, "role": "editor"}),
        json!({"email": vw_email, "role": "viewer"}),
    ];
    want.sort_by_key(|v| v["email"].as_str().unwrap().to_string());
    assert_eq!(v, Value::Array(want));

    // Unknown user, bad role, sharing with self.
    let (s, _) = call(
        &app,
        Method::PUT,
        &shares,
        Some(&owner),
        Some(json!({"email": email(), "role": "viewer"})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        Method::PUT,
        &shares,
        Some(&owner),
        Some(json!({"email": ed_email, "role": "owner"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Shared projects appear in the list with the right role.
    for (t, role) in [(&editor, "editor"), (&viewer, "viewer")] {
        let (_, v) = call(&app, Method::GET, "/api/projects", Some(t), None).await;
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["id"], id);
        assert_eq!(v[0]["role"], role);
        let (s, v) = call(&app, Method::GET, &url, Some(t), None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["role"], role);
    }

    // Viewer can read but not write; editor can write but not manage.
    let rename = |n: &str| Some(json!({ "name": n }));
    let (s, v) = call(&app, Method::PUT, &url, Some(&viewer), rename("by viewer")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(v["error"].is_string());
    let (s, v) = call(&app, Method::PUT, &url, Some(&editor), rename("by editor")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["role"], "editor");
    for t in [&editor, &viewer] {
        let (s, _) = call(&app, Method::DELETE, &url, Some(t), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _) = call(&app, Method::GET, &shares, Some(t), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    // A stranger learns nothing: every route is 404.
    let (_, v) = call(&app, Method::GET, "/api/projects", Some(&stranger), None).await;
    assert_eq!(v, json!([]));
    let (s, _) = call(&app, Method::GET, &url, Some(&stranger), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, Method::PUT, &url, Some(&stranger), rename("x")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, Method::DELETE, &url, Some(&stranger), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, Method::GET, &shares, Some(&stranger), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Downgrade the editor, then revoke the viewer (email is percent-encoded in the path).
    let (s, _) = call(
        &app,
        Method::PUT,
        &shares,
        Some(&owner),
        Some(json!({"email": ed_email, "role": "viewer"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, Method::PUT, &url, Some(&editor), rename("again")).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let del = format!("{shares}/{}", vw_email.replace('@', "%40"));
    let (s, _) = call(&app, Method::DELETE, &del, Some(&owner), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, Method::GET, &url, Some(&viewer), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = call(&app, Method::GET, &shares, Some(&owner), None).await;
    assert_eq!(v, json!([{"email": ed_email, "role": "viewer"}]));
}

#[tokio::test]
async fn malformed_body_is_json_error() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn oversized_body_is_413() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let pad = "x".repeat(bim_server::BODY_LIMIT + 1);
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": pad, "password": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(v["error"].is_string());
}

/// Records outgoing mail instead of sending it.
#[derive(Debug, Default)]
struct Outbox(Mutex<Vec<(String, String)>>);

impl Mailer for Outbox {
    fn send<'a>(&'a self, to: &'a str, _subject: &'a str, body: &'a str) -> SendFuture<'a> {
        self.0.lock().unwrap().push((to.into(), body.into()));
        Box::pin(async { Ok(()) })
    }
}

fn reset_opts(outbox: &Arc<Outbox>) -> Options {
    Options {
        mailer: Some(outbox.clone() as Arc<dyn Mailer>),
        public_url: Some("https://app.example".into()),
        ..Options::default()
    }
}

/// Waits for the background send, then returns the token from the mailed link to `to`.
async fn mailed_token(outbox: &Outbox, to: &str) -> String {
    mailed_link(outbox, to, "reset").await
}

/// Waits for the background send, then returns the token from the newest `#<kind>=` link to `to`.
async fn mailed_link(outbox: &Outbox, to: &str, kind: &str) -> String {
    let prefix = format!("https://app.example/#{kind}=");
    for _ in 0..100 {
        let link = outbox
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .filter(|(t, _)| t == to)
            .find_map(|(_, b)| {
                let at = b.find(&prefix)?;
                Some(
                    b[at + prefix.len()..]
                        .split_whitespace()
                        .next()?
                        .to_string(),
                )
            });
        if let Some(t) = link {
            return t;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("no {kind} email to {to}");
}

async fn login(app: &Router, email: &str, password: &str) -> StatusCode {
    call(
        app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await
    .0
}

#[tokio::test]
async fn password_reset_flow() {
    let outbox = Arc::new(Outbox::default());
    let Some((app, pool)) = setup_with(reset_opts(&outbox)).await else {
        return;
    };
    let (e, old_session) = user(&app).await;
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(json!({"email": e.to_uppercase()})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let token = mailed_token(&outbox, &e).await;
    // only the hash is stored
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM password_resets WHERE token_hash = $1")
            .bind(hash_token(&token))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, 1);

    let confirm = |t: &str, p: &str| {
        let body = json!({"token": t, "password": p});
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/api/auth/password-reset/confirm",
                None,
                Some(body),
            )
            .await
        }
    };
    let (s, v) = confirm(&token, "short").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = confirm(&token, "brand new pass").await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    // single use
    let (s, v) = confirm(&token, "another new pass").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"]
        .as_str()
        .unwrap()
        .contains("invalid or has expired"));
    // signed out everywhere; old password gone, new one works
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&old_session), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(
        login(&app, &e, "correct horse").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(login(&app, &e, "brand new pass").await, StatusCode::OK);
}

#[tokio::test]
async fn password_reset_edge_cases() {
    let outbox = Arc::new(Outbox::default());
    let Some((app, pool)) = setup_with(reset_opts(&outbox)).await else {
        return;
    };
    // unknown account: same 202, nothing sent
    let ghost = email();
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(json!({"email": ghost})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(outbox.0.lock().unwrap().iter().all(|(t, _)| *t != ghost));
    // invalid email shape
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(json!({"email": "nope"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // expired link
    let (e, _) = user(&app).await;
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(json!({"email": e})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let token = mailed_token(&outbox, &e).await;
    sqlx::query(
        "UPDATE password_resets SET expires_at = now() - interval '1 second' WHERE token_hash = $1",
    )
    .bind(hash_token(&token))
    .execute(&pool)
    .await
    .unwrap();
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset/confirm",
        None,
        Some(json!({"token": token, "password": "brand new pass"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(login(&app, &e, "correct horse").await, StatusCode::OK);
}

#[tokio::test]
async fn password_reset_unconfigured_is_503() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(json!({"email": email()})),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(v["error"].is_string());
}

fn tight_limits() -> RateLimits {
    RateLimits {
        login_ip: Limit::per(4, 60),
        login_email_failures: Limit::per(2, 60),
        register_ip: Limit::per(3, 60),
        reset_ip: Limit::per(5, 60),
        reset_email: Limit::per(1, 60),
        reset_confirm_ip: Limit::per(2, 60),
    }
}

#[tokio::test]
async fn login_rate_limits() {
    let Some((app, _)) = setup_with(Options {
        limits: tight_limits(),
        ..Options::default()
    })
    .await
    else {
        return;
    };
    let (e, _) = user(&app).await;
    // failures per email: 2 allowed, then 429 even with the right password
    assert_eq!(
        login(&app, &e, "wrong pass 1").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&app, &e, "wrong pass 2").await,
        StatusCode::UNAUTHORIZED
    );
    let ((s, v), retry) = call_with(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": e, "password": "correct horse"})),
        &[],
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(v["error"].is_string());
    assert!(retry.unwrap().parse::<u64>().unwrap() >= 1);
    // per IP: 4 attempts per window in total, whatever the email (3 used above)
    for expect in [StatusCode::UNAUTHORIZED, StatusCode::TOO_MANY_REQUESTS] {
        let (s, _) = call(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": email(), "password": "x"})),
        )
        .await;
        assert_eq!(s, expect);
    }
}

#[tokio::test]
async fn register_and_reset_rate_limits() {
    let outbox = Arc::new(Outbox::default());
    let opts = Options {
        limits: tight_limits(),
        ..reset_opts(&outbox)
    };
    let Some((app, _)) = setup_with(opts).await else {
        return;
    };
    let (e, _) = user(&app).await;
    user(&app).await;
    user(&app).await;
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({"email": email(), "password": "correct horse"})),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    // one reset email per address per window
    let req = || json!({"email": e});
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(req()),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(req()),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    // confirm attempts per IP
    for expect in [
        StatusCode::BAD_REQUEST,
        StatusCode::BAD_REQUEST,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let (s, _) = call(
            &app,
            Method::POST,
            "/api/auth/password-reset/confirm",
            None,
            Some(json!({"token": "guess", "password": "brand new pass"})),
        )
        .await;
        assert_eq!(s, expect);
    }
}

#[tokio::test]
async fn client_ip_from_forwarded_for_only_when_trusted() {
    let limits = RateLimits {
        register_ip: Limit::per(1, 60),
        ..tight_limits()
    };
    let register = |app: Router, xff: &'static str| async move {
        call_with(
            &app,
            Method::POST,
            "/api/auth/register",
            None,
            Some(json!({"email": email(), "password": "correct horse"})),
            &[("x-forwarded-for", xff)],
        )
        .await
        .0
         .0
    };
    // trusted: the rightmost (proxy-appended) address is the key; a spoofed left part is ignored
    let Some((app, _)) = setup_with(Options {
        limits,
        trust_proxy: true,
        ..Options::default()
    })
    .await
    else {
        return;
    };
    assert_eq!(
        register(app.clone(), "6.6.6.6, 1.1.1.1").await,
        StatusCode::CREATED
    );
    assert_eq!(
        register(app.clone(), "7.7.7.7, 1.1.1.1").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(register(app.clone(), "2.2.2.2").await, StatusCode::CREATED);
    // untrusted: the header is ignored, so changing it does not dodge the limit
    let (app, _) = setup_with(Options {
        limits,
        ..Options::default()
    })
    .await
    .unwrap();
    assert_eq!(register(app.clone(), "1.1.1.1").await, StatusCode::CREATED);
    assert_eq!(
        register(app.clone(), "2.2.2.2").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

async fn share(app: &Router, token: &str, project: &str, email: &str) -> (StatusCode, Value) {
    call(
        app,
        Method::PUT,
        &format!("/api/projects/{project}/shares"),
        Some(token),
        Some(json!({"email": email, "role": "viewer"})),
    )
    .await
}

#[tokio::test]
async fn email_verification_gates_sharing() {
    let outbox = Arc::new(Outbox::default());
    let Some((app, _)) = setup_with(reset_opts(&outbox)).await else {
        return;
    };
    let (_, owner) = user(&app).await;
    let (e, t) = user(&app).await;
    let (s, me) = call(&app, Method::GET, "/api/me", Some(&t), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["email_verified"], json!(false));
    assert_eq!(me["verification_required"], json!(true));
    let p = new_project(&app, &owner, "Gated").await;
    let pid = p["id"].as_str().unwrap();
    // an unproven address cannot receive a share
    let (s, v) = share(&app, &owner, pid, &e).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert!(v["error"].as_str().unwrap().contains("not verified"));

    let token = mailed_link(&outbox, &e, "verify").await;
    let verify = |tok: String| {
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                "/api/auth/verify",
                None,
                Some(json!({"token": tok})),
            )
            .await
        }
    };
    let (s, _) = verify("nope".into()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = verify(token.clone()).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = verify(token).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "single use");
    let (_, me) = call(&app, Method::GET, "/api/me", Some(&t), None).await;
    assert_eq!(me["email_verified"], json!(true));
    let (s, _) = share(&app, &owner, pid, &e).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // nothing left to resend
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/verify/resend",
        Some(&t),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    // login reports the flag too
    let (_, v) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": e, "password": "correct horse"})),
    )
    .await;
    assert_eq!(v["user"]["email_verified"], json!(true));
}

#[tokio::test]
async fn verification_resend_and_reset_also_verify() {
    let outbox = Arc::new(Outbox::default());
    let opts = Options {
        limits: tight_limits(),
        ..reset_opts(&outbox)
    };
    let Some((app, _)) = setup_with(opts).await else {
        return;
    };
    let (e, t) = user(&app).await;
    mailed_link(&outbox, &e, "verify").await;
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/verify/resend",
        Some(&t),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/verify/resend",
        Some(&t),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "one resend per window");
    let verify_mails = || {
        outbox
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(to, b)| *to == e && b.contains("#verify="))
            .count()
    };
    for _ in 0..50 {
        if verify_mails() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(verify_mails(), 2);
    // a completed password reset proves the address as well
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset",
        None,
        Some(json!({"email": e})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let token = mailed_token(&outbox, &e).await;
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/password-reset/confirm",
        None,
        Some(json!({"token": token, "password": "brand new pass"})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": e, "password": "brand new pass"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["user"]["email_verified"], json!(true));
}

#[tokio::test]
async fn verification_not_enforced_without_email() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (_, owner) = user(&app).await;
    let (e, t) = user(&app).await;
    let (_, me) = call(&app, Method::GET, "/api/me", Some(&t), None).await;
    assert_eq!(me["verification_required"], json!(false));
    let p = new_project(&app, &owner, "Open").await;
    let (s, _) = share(&app, &owner, p["id"].as_str().unwrap(), &e).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/verify/resend",
        Some(&t),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn export_contains_own_and_shared_data_only() {
    let Some((app, _)) = setup().await else {
        return;
    };
    let (owner_email, owner) = user(&app).await;
    let (friend_email, friend) = user(&app).await;
    let p = new_project(&app, &owner, "Exported").await;
    let pid = p["id"].as_str().unwrap().to_string();
    assert_eq!(
        share(&app, &owner, &pid, &friend_email).await.0,
        StatusCode::NO_CONTENT
    );

    let req = Request::builder()
        .uri("/api/me/export")
        .header(header::AUTHORIZATION, format!("Bearer {owner}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cd = res.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cd.starts_with("attachment; filename=\"buildsim-export-"),
        "{cd}"
    );
    let raw = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(raw.to_vec()).unwrap();
    assert!(
        !text.contains("password") && !text.contains("argon2"),
        "no secrets in export"
    );
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["format"], "buildsim-account-export/1");
    assert_eq!(v["account"]["email"], json!(owner_email));
    assert_eq!(v["projects"].as_array().unwrap().len(), 1);
    assert_eq!(v["projects"][0]["id"], json!(pid));
    assert_eq!(v["projects"][0]["data"]["name"], json!("Exported"));
    assert_eq!(
        v["projects"][0]["shared_with"],
        json!([{"email": friend_email, "role": "viewer"}])
    );
    assert_eq!(v["shared_with_me"], json!([]));

    let (s, fv) = call(&app, Method::GET, "/api/me/export", Some(&friend), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(fv["projects"], json!([]));
    assert_eq!(fv["shared_with_me"][0]["owner_email"], json!(owner_email));
    assert_eq!(fv["shared_with_me"][0]["role"], json!("viewer"));
    let (s, _) = call(&app, Method::GET, "/api/me/export", None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_account_needs_password_and_removes_everything() {
    let Some((app, pool)) = setup().await else {
        return;
    };
    let (e, t) = user(&app).await;
    let (friend_email, friend) = user(&app).await;
    let p = new_project(&app, &t, "Doomed").await;
    let pid = p["id"].as_str().unwrap().to_string();
    assert_eq!(
        share(&app, &t, &pid, &friend_email).await.0,
        StatusCode::NO_CONTENT
    );
    let del = |pw: &'static str| {
        let (app, t) = (app.clone(), t.clone());
        async move {
            call(
                &app,
                Method::DELETE,
                "/api/me",
                Some(&t),
                Some(json!({"password": pw})),
            )
            .await
        }
    };
    let (s, v) = del("wrong password").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&t), None).await;
    assert_eq!(s, StatusCode::OK, "still signed in after a wrong password");
    let (s, _) = del("correct horse").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = call(&app, Method::GET, "/api/me", Some(&t), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"email": e, "password": "correct horse"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // owned projects go too, also for the people they were shared with
    let (s, _) = call(
        &app,
        Method::GET,
        &format!("/api/projects/{pid}"),
        Some(&friend),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE id = $1::uuid")
        .bind(&pid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    // the address is free again
    let (s, _) = call(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({"email": e, "password": "correct horse"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
}

/// Needs no database: the pool points at a port nothing listens on.
#[tokio::test]
async fn health_is_503_when_the_database_is_down() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .unwrap();
    let app = router(pool, &Options::default());
    let (s, v) = call(&app, Method::GET, "/api/health", None, None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v, json!({"ok": false, "db": false}));
}
