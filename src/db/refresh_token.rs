use super::*;
use crate::models::RefreshToken;

/// Save a refresh token to the database
#[instrument(skip(pool, token))]
pub async fn save(pool: &Pool<Sqlite>, token: &RefreshToken) -> Result<()> {
    sqlx::query!(
        r#"
            INSERT INTO refresh_token (
                id, user_id, token_hash, expires_at, revoked, created_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(token_hash) DO UPDATE SET
                revoked = excluded.revoked
            "#,
        token.id,
        token.user_id,
        token.token_hash,
        token.expires_at,
        token.revoked,
        token.created_at
    )
    .execute(pool)
    .await?;

    debug!("Saved refresh token for user '{}'", token.user_id);
    Ok(())
}

/// Get a refresh token by token hash
#[instrument(skip(pool))]
pub async fn get_by_token_hash(pool: &Pool<Sqlite>, hash: &str) -> Result<Option<RefreshToken>> {
    let token = sqlx::query_as!(
        RefreshToken,
        r#"
            SELECT id, user_id, token_hash, expires_at, revoked, created_at
            FROM refresh_token
            WHERE token_hash = ?
            "#,
        hash
    )
    .fetch_optional(pool)
    .await?;

    Ok(token)
}

/// Revoke a refresh token
#[instrument(skip(pool))]
pub async fn revoke(pool: &Pool<Sqlite>, token_hash: &str) -> Result<()> {
    sqlx::query!(
        r#"
            UPDATE refresh_token
            SET revoked = true
            WHERE token_hash = ?
            "#,
        token_hash
    )
    .execute(pool)
    .await?;

    debug!("Revoked refresh token");
    Ok(())
}

/// Revoke all refresh tokens for a user
#[instrument(skip(pool))]
pub async fn revoke_all_for_user(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    sqlx::query!(
        r#"
            UPDATE refresh_token
            SET revoked = true
            WHERE user_id = ?
            "#,
        user_id
    )
    .execute(pool)
    .await?;

    debug!("Revoked all refresh tokens for user '{}'", user_id);
    Ok(())
}

/// Cleanup expired tokens
#[instrument(skip(pool))]
pub async fn cleanup_expired(pool: &Pool<Sqlite>) -> Result<()> {
    let result = sqlx::query!(
        r#"
            DELETE FROM refresh_token
            WHERE expires_at < datetime('now')
            "#
    )
    .execute(pool)
    .await?;

    debug!("Cleaned up {} expired refresh tokens", result.rows_affected());
    Ok(())
}

/// Get all active tokens for a user
#[instrument(skip(pool))]
pub async fn get_active_tokens_for_user(pool: &Pool<Sqlite>, user_id: &str) -> Result<Vec<RefreshToken>> {
    let tokens = sqlx::query_as!(
        RefreshToken,
        r#"
            SELECT id, user_id, token_hash, expires_at, revoked, created_at
            FROM refresh_token
            WHERE user_id = ? AND revoked = false AND expires_at > datetime('now')
            ORDER BY created_at DESC
            "#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(tokens)
}

/// Delete a specific refresh token
#[instrument(skip(pool))]
pub async fn delete(pool: &Pool<Sqlite>, token_hash: &str) -> Result<()> {
    sqlx::query!(
        r#"
            DELETE FROM refresh_token
            WHERE token_hash = ?
            "#,
        token_hash
    )
    .execute(pool)
    .await?;

    debug!("Deleted refresh token");
    Ok(())
}
