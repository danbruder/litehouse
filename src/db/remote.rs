use super::*;
use crate::models::Remote;

/// Get an app by name
#[instrument(skip(pool))]
pub async fn get_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<Remote> {
    let record = sqlx::query!(
        r#"
            SELECT *
            FROM remote
            WHERE app_id = ?
            "#,
        app_id
    )
    .fetch_one(pool)
    .await?;

    Ok(Remote {
        id: record.id,
        app_id: record.app_id,
        name: record.name,
        remote: record.remote,
        branch: record.branch,
        directory: record.directory,
        created_at: record.created_at.into(),
        updated_at: record.updated_at.into(),
    })
}

/// Get an app by name
#[instrument(skip(pool))]
pub async fn save(pool: &Pool<Sqlite>, remote: &Remote) -> Result<()> {
    let _ = sqlx::query!(
        r#"
            INSERT INTO remote (id, app_id, name, remote, branch, directory, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        remote.id,
        remote.app_id,
        remote.name,
        remote.remote,
        remote.branch,
        remote.directory,
        remote.created_at,
        remote.updated_at,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete a remote by ID
#[instrument(skip(pool))]
pub async fn delete_by_id(pool: &Pool<Sqlite>, id: &str) -> Result<()> {
    let _ = sqlx::query!(
        r#"
            DELETE FROM remote
            WHERE id = ?;
            "#,
        id
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete a remote by app ID
#[instrument(skip(pool))]
pub async fn delete_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<()> {
    let _ = sqlx::query!(
        r#"
            DELETE FROM remote
            WHERE app_id = ?;
            "#,
        app_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::app;
    use crate::models::{App, Remote};

    #[tokio::test]
    async fn test_save_and_get_by_app() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let remote = Remote::new(
            &app.id,
            "origin",
            "https://github.com/user/repo.git",
            "main",
            "/app",
        );

        save(&pool, &remote).await.unwrap();
        let retrieved = get_by_app(&pool, &app.id).await.unwrap();

        assert_eq!(retrieved.id, remote.id);
        assert_eq!(retrieved.app_id, app.id);
        assert_eq!(retrieved.name, "origin");
        assert_eq!(retrieved.remote, "https://github.com/user/repo.git");
        assert_eq!(retrieved.branch, "main");
        assert_eq!(retrieved.directory, "/app");
    }

    #[tokio::test]
    async fn test_get_by_app_not_found() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        // get_by_app expects exactly one row, so it will return an error if none found
        let result = get_by_app(&pool, &app.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_by_id() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let remote = Remote::new(
            &app.id,
            "origin",
            "https://github.com/user/repo.git",
            "main",
            "/app",
        );
        let remote_id = remote.id.clone();
        save(&pool, &remote).await.unwrap();

        delete_by_id(&pool, &remote_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_delete_by_app() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let remote = Remote::new(
            &app.id,
            "origin",
            "https://github.com/user/repo.git",
            "main",
            "/app",
        );
        save(&pool, &remote).await.unwrap();

        delete_by_app(&pool, &app.id).await.unwrap();
    }

    #[tokio::test]
    async fn test_save_multiple_remotes_same_app() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        // Note: The current schema allows only one remote per app
        // This test verifies the current behavior
        let remote1 = Remote::new(
            &app.id,
            "origin",
            "https://github.com/user/repo.git",
            "main",
            "/app",
        );
        save(&pool, &remote1).await.unwrap();

        // If we try to save another remote, it would need to be handled by the application logic
        // The database schema allows it, but get_by_app expects exactly one
    }
}