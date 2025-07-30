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
