//! `bim-server`: env `DATABASE_URL` (required), `BIND_ADDR` (default 0.0.0.0:8081),
//! `CORS_ORIGIN`, `WEB_DIR` (both optional). Applies migrations at startup.

use bim_server::{connect, migrate, router, Options};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn")),
        )
        .init();
    if let Err(e) = run().await {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set")?;
    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".into());
    let opts = Options::from_env();
    let pool = connect(&url).await?;
    migrate(&pool).await?;
    let app = router(pool, &opts);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(
        "listening on {bind} (cors: {:?}, web: {:?})",
        opts.cors_origin,
        opts.web_dir
    );
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
