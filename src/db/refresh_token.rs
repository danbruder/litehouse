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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::db::user;
    use crate::models::{RefreshToken, User};

    #[tokio::test]
    async fn test_save_and_get_by_token_hash() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let token = RefreshToken::new(&test_user.id, "raw-token-123", 7);
        let token_hash = token.token_hash.clone();

        save(&pool, &token).await.unwrap();
        let retrieved = get_by_token_hash(&pool, &token_hash).await.unwrap().unwrap();

        assert_eq!(retrieved.user_id, test_user.id);
        assert_eq!(retrieved.token_hash, token_hash);
        assert!(!retrieved.revoked);
    }

    #[tokio::test]
    async fn test_get_by_token_hash_not_found() {
        let pool = get_test_pool().await;
        let hash = RefreshToken::hash_token("nonexistent");
        let result = get_by_token_hash(&pool, &hash).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_revoke() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let token = RefreshToken::new(&test_user.id, "raw-token-123", 7);
        let token_hash = token.token_hash.clone();

        save(&pool, &token).await.unwrap();
        assert!(!get_by_token_hash(&pool, &token_hash).await.unwrap().unwrap().revoked);

        revoke(&pool, &token_hash).await.unwrap();
        let retrieved = get_by_token_hash(&pool, &token_hash).await.unwrap().unwrap();
        assert!(retrieved.revoked);
    }

    #[tokio::test]
    async fn test_revoke_all_for_user() {
        let pool = get_test_pool().await;
        let user1 = User::new("user1@example.com", "password123", None).unwrap();
        let user2 = User::new("user2@example.com", "password123", None).unwrap();
        user::save(&pool, &user1).await.unwrap();
        user::save(&pool, &user2).await.unwrap();

        let token1 = RefreshToken::new(&user1.id, "token1", 7);
        let token2 = RefreshToken::new(&user1.id, "token2", 7);
        let token3 = RefreshToken::new(&user2.id, "token3", 7);

        save(&pool, &token1).await.unwrap();
        save(&pool, &token2).await.unwrap();
        save(&pool, &token3).await.unwrap();

        revoke_all_for_user(&pool, &user1.id).await.unwrap();

        let retrieved1 = get_by_token_hash(&pool, &token1.token_hash).await.unwrap().unwrap();
        let retrieved2 = get_by_token_hash(&pool, &token2.token_hash).await.unwrap().unwrap();
        let retrieved3 = get_by_token_hash(&pool, &token3.token_hash).await.unwrap().unwrap();

        assert!(retrieved1.revoked);
        assert!(retrieved2.revoked);
        assert!(!retrieved3.revoked); // Different user
    }

    #[tokio::test]
    async fn test_get_active_tokens_for_user() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let token1 = RefreshToken::new(&test_user.id, "token1", 7);
        let token2 = RefreshToken::new(&test_user.id, "token2", 7);
        let mut token3 = RefreshToken::new(&test_user.id, "token3", 7);
        token3.revoke();

        save(&pool, &token1).await.unwrap();
        save(&pool, &token2).await.unwrap();
        save(&pool, &token3).await.unwrap();

        let active = get_active_tokens_for_user(&pool, &test_user.id).await.unwrap();
        assert_eq!(active.len(), 2);
        assert!(active.iter().all(|t| !t.revoked));
    }

    #[tokio::test]
    async fn test_delete() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let token = RefreshToken::new(&test_user.id, "raw-token-123", 7);
        let token_hash = token.token_hash.clone();

        save(&pool, &token).await.unwrap();
        assert!(get_by_token_hash(&pool, &token_hash).await.unwrap().is_some());

        delete(&pool, &token_hash).await.unwrap();
        assert!(get_by_token_hash(&pool, &token_hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_save_update_existing() {
        let pool = get_test_pool().await;
        let test_user = User::new("user@example.com", "password123", None).unwrap();
        user::save(&pool, &test_user).await.unwrap();

        let token = RefreshToken::new(&test_user.id, "raw-token-123", 7);
        let token_hash = token.token_hash.clone();

        save(&pool, &token).await.unwrap();
        assert!(!get_by_token_hash(&pool, &token_hash).await.unwrap().unwrap().revoked);

        // Update by saving again with revoked flag
        let mut updated_token = token;
        updated_token.revoke();
        save(&pool, &updated_token).await.unwrap();

        let retrieved = get_by_token_hash(&pool, &token_hash).await.unwrap().unwrap();
        assert!(retrieved.revoked);
    }
}
