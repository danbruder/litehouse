use super::*;

/// Save an app to the database
#[instrument(skip(pool, app))]
pub async fn save(pool: &Pool<Sqlite>, app: &App) -> Result<()> {
    // Update or insert
    let result = sqlx::query!(
        r#"
            INSERT INTO app (
                id, name, port, created_at, updated_at, state
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                port = excluded.port,
                updated_at = excluded.updated_at,
                state = excluded.state
            "#,
        app.id,
        app.name,
        app.port,
        app.created_at,
        app.updated_at,
        //state
        app.state
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
