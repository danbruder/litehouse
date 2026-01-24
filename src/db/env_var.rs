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
}
