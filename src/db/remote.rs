use super::*;
use crate::models::Remote;

/// Get an app by name
#[instrument(skip(pool))]
pub async fn get_by_app_id(pool: &Pool<Sqlite>, app_id: &str) -> Result<Remote> {
    let record = sqlx::query_as!(
        Remote,
        r#"
            SELECT *
            FROM remote
            WHERE app_id = ?
            "#,
        name
    )
    .fetch(pool)
    .await?;

    Ok(record)
}
