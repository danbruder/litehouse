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