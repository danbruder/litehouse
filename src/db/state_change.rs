use super::*;

/// Save a process history entry
#[instrument(skip(pool, change))]
pub async fn save(pool: &Pool<Sqlite>, change: &AppStateChange) -> Result<()> {
    // Insert new entry
    let created_at_str = change.created_at.to_rfc3339();
    let state_str = change.state.to_string();
    let last_state_str = change.last_state.map(|s| s.to_string());

    sqlx::query!(
        r#"
            INSERT INTO app_state_change (
                id, app_id, created_at, state, last_state, last_error
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        change.id,
        change.app_id,
        created_at_str,
        state_str,
        last_state_str,
        change.last_error,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Get process history for an app
#[instrument(skip(pool))]
pub async fn get_by_app_id(pool: &Pool<Sqlite>, app_id: &str) -> Result<Vec<AppStateChange>> {
    let records = sqlx::query!(
        r#"
            SELECT id, app_id, created_at, state, last_state, last_error
            FROM app_state_change
            WHERE app_id = ?
            ORDER BY created_at DESC
            "#,
        app_id
    )
    .fetch_all(pool)
    .await?;

    let mut changes = Vec::new();

    for record in records {
        changes.push(AppStateChange {
            id: record.id,
            app_id: record.app_id,
            created_at: record.created_at.parse()?,
            state: parse_app_state(&record.state),
            last_state: record.last_state.as_deref().map(parse_app_state),
            last_error: record.last_error.unwrap_or_default(),
        });
    }

    Ok(changes)
}

/// Delete process history for an app
#[instrument(skip(pool))]
pub async fn delete_by_app_id(pool: &Pool<Sqlite>, app_id: &str) -> Result<u64> {
    let result = sqlx::query!(
        r#"
            DELETE FROM app_state_change
            WHERE app_id = ?
            "#,
        app_id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
