//! API error type: every failure becomes `{"error": "..."}` with a fitting status.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    /// 400: malformed or invalid input.
    BadRequest(String),
    /// 401: missing, invalid or expired token, or bad credentials.
    Unauthorized(&'static str),
    /// 403: the caller can see the project but their role is too low.
    Forbidden,
    /// 404: no such resource, or no access to it at all.
    NotFound(&'static str),
    /// 409: duplicate email or stale `updated_at`.
    Conflict(&'static str),
    /// 422: project data failed `bim_core::Project` validation.
    Unprocessable(String),
    /// 429: rate limited; seconds until the caller may retry (sent as `Retry-After`).
    TooManyRequests(u64),
    /// 503: a feature this deployment has not configured (e.g. email for password reset).
    Unavailable(&'static str),
    /// Body rejected by the JSON extractor (status taken from the rejection).
    Body(StatusCode, String),
    /// 500: logged, never echoed to the client.
    Internal(String),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(format!("database: {e}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.into()),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "insufficient role".into()),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m.into()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m.into()),
            ApiError::Unprocessable(m) => (StatusCode::UNPROCESSABLE_ENTITY, m),
            ApiError::TooManyRequests(secs) => {
                let body = Json(json!({ "error": "too many attempts; try again later" }));
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, secs.to_string())],
                    body,
                )
                    .into_response();
            }
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m.into()),
            ApiError::Body(s, m) => (s, m),
            ApiError::Internal(m) => {
                tracing::error!("{m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

/// `Json<T>` whose rejections use the `{error}` body instead of plain text.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(r) => Err(rejection(r)),
        }
    }
}

fn rejection(r: JsonRejection) -> ApiError {
    ApiError::Body(r.status(), r.body_text())
}
