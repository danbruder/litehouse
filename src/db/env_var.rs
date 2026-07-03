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

/// Insert an env var row, doing nothing if a row with the same id already
/// exists. Unlike [`save`] (which replaces by app_id+key), this never
/// overwrites an existing row — used by disaster-recovery restore
/// (`backup::copy_apps_from_snapshot`) to merge env vars from an S3
/// snapshot into the live DB without clobbering current local values.
#[instrument(skip(pool, env_var))]
pub async fn insert_or_ignore(pool: &Pool<Sqlite>, env_var: &EnvVar) -> Result<()> {
    sqlx::query!(
        r#"
        INSERT OR IGNORE INTO env_var (id, app_id, key, value, created_at, updated_at)
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

/// Initialize default environment variables for a new app
#[instrument(skip(pool))]
pub async fn init_default_env_vars(
    pool: &Pool<Sqlite>,
    app_id: &str,
    app_name: &str,
) -> Result<()> {
    let vars = vec![
        EnvVar::new(app_id, "DATABASE_PATH", "/data/app.db"),
        EnvVar::new(app_id, "DATA_DIR", "/data"),
        EnvVar::new(app_id, "APP_ID", app_id),
        EnvVar::new(app_id, "APP_NAME", app_name),
    ];

    for var in vars {
        save(pool, &var).await?;
    }

    info!("Initialized {} default environment variables for app {}", 4, app_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::app;
    use crate::models::{App, EnvVar};

    #[tokio::test]
    async fn test_save_and_get_by_app() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let env_var1 = EnvVar::new(&app.id, "KEY1", "value1");
        let env_var2 = EnvVar::new(&app.id, "KEY2", "value2");

        save(&pool, &env_var1).await.unwrap();
        save(&pool, &env_var2).await.unwrap();

        let retrieved = get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(retrieved.len(), 2);
        assert!(retrieved.iter().any(|e| e.key == "KEY1" && e.value == "value1"));
        assert!(retrieved.iter().any(|e| e.key == "KEY2" && e.value == "value2"));
    }

    #[tokio::test]
    async fn test_save_replace_existing() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let env_var1 = EnvVar::new(&app.id, "KEY1", "value1");
        save(&pool, &env_var1).await.unwrap();

        let env_var2 = EnvVar::new(&app.id, "KEY1", "updated_value");
        save(&pool, &env_var2).await.unwrap();

        let retrieved = get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].key, "KEY1");
        assert_eq!(retrieved[0].value, "updated_value");
    }

    #[tokio::test]
    async fn test_insert_or_ignore_never_overwrites_existing_row() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let env_var = EnvVar::new(&app.id, "KEY1", "value1");
        save(&pool, &env_var).await.unwrap();

        // Same id, different value (simulating a stale snapshot row).
        let mut stale = env_var.clone();
        stale.value = "stale-value".to_string();
        insert_or_ignore(&pool, &stale).await.unwrap();

        let retrieved = get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].value, "value1");
    }

    #[tokio::test]
    async fn test_delete_by_key() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let env_var1 = EnvVar::new(&app.id, "KEY1", "value1");
        let env_var2 = EnvVar::new(&app.id, "KEY2", "value2");
        save(&pool, &env_var1).await.unwrap();
        save(&pool, &env_var2).await.unwrap();

        delete_by_key(&pool, &app.id, "KEY1").await.unwrap();

        let retrieved = get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].key, "KEY2");
    }

    #[tokio::test]
    async fn test_delete_by_app() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        let env_var1 = EnvVar::new(&app.id, "KEY1", "value1");
        let env_var2 = EnvVar::new(&app.id, "KEY2", "value2");
        let env_var3 = EnvVar::new(&app.id, "KEY3", "value3");
        save(&pool, &env_var1).await.unwrap();
        save(&pool, &env_var2).await.unwrap();
        save(&pool, &env_var3).await.unwrap();

        delete_by_app(&pool, &app.id).await.unwrap();

        let retrieved = get_by_app(&pool, &app.id).await.unwrap();
        assert_eq!(retrieved.len(), 0);
    }

    #[tokio::test]
    async fn test_multiple_apps_isolation() {
        let pool = get_test_pool().await;
        let app1 = App::new("app1").unwrap();
        let app2 = App::new("app2").unwrap();
        app::save(&pool, &app1).await.unwrap();
        app::save(&pool, &app2).await.unwrap();

        let env_var1 = EnvVar::new(&app1.id, "KEY1", "value1");
        let env_var2 = EnvVar::new(&app2.id, "KEY1", "value2");
        save(&pool, &env_var1).await.unwrap();
        save(&pool, &env_var2).await.unwrap();

        let app1_vars = get_by_app(&pool, &app1.id).await.unwrap();
        let app2_vars = get_by_app(&pool, &app2.id).await.unwrap();

        assert_eq!(app1_vars.len(), 1);
        assert_eq!(app1_vars[0].value, "value1");
        assert_eq!(app2_vars.len(), 1);
        assert_eq!(app2_vars[0].value, "value2");
    }

    #[tokio::test]
    async fn test_init_default_env_vars() {
        let pool = get_test_pool().await;
        let app = App::new("testapp").unwrap();
        app::save(&pool, &app).await.unwrap();

        // Initialize default env vars
        init_default_env_vars(&pool, &app.id, &app.name).await.unwrap();

        // Retrieve all env vars
        let retrieved = get_by_app(&pool, &app.id).await.unwrap();

        // Should have exactly 4 default variables
        assert_eq!(retrieved.len(), 4);

        // Verify each default variable exists with correct value
        assert!(retrieved.iter().any(|e| e.key == "DATABASE_PATH" && e.value == "/data/app.db"));
        assert!(retrieved.iter().any(|e| e.key == "DATA_DIR" && e.value == "/data"));
        assert!(retrieved.iter().any(|e| e.key == "APP_ID" && e.value == app.id));
        assert!(retrieved.iter().any(|e| e.key == "APP_NAME" && e.value == app.name));
    }
}
