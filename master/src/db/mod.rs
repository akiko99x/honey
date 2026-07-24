//! database layer: connection pool, migrations, models and a small repo.
use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub mod models;
pub mod repo;

/// opens a connection pool to Postgres.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
        .context("connect to postgres")
}

/// applies pending migrations from ./migrations (embedded at build time).
pub async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("run migrations")
}
