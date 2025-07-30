use super::*;

use crate::models::Build;

/// Save an app to the database
#[instrument(skip(pool, build))]
pub async fn save(pool: &Pool<Sqlite>, build: &Build) -> Result<()> {
    // Update or insert
    let result = sqlx::query!(
        r#"
            INSERT INTO build (
                id, app_id, image_id, image_tag, git_commit, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                app_id = excluded.app_id, 
                image_id = excluded.image_id,
                image_tag = excluded.image_tag,
                git_commit = excluded.git_commit,
                updated_at = excluded.updated_at
            "#,
        build.id,
        build.app_id,
        build.image_id,
        build.image_tag,
        build.git_commit,
        build.created_at,
        build.updated_at
    )
    .execute(pool)
    .await?;

    debug!(
        "Saved build '{}' (affected rows: {})",
        build.id,
        result.rows_affected()
    );

    Ok(())
}
/// Get an app by name
#[instrument(skip(pool))]
pub async fn get_latest_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<Option<Build>> {
    let build = sqlx::query_as!(
        Build,
        r#"
            SELECT *
            FROM build
            WHERE app_id = ?
            "#,
        app_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(build)
}
