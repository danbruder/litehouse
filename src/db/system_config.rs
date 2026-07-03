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
            ghcr_token,
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

/// Save or update the GHCR token used to authenticate `docker pull` against
/// private ghcr.io images.
#[instrument(skip(pool, token))]
pub async fn set_ghcr_token(pool: &Pool<Sqlite>, token: &str) -> Result<()> {
    // Delete existing ghcr_token config if it exists
    sqlx::query!(
        r#"DELETE FROM system_config WHERE config_type = 'ghcr_token'"#,
    )
    .execute(pool)
    .await?;

    let config = SystemConfig::new_ghcr_token(token);

    sqlx::query!(
        r#"
        INSERT INTO system_config (
            id, config_type, ghcr_token, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?)
        "#,
        config.id,
        config.config_type,
        config.ghcr_token,
        config.created_at,
        config.updated_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the configured GHCR token, if any
#[instrument(skip(pool))]
pub async fn get_ghcr_token(pool: &Pool<Sqlite>) -> Result<Option<String>> {
    let record = sqlx::query!(
        r#"SELECT ghcr_token FROM system_config WHERE config_type = 'ghcr_token'"#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(record.and_then(|r| r.ghcr_token))
}

/// Delete the configured GHCR token
#[instrument(skip(pool))]
pub async fn delete_ghcr_token(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query!(
        r#"DELETE FROM system_config WHERE config_type = 'ghcr_token'"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::{SystemConfig, S3Config};

    #[tokio::test]
    async fn test_save_and_get_s3_config() {
        let pool = get_test_pool().await;
        let s3_config = S3Config {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            bucket: "my-bucket".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            path_prefix: Some("backups".to_string()),
        };

        let system_config = SystemConfig::new_s3_config(&s3_config);
        save_s3_config(&pool, &system_config).await.unwrap();

        let retrieved = get_s3_config(&pool).await.unwrap().unwrap();
        assert_eq!(retrieved.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(retrieved.secret_access_key, "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        assert_eq!(retrieved.bucket, "my-bucket");
        assert_eq!(retrieved.region, "us-east-1");
        assert_eq!(retrieved.endpoint, None);
        assert_eq!(retrieved.path_prefix, Some("backups".to_string()));
    }

    #[tokio::test]
    async fn test_get_s3_config_not_found() {
        let pool = get_test_pool().await;
        let result = get_s3_config(&pool).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_save_s3_config_replace_existing() {
        let pool = get_test_pool().await;
        let s3_config1 = S3Config {
            access_key_id: "KEY1".to_string(),
            secret_access_key: "SECRET1".to_string(),
            bucket: "bucket1".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            path_prefix: None,
        };

        let system_config1 = SystemConfig::new_s3_config(&s3_config1);
        save_s3_config(&pool, &system_config1).await.unwrap();

        let s3_config2 = S3Config {
            access_key_id: "KEY2".to_string(),
            secret_access_key: "SECRET2".to_string(),
            bucket: "bucket2".to_string(),
            region: "us-west-2".to_string(),
            endpoint: Some("https://s3.example.com".to_string()),
            path_prefix: Some("new-prefix".to_string()),
        };

        let system_config2 = SystemConfig::new_s3_config(&s3_config2);
        save_s3_config(&pool, &system_config2).await.unwrap();

        let retrieved = get_s3_config(&pool).await.unwrap().unwrap();
        assert_eq!(retrieved.access_key_id, "KEY2");
        assert_eq!(retrieved.bucket, "bucket2");
        assert_eq!(retrieved.region, "us-west-2");
        assert_eq!(retrieved.endpoint, Some("https://s3.example.com".to_string()));
    }

    #[tokio::test]
    async fn test_delete_s3_config() {
        let pool = get_test_pool().await;
        let s3_config = S3Config {
            access_key_id: "KEY1".to_string(),
            secret_access_key: "SECRET1".to_string(),
            bucket: "bucket1".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            path_prefix: None,
        };

        let system_config = SystemConfig::new_s3_config(&s3_config);
        save_s3_config(&pool, &system_config).await.unwrap();
        assert!(get_s3_config(&pool).await.unwrap().is_some());

        delete_s3_config(&pool).await.unwrap();
        assert!(get_s3_config(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_s3_config_with_endpoint() {
        let pool = get_test_pool().await;
        let s3_config = S3Config {
            access_key_id: "KEY1".to_string(),
            secret_access_key: "SECRET1".to_string(),
            bucket: "bucket1".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some("https://s3.custom.com".to_string()),
            path_prefix: None,
        };

        let system_config = SystemConfig::new_s3_config(&s3_config);
        save_s3_config(&pool, &system_config).await.unwrap();

        let retrieved = get_s3_config(&pool).await.unwrap().unwrap();
        assert_eq!(retrieved.endpoint, Some("https://s3.custom.com".to_string()));
    }

    #[tokio::test]
    async fn test_s3_config_default_path_prefix() {
        let pool = get_test_pool().await;
        let s3_config = S3Config {
            access_key_id: "KEY1".to_string(),
            secret_access_key: "SECRET1".to_string(),
            bucket: "bucket1".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            path_prefix: None,
        };

        let system_config = SystemConfig::new_s3_config(&s3_config);
        // SystemConfig::new_s3_config should set default path_prefix to "litehouse" if None
        assert_eq!(system_config.s3_path_prefix, Some("litehouse".to_string()));

        save_s3_config(&pool, &system_config).await.unwrap();
        let retrieved = get_s3_config(&pool).await.unwrap().unwrap();
        assert_eq!(retrieved.path_prefix, Some("litehouse".to_string()));
    }

    #[tokio::test]
    async fn test_set_and_get_ghcr_token() {
        let pool = get_test_pool().await;
        set_ghcr_token(&pool, "ghp_exampletoken").await.unwrap();

        let retrieved = get_ghcr_token(&pool).await.unwrap();
        assert_eq!(retrieved, Some("ghp_exampletoken".to_string()));
    }

    #[tokio::test]
    async fn test_get_ghcr_token_not_found() {
        let pool = get_test_pool().await;
        let result = get_ghcr_token(&pool).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_set_ghcr_token_replace_existing() {
        let pool = get_test_pool().await;
        set_ghcr_token(&pool, "ghp_first").await.unwrap();
        set_ghcr_token(&pool, "ghp_second").await.unwrap();

        let retrieved = get_ghcr_token(&pool).await.unwrap();
        assert_eq!(retrieved, Some("ghp_second".to_string()));
    }

    #[tokio::test]
    async fn test_delete_ghcr_token() {
        let pool = get_test_pool().await;
        set_ghcr_token(&pool, "ghp_exampletoken").await.unwrap();
        assert!(get_ghcr_token(&pool).await.unwrap().is_some());

        delete_ghcr_token(&pool).await.unwrap();
        assert!(get_ghcr_token(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_ghcr_token_independent_of_s3_config() {
        let pool = get_test_pool().await;
        let s3_config = S3Config {
            access_key_id: "KEY1".to_string(),
            secret_access_key: "SECRET1".to_string(),
            bucket: "bucket1".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            path_prefix: None,
        };
        let system_config = SystemConfig::new_s3_config(&s3_config);
        save_s3_config(&pool, &system_config).await.unwrap();
        set_ghcr_token(&pool, "ghp_exampletoken").await.unwrap();

        assert!(get_s3_config(&pool).await.unwrap().is_some());
        assert_eq!(
            get_ghcr_token(&pool).await.unwrap(),
            Some("ghp_exampletoken".to_string())
        );
    }
}
