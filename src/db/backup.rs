use super::*;
use crate::models::BackupRecord;

/// Record (or, for an already-catalogued S3 key, update) a successfully
/// uploaded backup artifact.
#[instrument(skip(pool))]
pub async fn record_upload(
    pool: &Pool<Sqlite>,
    app_name: &str,
    s3_key: &str,
    size_bytes: i64,
) -> Result<()> {
    let record = BackupRecord::new(app_name, s3_key, size_bytes);
    sqlx::query!(
        r#"
        INSERT INTO backup (id, app_name, s3_key, size_bytes, status, created_at)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(s3_key) DO UPDATE SET
            size_bytes = excluded.size_bytes,
            status = excluded.status,
            created_at = excluded.created_at
        "#,
        record.id,
        record.app_name,
        record.s3_key,
        record.size_bytes,
        record.status,
        record.created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Every catalogued backup, newest first.
#[instrument(skip(pool))]
pub async fn list_all(pool: &Pool<Sqlite>) -> Result<Vec<BackupRecord>> {
    let rows = sqlx::query_as!(
        BackupRecord,
        r#"SELECT id, app_name, s3_key, size_bytes, status, created_at FROM backup ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Remove catalog rows for the given S3 keys — called alongside S3-side
/// pruning so the catalog never lists a backup that no longer exists in S3.
#[instrument(skip(pool, keys))]
pub async fn delete_by_keys(pool: &Pool<Sqlite>, keys: &[String]) -> Result<()> {
    for key in keys {
        sqlx::query!(r#"DELETE FROM backup WHERE s3_key = ?"#, key)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;

    #[tokio::test]
    async fn record_and_list_backups_newest_first() {
        let pool = get_test_pool().await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1000)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-11.tar.gz", 2000)
            .await
            .unwrap();

        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].s3_key, "apps/app-a/2026-07-11.tar.gz");
    }

    #[tokio::test]
    async fn record_upload_same_key_replaces_row() {
        let pool = get_test_pool().await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1000)
            .await
            .unwrap();
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1500)
            .await
            .unwrap();

        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size_bytes, 1500);
    }

    #[tokio::test]
    async fn delete_by_keys_removes_matching_rows_only() {
        let pool = get_test_pool().await;
        record_upload(&pool, "app-a", "apps/app-a/2026-07-10.tar.gz", 1000)
            .await
            .unwrap();
        record_upload(&pool, "app-b", "apps/app-b/2026-07-10.tar.gz", 2000)
            .await
            .unwrap();

        delete_by_keys(&pool, &["apps/app-a/2026-07-10.tar.gz".to_string()])
            .await
            .unwrap();

        let rows = list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].app_name, "app-b");
    }
}
