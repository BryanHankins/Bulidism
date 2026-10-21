//! End-to-end API tests against a real Postgres. Skipped (with a note) when `DATABASE_URL` is unset.
//! Every test uses fresh random emails, so tests can share one database and run in parallel.

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use bim_server::{auth::hash_token, connect, migrate, router, Options};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

async fn setup() -> Option<(Router, PgPool)> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set; skipping bim-server DB test");
        return None;
    };
    let pool = connect(&url).await.expect("connect");
    migrate(&pool).await.expect("migrate");
    Some((router(pool.clone(), &Options::default()), pool))
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
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
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("non-JSON body: {}", String::from_utf8_lossy(&bytes)))
    };
    (status, v)
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
    assert_eq!((s, v), (StatusCode::OK, json!({"ok": true})));
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
    assert_eq!(v, json!({"id": id, "email": e}));

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
