//! In-process fixed-window rate limits for the auth endpoints, keyed by client IP or email.
//!
//! State lives in this process only; that is enough for a single instance. Behind a reverse
//! proxy set `TRUST_PROXY` so the client IP comes from `X-Forwarded-For` (rightmost entry, the
//! one the proxy itself appended), otherwise every request looks like it came from the proxy.

use crate::error::ApiError;
use crate::AppState;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// At most `max` events per `window`.
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub max: u32,
    pub window: Duration,
}

impl Limit {
    pub const fn per(max: u32, secs: u64) -> Self {
        Limit {
            max,
            window: Duration::from_secs(secs),
        }
    }
}

/// Limits per endpoint. Login counts every attempt per IP but only failures per email, so a
/// stranger cannot lock a user out faster than they could guess.
#[derive(Debug, Clone, Copy)]
pub struct RateLimits {
    pub login_ip: Limit,
    pub login_email_failures: Limit,
    pub register_ip: Limit,
    pub reset_ip: Limit,
    pub reset_email: Limit,
    pub reset_confirm_ip: Limit,
}

impl Default for RateLimits {
    fn default() -> Self {
        RateLimits {
            login_ip: Limit::per(20, 5 * 60),
            login_email_failures: Limit::per(10, 15 * 60),
            register_ip: Limit::per(10, 60 * 60),
            reset_ip: Limit::per(10, 60 * 60),
            reset_email: Limit::per(3, 60 * 60),
            reset_confirm_ip: Limit::per(20, 15 * 60),
        }
    }
}

/// Entries beyond this trigger a sweep of expired windows.
const SWEEP_AT: usize = 10_000;

/// (scope, key) -> (window end, events in window)
type Windows = HashMap<(&'static str, String), (Instant, u32)>;

#[derive(Default)]
pub struct Limiter {
    windows: Mutex<Windows>,
}

impl Limiter {
    /// Records one event; 429 (with seconds until the window resets) once `limit` is reached.
    pub fn hit(&self, scope: &'static str, key: &str, limit: Limit) -> Result<(), ApiError> {
        self.apply(scope, key, limit, true)
    }

    /// Like `hit` but records nothing: is `key` already over `limit`?
    pub fn check(&self, scope: &'static str, key: &str, limit: Limit) -> Result<(), ApiError> {
        self.apply(scope, key, limit, false)
    }

    fn apply(
        &self,
        scope: &'static str,
        key: &str,
        limit: Limit,
        record: bool,
    ) -> Result<(), ApiError> {
        let now = Instant::now();
        let mut w = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if w.len() > SWEEP_AT {
            w.retain(|_, (end, _)| *end > now);
        }
        let entry = w
            .entry((scope, key.to_owned()))
            .or_insert((now + limit.window, 0));
        if entry.0 <= now {
            *entry = (now + limit.window, 0);
        }
        if entry.1 >= limit.max {
            let secs = entry.0.saturating_duration_since(now).as_secs().max(1);
            return Err(ApiError::TooManyRequests(secs));
        }
        if record {
            entry.1 += 1;
        }
        Ok(())
    }
}

/// The caller's address as used for rate limiting ("unknown" when it cannot be determined).
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let forwarded = state
            .trust_proxy
            .then(|| parts.headers.get("x-forwarded-for")?.to_str().ok())
            .flatten()
            .and_then(|v| v.rsplit(',').map(str::trim).find(|s| !s.is_empty()));
        let ip = match forwarded {
            Some(ip) => ip.to_owned(),
            None => parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip().to_string())
                .unwrap_or_else(|| "unknown".into()),
        };
        Ok(ClientIp(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_and_keys_are_independent() {
        let l = Limiter::default();
        let lim = Limit::per(2, 60);
        assert!(l.hit("s", "a", lim).is_ok());
        assert!(l.hit("s", "a", lim).is_ok());
        assert!(matches!(l.hit("s", "a", lim), Err(ApiError::TooManyRequests(s)) if s >= 1));
        assert!(l.hit("s", "b", lim).is_ok());
        assert!(l.hit("t", "a", lim).is_ok());
    }

    #[test]
    fn check_does_not_record_and_window_resets() {
        let l = Limiter::default();
        let lim = Limit {
            max: 1,
            window: Duration::from_millis(20),
        };
        for _ in 0..5 {
            assert!(l.check("s", "a", lim).is_ok());
        }
        assert!(l.hit("s", "a", lim).is_ok());
        assert!(l.check("s", "a", lim).is_err());
        std::thread::sleep(Duration::from_millis(30));
        assert!(l.hit("s", "a", lim).is_ok());
    }
}
