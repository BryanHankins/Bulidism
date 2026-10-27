//! `bim-server`: env `DATABASE_URL` (required), `BIND_ADDR` (else 0.0.0.0:$PORT, else 0.0.0.0:8081),
//! `CORS_ORIGIN`, `WEB_DIR`, `TRUST_PROXY`, `PUBLIC_URL`, `SMTP_URL` + `MAIL_FROM` (all optional; see
//! `Options::from_env`), `SENTRY_DSN` (optional error reporting). Applies migrations at startup.

use bim_server::{connect, migrate, router, Options};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

fn main() {
    // before the runtime starts, so panics on every thread are reported
    let sentry = init_sentry();
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn")),
        )
        .with(tracing_subscriber::fmt::layer())
        // error! events become Sentry issues, info!/warn! their breadcrumbs (no-op without a DSN)
        .with(sentry::integrations::tracing::layer())
        .init();
    match &sentry {
        Ok(Some(_)) => tracing::info!("error reporting: Sentry on"),
        Ok(None) => tracing::info!("error reporting: off (set SENTRY_DSN to enable)"),
        Err(e) => tracing::warn!("error reporting: off, {e}"),
    }
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(run());
    if let Err(e) = result {
        tracing::error!("{e}");
        drop(sentry); // flush the error before exiting
        std::process::exit(1);
    }
}

/// Sentry (or any Sentry-compatible service, e.g. self-hosted GlitchTip) when `SENTRY_DSN` is set.
/// No personal data is attached; the release is the deployed commit when Render provides it.
fn init_sentry() -> Result<Option<sentry::ClientInitGuard>, String> {
    let var = |k| {
        std::env::var(k)
            .ok()
            .filter(|v: &String| !v.trim().is_empty())
    };
    let Some(dsn) = var("SENTRY_DSN") else {
        return Ok(None);
    };
    let dsn: sentry::types::Dsn = dsn
        .trim()
        .parse()
        .map_err(|e| format!("bad SENTRY_DSN: {e}"))?;
    let release = var("RENDER_GIT_COMMIT")
        .map(|c| format!("bim-server@{}", &c[..c.len().min(12)]).into())
        .or_else(|| sentry::release_name!());
    let environment = var("SENTRY_ENVIRONMENT")
        .or_else(|| var("RENDER").map(|_| "render".into()))
        .map(Into::into);
    Ok(Some(sentry::init(sentry::ClientOptions {
        dsn: Some(dsn),
        release,
        environment,
        send_default_pii: false,
        attach_stacktrace: true,
        ..Default::default()
    })))
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set")?;
    // PORT is what Render, Railway and similar hosts inject.
    let bind = std::env::var("BIND_ADDR")
        .or_else(|_| std::env::var("PORT").map(|p| format!("0.0.0.0:{p}")))
        .unwrap_or_else(|_| "0.0.0.0:8081".into());
    let opts = Options::from_env();
    let pool = connect(&url).await?;
    migrate(&pool).await?;
    let app = router(pool, &opts);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(
        "listening on {bind} (cors: {:?}, web: {:?}, trust proxy: {}, password reset: {})",
        opts.cors_origin,
        opts.web_dir,
        opts.trust_proxy,
        if opts.mailer.is_some() && opts.public_url.is_some() {
            "on"
        } else {
            "off"
        }
    );
    // ConnectInfo gives the rate limiter the peer address when there is no trusted proxy
    let app = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
