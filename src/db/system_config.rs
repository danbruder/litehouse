use super::*;
use crate::models::{SystemConfig, S3Config};

/// Save or update S3 backup configuration
#[instrument(skip(pool))]
pub async fn save_s3_config(pool: &Pool<Sqlite>, config: &SystemConfig) -> Result<()> {
    // Delete existing s3_backup config if it exists
    sqlx::query!(
        r#"DELETE FROM system_config WHERE config_type = 's3_backup'"#,
    )
    .execute(pool)
    .await?;

    // Insert the new config
    sqlx::query!(
        r#"
        INSERT INTO system_config (
            id, config_type, s3_access_key_id, s3_secret_access_key,
            s3_bucket, s3_region, s3_endpoint, s3_path_prefix,
            created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        config.id,
        config.config_type,
        config.s3_access_key_id,
        config.s3_secret_access_key,
        config.s3_bucket,
        config.s3_region,
        config.s3_endpoint,
        config.s3_path_prefix,
        config.created_at,
        config.updated_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get S3 backup configuration
#[instrument(skip(pool))]
pub async fn get_s3_config(pool: &Pool<Sqlite>) -> Result<Option<S3Config>> {
    let record: Option<SystemConfig> = sqlx::query_as!(
        SystemConfig,
        r#"
        SELECT
            id, config_type,
            s3_access_key_id, s3_secret_access_key,
            s3_bucket, s3_region, s3_endpoint, s3_path_prefix,
            created_at as "created_at: _", updated_at as "updated_at: _"
        FROM system_config
        WHERE config_type = 's3_backup'
        "#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(record.and_then(|r| r.to_s3_config()))
}

/// Delete S3 backup configuration
#[instrument(skip(pool))]
pub async fn delete_s3_config(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query!(
        r#"DELETE FROM system_config WHERE config_type = 's3_backup'"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}
