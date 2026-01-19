use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::db;
use crate::models::RefreshToken;

#[derive(Debug, thiserror::Error)]
pub enum LogoutError {
    #[error("Invalid or expired refresh token")]
    InvalidToken,
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
}

pub type Result<T> = std::result::Result<T, LogoutError>;

/// Logout a user by revoking their refresh token
#[instrument(skip(pool))]
pub async fn execute(pool: &Pool<Sqlite>, refresh_token: &str) -> Result<()> {
    let token_hash = RefreshToken::hash_token(refresh_token);

    // Get the refresh token from database
    let token = db::refresh_token::get_by_token_hash(pool, &token_hash)
        .await?
        .ok_or(LogoutError::InvalidToken)?;

    // Revoke the token
    db::refresh_token::revoke(pool, &token_hash).await?;

    info!("Revoked refresh token for user '{}'", token.user_id);

    Ok(())
}

/// Logout all sessions for a user by revoking all their refresh tokens
#[instrument(skip(pool))]
pub async fn logout_all_sessions(pool: &Pool<Sqlite>, user_id: &str) -> Result<()> {
    db::refresh_token::revoke_all_for_user(pool, user_id).await?;

    info!("Revoked all refresh tokens for user '{}'", user_id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt::{generate_refresh_token, get_refresh_token_ttl_days};
    use crate::commands::auth::register;
    use crate::db::test::get_test_pool;

    const TEST_JWT_SECRET: &str = "test-secret-for-logout";

    #[tokio::test]
    async fn test_logout_happy_path() {
        let pool = get_test_pool().await;

        // Register a user
        let auth_response = register::execute(
            &pool,
            "logout@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Logout
        execute(&pool, &auth_response.tokens.refresh_token)
            .await
            .unwrap();

        // Verify token is revoked
        let token_hash = RefreshToken::hash_token(&auth_response.tokens.refresh_token);
        let token = db::refresh_token::get_by_token_hash(&pool, &token_hash)
            .await
            .unwrap()
            .unwrap();
        assert!(token.revoked);
    }

    #[tokio::test]
    async fn test_logout_invalid_token() {
        let pool = get_test_pool().await;

        let result = execute(&pool, "invalid-token-12345").await;

        assert!(result.is_err());
        match result.unwrap_err() {
            LogoutError::InvalidToken => {}
            _ => panic!("Expected InvalidToken error"),
        }
    }

    #[tokio::test]
    async fn test_logout_all_sessions() {
        let pool = get_test_pool().await;

        // Register a user
        let auth_response = register::execute(
            &pool,
            "logoutall@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Create additional refresh tokens for the same user
        let user_id = &auth_response.user.id;
        let token1_str = generate_refresh_token();
        let token1 = RefreshToken::new(user_id, &token1_str, get_refresh_token_ttl_days());
        db::refresh_token::save(&pool, &token1).await.unwrap();

        let token2_str = generate_refresh_token();
        let token2 = RefreshToken::new(user_id, &token2_str, get_refresh_token_ttl_days());
        db::refresh_token::save(&pool, &token2).await.unwrap();

        // Verify we have 3 active tokens
        let active_tokens = db::refresh_token::get_active_tokens_for_user(&pool, user_id)
            .await
            .unwrap();
        assert_eq!(active_tokens.len(), 3);

        // Logout all sessions
        logout_all_sessions(&pool, user_id).await.unwrap();

        // Verify all tokens are revoked
        let active_tokens = db::refresh_token::get_active_tokens_for_user(&pool, user_id)
            .await
            .unwrap();
        assert_eq!(active_tokens.len(), 0);
    }
}
