use super::*;
use crate::models::GitHubConnection;

/// Save a GitHub connection to the database (upsert)
#[instrument(skip(pool, connection))]
pub async fn save(pool: &Pool<Sqlite>, connection: &GitHubConnection) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT INTO github_connection (
                id, user_id, github_user_id, github_username, github_email, access_token, scopes, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(user_id) DO UPDATE SET
                github_user_id = excluded.github_user_id,
                github_username = excluded.github_username,
                github_email = excluded.github_email,
                access_token = excluded.access_token,
                scopes = excluded.scopes,
                updated_at = excluded.updated_at
            "#,
        connection.id,
        connection.user_id,
        connection.github_user_id,
        connection.github_username,
        connection.github_email,
        connection.access_token,
        connection.scopes,
        connection.created_at,
        connection.updated_at
    )
    .execute(pool)
    .await?;

    debug!("Saved GitHub connection for user '{}'", connection.user_id);
    Ok(())
}

/// Get a GitHub connection by user ID
#[instrument(skip(pool))]
pub async fn get_by_user_id(pool: &Pool<Sqlite>, user_id: &str) -> Result<Option<GitHubConnection>> {
    let connection = sqlx::query_as!(
        GitHubConnection,
        r#"
            SELECT id, user_id, github_user_id, github_username, github_email, access_token, scopes, created_at, updated_at
            FROM github_connection
            WHERE user_id = ?
            "#,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(connection)
}

/// Delete a GitHub connection by user ID
#[instrument(skip(pool))]
pub async fn delete_by_user_id(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            DELETE FROM github_connection
            WHERE user_id = ?
            "#,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Deleted GitHub connection for user '{}'", user_id);
    Ok(())
}
