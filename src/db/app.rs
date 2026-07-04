use super::*;

/// Save an app to the database
#[instrument(skip(pool, app))]
pub async fn save(pool: &Pool<Sqlite>, app: &App) -> Result<()> {
    // Update or insert
    let result = sqlx::query!(
        r#"
            INSERT INTO app (
                id, name, port, organization_id, created_at, updated_at, state,
                repo, image, exposed_port, deploy_token_hash, custom_domains
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                port = excluded.port,
                organization_id = excluded.organization_id,
                updated_at = excluded.updated_at,
                state = excluded.state,
                repo = excluded.repo,
                image = excluded.image,
                exposed_port = excluded.exposed_port,
                deploy_token_hash = excluded.deploy_token_hash,
                custom_domains = excluded.custom_domains
            "#,
        app.id,
        app.name,
        app.port,
        app.organization_id,
        app.created_at,
        app.updated_at,
        //state
        app.state,
        app.repo,
        app.image,
        app.exposed_port,
        app.deploy_token_hash,
        app.custom_domains,
    )
    .execute(pool)
    .await?;

    debug!(
        "Saved app '{}' (affected rows: {})",
        app.name,
        result.rows_affected()
    );

    Ok(())
}

/// Insert an app row, doing nothing if a row with the same id already
/// exists. Unlike [`save`] (which upserts), this never overwrites an
/// existing row — used by disaster-recovery restore
/// (`backup::copy_apps_from_snapshot`) to merge apps from an S3 snapshot
/// into the live DB without clobbering current local state.
#[instrument(skip(pool, app))]
pub async fn insert_or_ignore(pool: &Pool<Sqlite>, app: &App) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT OR IGNORE INTO app (
                id, name, port, organization_id, created_at, updated_at, state,
                repo, image, exposed_port, deploy_token_hash, custom_domains
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        app.id,
        app.name,
        app.port,
        app.organization_id,
        app.created_at,
        app.updated_at,
        app.state,
        app.repo,
        app.image,
        app.exposed_port,
        app.deploy_token_hash,
        app.custom_domains,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Get an app by name
#[instrument(skip(pool))]
pub async fn get_by_name(pool: &Pool<Sqlite>, name: &str) -> Result<Option<App>> {
    let app = sqlx::query_as!(
        App,
        r#"
            SELECT *
            FROM app
            WHERE name = ?
            "#,
        name
    )
    .fetch_optional(pool)
    .await?;
    Ok(app)
}

/// Get an app by id
#[instrument(skip(pool))]
pub async fn get_by_id(pool: &Pool<Sqlite>, id: &str) -> Result<Option<App>> {
    let app = sqlx::query_as!(
        App,
        r#"
            SELECT *
            FROM app
            WHERE id = ?
            "#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(app)
}

/// Get all apps with a specific state
#[instrument(skip(pool))]
pub async fn get_by_state(pool: &Pool<Sqlite>, state: AppState) -> Result<Vec<App>> {
    let apps = sqlx::query_as!(
        App,
        r#"
            SELECT *
            FROM app 
            WHERE state = ?
            "#,
        state
    )
    .fetch_all(pool)
    .await?;

    Ok(apps)
}

/// Delete an app by ID
#[instrument(skip(pool))]
pub async fn delete_by_app_id(pool: &Pool<Sqlite>, id: &str) -> Result<()> {
    let _ = sqlx::query!(
        r#"
            DELETE FROM app
            WHERE id = ?;
            "#,
        id
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Get all apps
#[instrument(skip(pool))]
pub async fn get_all(pool: &Pool<Sqlite>) -> Result<Vec<App>> {
    let apps = sqlx::query_as!(
        App,
        r#"
            SELECT *
            FROM app 
            ORDER BY name
            "#
    )
    .fetch_all(pool)
    .await?;

    Ok(apps)
}

/// Record a successful deploy: update the app's image, exposed port, and
/// mark it running.
#[instrument(skip(pool))]
pub async fn set_deployed(
    pool: &Pool<Sqlite>,
    app_id: &str,
    image: &str,
    exposed_port: &str,
) -> Result<()> {
    let updated_at = chrono::Utc::now().to_rfc3339();
    let state = AppState::Running;
    sqlx::query!(
        r#"
            UPDATE app
            SET image = ?, exposed_port = ?, state = ?, updated_at = ?
            WHERE id = ?
            "#,
        image,
        exposed_port,
        state,
        updated_at,
        app_id,
    )
    .execute(pool)
    .await?;

    debug!("Marked app '{}' deployed with image '{}'", app_id, image);

    Ok(())
}

/// Set (or rotate) the deploy token hash for an app.
#[instrument(skip(pool, token_hash))]
pub async fn set_deploy_token_hash(
    pool: &Pool<Sqlite>,
    app_id: &str,
    token_hash: &str,
) -> Result<()> {
    let updated_at = chrono::Utc::now().to_rfc3339();
    sqlx::query!(
        r#"
            UPDATE app
            SET deploy_token_hash = ?, updated_at = ?
            WHERE id = ?
            "#,
        token_hash,
        updated_at,
        app_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Get all apps with assigned ports
#[instrument(skip(pool))]
pub async fn get_all_with_ports(pool: &Pool<Sqlite>) -> Result<Vec<App>> {
    let apps = sqlx::query_as!(
        App,
        r#"
            SELECT *
            FROM app 
            WHERE port IS NOT NULL
            ORDER BY name
            "#
    )
    .fetch_all(pool)
    .await?;

    Ok(apps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::{App, AppState};

    #[tokio::test]
    async fn test_save_and_get_by_id() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();

        save(&pool, &app).await.unwrap();
        let retrieved = get_by_id(&pool, &app.id).await.unwrap().unwrap();

        assert_eq!(retrieved.id, app.id);
        assert_eq!(retrieved.name, app.name);
        assert_eq!(retrieved.port, app.port);
        assert_eq!(retrieved.state, app.state);
    }

    #[tokio::test]
    async fn test_save_and_get_by_name() {
        let pool = get_test_pool().await;
        let app = App::new("myapp").unwrap();

        save(&pool, &app).await.unwrap();
        let retrieved = get_by_name(&pool, "myapp").await.unwrap().unwrap();

        assert_eq!(retrieved.id, app.id);
        assert_eq!(retrieved.name, "myapp");
        assert_eq!(retrieved.port, None);
    }

    #[tokio::test]
    async fn test_get_by_name_not_found() {
        let pool = get_test_pool().await;
        let result = get_by_name(&pool, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_save_and_get_by_state() {
        let pool = get_test_pool().await;
        let app1 = App::new("app1").unwrap();
        let mut app2 = App::new("app2").unwrap();
        app2.state = AppState::Running;

        save(&pool, &app1).await.unwrap();
        save(&pool, &app2).await.unwrap();

        let stopped_apps = get_by_state(&pool, AppState::Stopped).await.unwrap();
        assert_eq!(stopped_apps.len(), 1);
        assert_eq!(stopped_apps[0].name, "app1");

        let running_apps = get_by_state(&pool, AppState::Running).await.unwrap();
        assert_eq!(running_apps.len(), 1);
        assert_eq!(running_apps[0].name, "app2");
    }

    #[tokio::test]
    async fn test_delete_by_app_id() {
        let pool = get_test_pool().await;
        let app = App::new("todelete").unwrap();

        save(&pool, &app).await.unwrap();
        assert!(get_by_id(&pool, &app.id).await.unwrap().is_some());

        delete_by_app_id(&pool, &app.id).await.unwrap();
        assert!(get_by_id(&pool, &app.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_get_all() {
        let pool = get_test_pool().await;
        let app1 = App::new("app1").unwrap();
        let app2 = App::new("app2").unwrap();
        let app3 = App::new("app3").unwrap();

        save(&pool, &app1).await.unwrap();
        save(&pool, &app2).await.unwrap();
        save(&pool, &app3).await.unwrap();

        let all_apps = get_all(&pool).await.unwrap();
        assert_eq!(all_apps.len(), 3);
        assert_eq!(all_apps[0].name, "app1");
        assert_eq!(all_apps[1].name, "app2");
        assert_eq!(all_apps[2].name, "app3");
    }

    #[tokio::test]
    async fn test_get_all_with_ports() {
        let pool = get_test_pool().await;
        let app1 = App::new("app1").unwrap();
        let mut app2 = App::new("app2").unwrap();
        app2.port = Some(8080); // Explicitly set port for testing

        save(&pool, &app1).await.unwrap();
        save(&pool, &app2).await.unwrap();

        let apps_with_ports = get_all_with_ports(&pool).await.unwrap();
        assert_eq!(apps_with_ports.len(), 1);
        assert_eq!(apps_with_ports[0].name, "app2");
    }

    #[tokio::test]
    async fn test_save_update_existing() {
        let pool = get_test_pool().await;
        let mut app = App::new("updateapp").unwrap();
        let original_id = app.id.clone();

        save(&pool, &app).await.unwrap();
        app.name = "updatedname".to_string();
        app.port = Some(9000);
        app.state = AppState::Running;

        save(&pool, &app).await.unwrap();
        let retrieved = get_by_id(&pool, &original_id).await.unwrap().unwrap();

        assert_eq!(retrieved.id, original_id);
        assert_eq!(retrieved.name, "updatedname");
        assert_eq!(retrieved.port, Some(9000));
        assert_eq!(retrieved.state, AppState::Running);
    }

    #[tokio::test]
    async fn test_token_rotation_survives_repo_save() {
        // Regression: the create/rotate API path saves the repo (full-row
        // upsert) and then mints a new deploy-token hash. If the order were
        // reversed, save() would clobber the fresh hash with the stale one.
        let pool = get_test_pool().await;
        let app = App::new("rotateapp").unwrap();
        save(&pool, &app).await.unwrap();
        set_deploy_token_hash(&pool, &app.id, "old-hash").await.unwrap();

        // Mirror the handler's sequence: repo save first, then rotate.
        let mut updated = get_by_name(&pool, "rotateapp").await.unwrap().unwrap();
        updated.repo = Some("dan/rotateapp".to_string());
        save(&pool, &updated).await.unwrap();
        set_deploy_token_hash(&pool, &app.id, "new-hash").await.unwrap();

        let reloaded = get_by_name(&pool, "rotateapp").await.unwrap().unwrap();
        assert_eq!(reloaded.deploy_token_hash.as_deref(), Some("new-hash"));
        assert_eq!(reloaded.repo.as_deref(), Some("dan/rotateapp"));
    }

    #[tokio::test]
    async fn test_insert_or_ignore_inserts_new_row() {
        let pool = get_test_pool().await;
        let app = App::new("fresh-app").unwrap();

        insert_or_ignore(&pool, &app).await.unwrap();

        let retrieved = get_by_id(&pool, &app.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "fresh-app");
    }

    #[tokio::test]
    async fn test_insert_or_ignore_never_overwrites_existing_row() {
        let pool = get_test_pool().await;
        let mut app = App::new("existing-app").unwrap();
        save(&pool, &app).await.unwrap();

        // A "snapshot" copy of the same id with a different image.
        app.image = Some("ghcr.io/example/stale:tag".to_string());
        insert_or_ignore(&pool, &app).await.unwrap();

        let retrieved = get_by_id(&pool, &app.id).await.unwrap().unwrap();
        assert_eq!(retrieved.image, None, "insert_or_ignore must not clobber the live row");
    }

    #[tokio::test]
    async fn test_app_with_organization() {
        // Note: the `organization` table was dropped in the v2 simplification;
        // `organization_id` is now just an unused free-text column.
        let pool = get_test_pool().await;
        let app = App::new_with_org("orgapp", "some-org-id").unwrap();

        save(&pool, &app).await.unwrap();
        let retrieved = get_by_id(&pool, &app.id).await.unwrap().unwrap();

        assert_eq!(retrieved.organization_id, Some("some-org-id".to_string()));
    }
}
