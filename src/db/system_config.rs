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

/// Save the JSON-serialized report of the most recent backup run, keyed
/// under its own `config_type` row (mirrors the ghcr_token pattern).
#[instrument(skip(pool, report))]
pub async fn set_last_backup_report(
    pool: &Pool<Sqlite>,
    report: &crate::backup::BackupReport,
) -> Result<()> {
    let report_json = serde_json::to_string(report)?;

    sqlx::query!(
        r#"DELETE FROM system_config WHERE config_type = 'backup_report'"#,
    )
    .execute(pool)
    .await?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::models::now();

    sqlx::query!(
        r#"
        INSERT INTO system_config (
            id, config_type, last_backup_report, created_at, updated_at
        )
        VALUES (?, 'backup_report', ?, ?, ?)
        "#,
        id,
        report_json,
        now,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the JSON-serialized report of the most recent backup run, if any.
#[instrument(skip(pool))]
pub async fn get_last_backup_report(
    pool: &Pool<Sqlite>,
) -> Result<Option<crate::backup::BackupReport>> {
    let record = sqlx::query!(
        r#"SELECT last_backup_report FROM system_config WHERE config_type = 'backup_report'"#,
    )
    .fetch_optional(pool)
    .await?;

    match record.and_then(|r| r.last_backup_report) {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

/// Record the UTC date (YYYY-MM-DD) the daily backup scheduler last
/// successfully completed a run. Stored under its own `config_type` row
/// (`backup_meta`), independent of `backup_report`, so scheduler bookkeeping
/// can't be clobbered by (or clobber) the report row.
#[instrument(skip(pool))]
pub async fn set_last_backup_date(pool: &Pool<Sqlite>, date: &str) -> Result<()> {
    let now = crate::models::now();
    // id is only used on first insert; ON CONFLICT keeps the existing one.
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query!(
        r#"
        INSERT INTO system_config (id, config_type, last_backup_date, created_at, updated_at)
        VALUES (?, 'backup_meta', ?, ?, ?)
        ON CONFLICT(config_type) DO UPDATE SET
            last_backup_date = excluded.last_backup_date,
            updated_at = excluded.updated_at
        "#,
        id,
        date,
        now,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the UTC date (YYYY-MM-DD) of the last successful daily backup run,
/// if any has completed yet.
#[instrument(skip(pool))]
pub async fn get_last_backup_date(pool: &Pool<Sqlite>) -> Result<Option<String>> {
    let record = sqlx::query!(
        r#"SELECT last_backup_date FROM system_config WHERE config_type = 'backup_meta'"#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(record.and_then(|r| r.last_backup_date))
}

/// Record the Eastern-time date (YYYY-MM-DD) the nightly app-restart
/// scheduler last completed a pass. Stored under its own `config_type` row
/// (`nightly_restart_meta`), independent of `backup_meta` and every other
/// system_config row.
#[instrument(skip(pool))]
pub async fn set_last_nightly_restart_date(pool: &Pool<Sqlite>, date: &str) -> Result<()> {
    let now = crate::models::now();
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query!(
        r#"
        INSERT INTO system_config (id, config_type, last_nightly_restart_date, created_at, updated_at)
        VALUES (?, 'nightly_restart_meta', ?, ?, ?)
        ON CONFLICT(config_type) DO UPDATE SET
            last_nightly_restart_date = excluded.last_nightly_restart_date,
            updated_at = excluded.updated_at
        "#,
        id,
        date,
        now,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the Eastern-time date (YYYY-MM-DD) of the last completed nightly
/// restart pass, if any has run yet.
#[instrument(skip(pool))]
pub async fn get_last_nightly_restart_date(pool: &Pool<Sqlite>) -> Result<Option<String>> {
    let record = sqlx::query!(
        r#"SELECT last_nightly_restart_date FROM system_config WHERE config_type = 'nightly_restart_meta'"#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(record.and_then(|r| r.last_nightly_restart_date))
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

    #[tokio::test]
    async fn test_set_and_get_last_backup_report() {
        let pool = get_test_pool().await;
        let report = crate::backup::BackupReport {
            succeeded: vec!["app1".to_string()],
            failed: vec![("app2".to_string(), "boom".to_string())],
            ran_at: "2026-07-03T00:00:00Z".to_string(),
        };

        set_last_backup_report(&pool, &report).await.unwrap();
        let retrieved = get_last_backup_report(&pool).await.unwrap().unwrap();
        assert_eq!(retrieved.succeeded, vec!["app1".to_string()]);
        assert_eq!(
            retrieved.failed,
            vec![("app2".to_string(), "boom".to_string())]
        );
        assert_eq!(retrieved.ran_at, "2026-07-03T00:00:00Z");
    }

    #[tokio::test]
    async fn test_get_last_backup_report_not_found() {
        let pool = get_test_pool().await;
        assert!(get_last_backup_report(&pool).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_set_and_get_last_backup_date() {
        let pool = get_test_pool().await;
        assert!(get_last_backup_date(&pool).await.unwrap().is_none());

        set_last_backup_date(&pool, "2026-07-03").await.unwrap();
        assert_eq!(
            get_last_backup_date(&pool).await.unwrap(),
            Some("2026-07-03".to_string())
        );

        set_last_backup_date(&pool, "2026-07-04").await.unwrap();
        assert_eq!(
            get_last_backup_date(&pool).await.unwrap(),
            Some("2026-07-04".to_string())
        );
    }

    #[tokio::test]
    async fn test_last_backup_date_independent_of_report() {
        let pool = get_test_pool().await;
        let report = crate::backup::BackupReport {
            succeeded: vec!["app1".to_string()],
            failed: vec![],
            ran_at: "2026-07-03T00:00:00Z".to_string(),
        };
        set_last_backup_report(&pool, &report).await.unwrap();
        set_last_backup_date(&pool, "2026-07-03").await.unwrap();

        // Both should still be readable after each other's writes.
        assert_eq!(
            get_last_backup_date(&pool).await.unwrap(),
            Some("2026-07-03".to_string())
        );
        assert_eq!(
            get_last_backup_report(&pool).await.unwrap().unwrap().ran_at,
            "2026-07-03T00:00:00Z"
        );
    }

    #[tokio::test]
    async fn test_set_and_get_last_nightly_restart_date() {
        let pool = get_test_pool().await;
        assert!(get_last_nightly_restart_date(&pool).await.unwrap().is_none());

        set_last_nightly_restart_date(&pool, "2026-07-15").await.unwrap();
        assert_eq!(
            get_last_nightly_restart_date(&pool).await.unwrap(),
            Some("2026-07-15".to_string())
        );

        // Overwriting updates the same row rather than erroring or inserting
        // a second one.
        set_last_nightly_restart_date(&pool, "2026-07-16").await.unwrap();
        assert_eq!(
            get_last_nightly_restart_date(&pool).await.unwrap(),
            Some("2026-07-16".to_string())
        );
    }

    #[tokio::test]
    async fn test_last_nightly_restart_date_independent_of_last_backup_date() {
        let pool = get_test_pool().await;
        set_last_backup_date(&pool, "2026-07-01").await.unwrap();
        set_last_nightly_restart_date(&pool, "2026-07-02").await.unwrap();

        assert_eq!(
            get_last_backup_date(&pool).await.unwrap(),
            Some("2026-07-01".to_string())
        );
        assert_eq!(
            get_last_nightly_restart_date(&pool).await.unwrap(),
            Some("2026-07-02".to_string())
        );
    }

    #[tokio::test]
    async fn test_set_last_backup_report_replace_existing() {
        let pool = get_test_pool().await;
        let report1 = crate::backup::BackupReport {
            succeeded: vec!["app1".to_string()],
            failed: vec![],
            ran_at: "2026-07-03T00:00:00Z".to_string(),
        };
        set_last_backup_report(&pool, &report1).await.unwrap();

        let report2 = crate::backup::BackupReport {
            succeeded: vec!["app2".to_string()],
            failed: vec![],
            ran_at: "2026-07-04T00:00:00Z".to_string(),
        };
        set_last_backup_report(&pool, &report2).await.unwrap();

        let retrieved = get_last_backup_report(&pool).await.unwrap().unwrap();
        assert_eq!(retrieved.succeeded, vec!["app2".to_string()]);
        assert_eq!(retrieved.ran_at, "2026-07-04T00:00:00Z");
    }
}
