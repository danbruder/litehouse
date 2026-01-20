use super::*;

use crate::models::Build;
use crate::models::UtcDateTime;

/// Save a build to the database
#[instrument(skip(pool, build))]
pub async fn save(pool: &Pool<Sqlite>, build: &Build) -> Result<()> {
    // Update or insert
    let result = sqlx::query(
        r#"
            INSERT INTO build (
                id, app_id, image_id, image_tag, git_commit, log_path, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                app_id = excluded.app_id,
                image_id = excluded.image_id,
                image_tag = excluded.image_tag,
                git_commit = excluded.git_commit,
                log_path = excluded.log_path,
                updated_at = excluded.updated_at
            "#,
    )
    .bind(&build.id)
    .bind(&build.app_id)
    .bind(&build.image_id)
    .bind(&build.image_tag)
    .bind(&build.git_commit)
    .bind(&build.log_path)
    .bind(&build.created_at)
    .bind(&build.updated_at)
    .execute(pool)
    .await?;

    debug!(
        "Saved build '{}' (affected rows: {})",
        build.id,
        result.rows_affected()
    );

    Ok(())
}

/// Get latest build by app id
#[instrument(skip(pool))]
pub async fn get_latest_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<Option<Build>> {
    let row = sqlx::query(
        r#"
            SELECT id, app_id, image_id, image_tag, git_commit, log_path, created_at, updated_at
            FROM build
            WHERE app_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
    )
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| build_from_row(&r)))
}

/// Get build by id
#[instrument(skip(pool))]
pub async fn get_by_id(pool: &Pool<Sqlite>, build_id: &str) -> Result<Option<Build>> {
    let row = sqlx::query(
        r#"
            SELECT id, app_id, image_id, image_tag, git_commit, log_path, created_at, updated_at
            FROM build
            WHERE id = ?
            "#,
    )
    .bind(build_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| build_from_row(&r)))
}

/// Get all builds for an app, ordered by created_at desc
#[instrument(skip(pool))]
pub async fn get_all_by_app(pool: &Pool<Sqlite>, app_id: &str) -> Result<Vec<Build>> {
    let rows = sqlx::query(
        r#"
            SELECT id, app_id, image_id, image_tag, git_commit, log_path, created_at, updated_at
            FROM build
            WHERE app_id = ?
            ORDER BY created_at DESC
            "#,
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(build_from_row).collect())
}

/// Delete old builds, keeping only the most recent `keep_count` builds per app
#[instrument(skip(pool))]
pub async fn delete_old_builds(pool: &Pool<Sqlite>, app_id: &str, keep_count: i64) -> Result<Vec<Build>> {
    // First get the builds to delete (so we can return them for log cleanup)
    let rows = sqlx::query(
        r#"
            SELECT id, app_id, image_id, image_tag, git_commit, log_path, created_at, updated_at
            FROM build
            WHERE app_id = ?
            ORDER BY created_at DESC
            LIMIT -1 OFFSET ?
            "#,
    )
    .bind(app_id)
    .bind(keep_count)
    .fetch_all(pool)
    .await?;

    let builds_to_delete: Vec<Build> = rows.iter().map(build_from_row).collect();

    // Delete them
    sqlx::query(
        r#"
            DELETE FROM build
            WHERE app_id = ? AND id NOT IN (
                SELECT id FROM build
                WHERE app_id = ?
                ORDER BY created_at DESC
                LIMIT ?
            )
            "#,
    )
    .bind(app_id)
    .bind(app_id)
    .bind(keep_count)
    .execute(pool)
    .await?;

    Ok(builds_to_delete)
}

/// Helper to convert a row to a Build
fn build_from_row(row: &sqlx::sqlite::SqliteRow) -> Build {
    use sqlx::Row;
    Build {
        id: row.get("id"),
        app_id: row.get("app_id"),
        image_id: row.get("image_id"),
        image_tag: row.get("image_tag"),
        git_commit: row.get("git_commit"),
        log_path: row.get("log_path"),
        created_at: row.get::<UtcDateTime, _>("created_at"),
        updated_at: row.get::<UtcDateTime, _>("updated_at"),
    }
}
