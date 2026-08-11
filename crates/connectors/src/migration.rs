use sqlx::SqlitePool;

use crate::Result;

pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await?;
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
