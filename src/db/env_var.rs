use super::*;
use crate::models::EnvVar;

/// Save environment variable (replaces existing value if key exists)
#[instrument(skip(pool))]
pub async fn save(pool: &Pool<Sqlite>, env_var: &EnvVar) -> Result<()> {
    // Delete existing env var with the same app_id and key if it exists
    sqlx::query!(
        r#"DELETE FROM env_var WHERE app_id = ? AND key = ?"#,
        env_var.app_id,
        env_var.key
    )
    .execute(pool)
    .await?;

    // Insert the new env var
    sqlx::query!(
        r#"
        INSERT INTO env_var (id, app_id, key, value, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
        env_var.id,
        env_var.app_id,
        env_var.key,
        env_var.value,
        env_var.created_at,
        env_var.updated_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get all environment variables for an app
#[instrument(skip(pool))]
pub async fn get_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<Vec<EnvVar>> {
    let records = sqlx::query_as!(
        EnvVar,
        r#"
        SELECT id, app_id, key, value, created_at as "created_at: _", updated_at as "updated_at: _"
        FROM env_var
        WHERE app_id = ?
        "#,
        app_id
    )
    .fetch_all(pool)
    .await?;
    Ok(records)
}

/// Delete a specific environment variable
#[instrument(skip(pool))]
pub async fn delete_by_key(pool: &Pool<Sqlite>, app_id: &str, key: &str) -> Result<()> {
    sqlx::query!(
        r#"DELETE FROM env_var WHERE app_id = ? AND key = ?"#,
        app_id,
        key
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete all environment variables for an app
#[instrument(skip(pool))]
pub async fn delete_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<()> {
    sqlx::query!(
        r#"DELETE FROM env_var WHERE app_id = ?"#,
        app_id
    )
    .execute(pool)
    .await?;
    Ok(())
}
