//! SQLite connection helpers.
//!
//! Migrations are embedded with [`sqlx::migrate!`] at compile time from
//! `crates/core/migrations/`. This keeps releases self-contained — the
//! binary never has to ship a separate SQL file.

use sqlx::SqlitePool;

use crate::error::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Apply any pending migrations against `pool`.
pub(crate) async fn migrate(pool: &SqlitePool) -> Result<()> {
    MIGRATOR.run(pool).await?;
    Ok(())
}
