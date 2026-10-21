//! Connection pool and embedded, versioned SQL migrations (no live DB needed to build).

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Connection;

/// Ordered migrations; a version is applied once and recorded in `schema_migrations`.
const MIGRATIONS: &[(i32, &str)] = &[(1, include_str!("../migrations/0001_init.sql"))];

/// Arbitrary key for the advisory lock that serialises concurrent migrators.
const LOCK_KEY: i64 = 0x0062_696d_5f73_7276;

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new().max_connections(10).connect(url).await
}

/// Applies pending migrations, each in its own transaction.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    let result = async {
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version int PRIMARY KEY,
                applied_at timestamptz NOT NULL DEFAULT now())",
        )
        .execute(&mut *conn)
        .await?;
        for &(version, sql) in MIGRATIONS {
            let done: Option<i32> =
                sqlx::query_scalar("SELECT version FROM schema_migrations WHERE version = $1")
                    .bind(version)
                    .fetch_optional(&mut *conn)
                    .await?;
            if done.is_some() {
                continue;
            }
            let mut tx = conn.begin().await?;
            sqlx::raw_sql(sql).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO schema_migrations (version) VALUES ($1)")
                .bind(version)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            tracing::info!("applied migration {version}");
        }
        Ok::<_, sqlx::Error>(())
    }
    .await;
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    result
}
