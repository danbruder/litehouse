use super::*;
use crate::models::Deploy;

/// Insert a new deploy record
#[instrument(skip(pool, deploy))]
pub async fn insert(pool: &Pool<Sqlite>, deploy: &Deploy) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT INTO deploy (
                id, app_id, image, git_sha, status, error, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        deploy.id,
        deploy.app_id,
        deploy.image,
        deploy.git_sha,
        deploy.status,
        deploy.error,
        deploy.created_at,
        deploy.updated_at,
    )
    .execute(pool)
    .await?;

    debug!("Inserted deploy '{}' for app '{}'", deploy.id, deploy.app_id);

    Ok(())
}

/// Update the status (and optional error) of an existing deploy
#[instrument(skip(pool))]
pub async fn set_status(
    pool: &Pool<Sqlite>,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let updated_at = chrono::Utc::now().to_rfc3339();
    sqlx::query!(
        r#"
            UPDATE deploy
            SET status = ?, error = ?, updated_at = ?
            WHERE id = ?
            "#,
        status,
        error,
        updated_at,
        id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Get the most recent deploy for an app
#[instrument(skip(pool))]
pub async fn latest_for_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<Option<Deploy>> {
    let deploy = sqlx::query_as!(
        Deploy,
        r#"
            SELECT *
            FROM deploy
            WHERE app_id = ?
            ORDER BY created_at DESC, rowid DESC
            LIMIT 1
            "#,
        app_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(deploy)
}

/// List the most recent deploys for an app, newest first
#[instrument(skip(pool))]
pub async fn list_for_app(pool: &Pool<Sqlite>, app_id: &str, limit: i64) -> Result<Vec<Deploy>> {
    let deploys = sqlx::query_as!(
        Deploy,
        r#"
            SELECT *
            FROM deploy
            WHERE app_id = ?
            ORDER BY created_at DESC, rowid DESC
            LIMIT ?
            "#,
        app_id,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(deploys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::{App, Deploy};

    fn new_deploy(app_id: &str, image: &str) -> Deploy {
        let now = chrono::Utc::now().to_rfc3339();
        Deploy {
            id: uuid::Uuid::new_v4().to_string(),
            app_id: app_id.to_string(),
            image: image.to_string(),
            git_sha: Some("abc123".to_string()),
            status: "in_progress".to_string(),
            error: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    async fn seed_app(pool: &Pool<Sqlite>, name: &str) -> App {
        let app = App::new(name).unwrap();
        crate::db::app::save(pool, &app).await.unwrap();
        app
    }

    #[tokio::test]
    async fn test_insert_and_latest_for_app() {
        let pool = get_test_pool().await;
        let app = seed_app(&pool, "deployapp").await;

        let deploy = new_deploy(&app.id, "deployapp:1");
        insert(&pool, &deploy).await.unwrap();

        let latest = latest_for_app(&pool, &app.id).await.unwrap().unwrap();
        assert_eq!(latest.id, deploy.id);
        assert_eq!(latest.image, "deployapp:1");
        assert_eq!(latest.status, "in_progress");
    }

    #[tokio::test]
    async fn test_set_status() {
        let pool = get_test_pool().await;
        let app = seed_app(&pool, "deployapp2").await;

        let deploy = new_deploy(&app.id, "deployapp2:1");
        insert(&pool, &deploy).await.unwrap();

        set_status(&pool, &deploy.id, "succeeded", None).await.unwrap();

        let latest = latest_for_app(&pool, &app.id).await.unwrap().unwrap();
        assert_eq!(latest.status, "succeeded");
        assert_eq!(latest.error, None);

        set_status(&pool, &deploy.id, "failed", Some("boom")).await.unwrap();
        let latest = latest_for_app(&pool, &app.id).await.unwrap().unwrap();
        assert_eq!(latest.status, "failed");
        assert_eq!(latest.error, Some("boom".to_string()));
    }

    #[tokio::test]
    async fn test_list_for_app() {
        let pool = get_test_pool().await;
        let app = seed_app(&pool, "deployapp3").await;

        for i in 0..3 {
            let mut d = new_deploy(&app.id, &format!("deployapp3:{}", i));
            // Ensure distinct created_at ordering
            d.created_at = format!("2026-01-0{}T00:00:00Z", i + 1);
            d.updated_at = d.created_at.clone();
            insert(&pool, &d).await.unwrap();
        }

        let deploys = list_for_app(&pool, &app.id, 10).await.unwrap();
        assert_eq!(deploys.len(), 3);
        // newest first
        assert_eq!(deploys[0].image, "deployapp3:2");
        assert_eq!(deploys[2].image, "deployapp3:0");

        let limited = list_for_app(&pool, &app.id, 2).await.unwrap();
        assert_eq!(limited.len(), 2);
    }
}
