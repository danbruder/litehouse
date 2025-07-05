use super::*;

/// Save an app to the database
#[instrument(skip(pool, app))]
pub async fn save(pool: &Pool<Sqlite>, app: &App) -> Result<()> {
    // Update or insert
    let state = app.state.to_string();
    let created_at_str = app.created_at.to_rfc3339();
    let updated_at_str = app.updated_at.to_rfc3339();

    let result = sqlx::query!(
        r#"
            INSERT INTO app (
                id, name, created_at, updated_at, state
            ) VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                updated_at = excluded.updated_at,
                state = excluded.state
            "#,
        app.id,
        app.name,
        created_at_str,
        updated_at_str,
        state,
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
    let record = sqlx::query!(
        r#"
            SELECT id, name, created_at, updated_at, state 
            FROM app 
            WHERE name = ?
            "#,
        name
    )
    .fetch_optional(pool)
    .await?;

    match record {
        Some(record) => {
            let state = parse_app_state(record.state.as_deref().unwrap_or("created"));
            Ok(Some(App {
                id: record.id,
                name: record.name,
                created_at: record.created_at.parse()?,
                updated_at: record.updated_at.parse()?,
                state,
            }))
        }
        None => Ok(None),
    }
}

/// Get all apps with a specific state
#[instrument(skip(pool))]
pub async fn get_by_state(pool: &Pool<Sqlite>, state: AppState) -> Result<Vec<App>> {
    let state_str = state.to_string();

    let records = sqlx::query!(
        r#"
            SELECT id, name, created_at, updated_at, state
            FROM app 
            WHERE state = ?
            "#,
        state_str
    )
    .fetch_all(pool)
    .await?;

    let mut apps = Vec::new();

    for record in records {
        let state = parse_app_state(record.state.as_deref().unwrap_or("created"));

        apps.push(App {
            id: record.id,
            name: record.name,
            created_at: record.created_at.parse()?,
            updated_at: record.updated_at.parse()?,
            state,
        });
    }

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
    let records = sqlx::query!(
        r#"
            SELECT id, name, created_at, updated_at, state 
            FROM app 
            ORDER BY name
            "#
    )
    .fetch_all(pool)
    .await?;

    let mut apps = Vec::new();

    for record in records {
        let state = parse_app_state(record.state.as_deref().unwrap_or("created"));
        apps.push(App {
            id: record.id,
            name: record.name,
            created_at: record.created_at.parse()?,
            updated_at: record.updated_at.parse()?,
            state,
        });
    }

    Ok(apps)
}
