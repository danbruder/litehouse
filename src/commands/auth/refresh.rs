use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::auth::jwt::{generate_access_token, generate_refresh_token, get_refresh_token_ttl_days};
use crate::db;
use crate::models::{RefreshToken, TokenPair};

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("Invalid or expired refresh token")]
    InvalidToken,
    #[error("Refresh token has been revoked")]
    TokenRevoked,
    #[error("User not found")]
    UserNotFound,
    #[error("User account is not active")]
    UserNotActive,
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("JWT error: {0}")]
    JwtError(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, RefreshError>;

/// Refresh an access token using a refresh token
/// Returns a new access token and refresh token
/// The old refresh token is revoked and replaced with a new one (token rotation)
#[instrument(skip(pool, jwt_secret))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    old_refresh_token: &str,
    jwt_secret: &str,
) -> Result<TokenPair> {
    let token_hash = RefreshToken::hash_token(old_refresh_token);

    // Get the refresh token from database
    let token = db::refresh_token::get_by_token_hash(pool, &token_hash)
        .await?
        .ok_or(RefreshError::InvalidToken)?;

    // Check if token is valid (not revoked, not expired)
    if !token.is_valid() {
        if token.revoked {
            return Err(RefreshError::TokenRevoked);
        } else {
            return Err(RefreshError::InvalidToken);
        }
    }

    // Get the user
    let user = db::user::get_by_id(pool, &token.user_id)
        .await?
        .ok_or(RefreshError::UserNotFound)?;

    // Check if user is active
    if !user.is_active {
        return Err(RefreshError::UserNotActive);
    }

    // Revoke the old refresh token
    db::refresh_token::revoke(pool, &token_hash).await?;

    // Generate new tokens
    let access_token = generate_access_token(&user.id, &user.email, jwt_secret)?;
    let new_refresh_token_str = generate_refresh_token();
    let new_refresh_token = RefreshToken::new(&user.id, &new_refresh_token_str, get_refresh_token_ttl_days());
    db::refresh_token::save(pool, &new_refresh_token).await?;

    info!("Refreshed tokens for user '{}'", user.email);

    Ok(TokenPair::new(access_token, new_refresh_token_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::auth::register;
    use crate::db::test::get_test_pool;

    const TEST_JWT_SECRET: &str = "test-secret-for-refresh";

    #[tokio::test]
    async fn test_refresh_happy_path() {
        let pool = get_test_pool().await;

        // Register a user
        let auth_response = register::execute(
            &pool,
            "refresh@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        let old_refresh_token = auth_response.tokens.refresh_token;
        let old_access_token = auth_response.tokens.access_token;

        // Refresh the token
        let new_tokens = execute(&pool, &old_refresh_token, TEST_JWT_SECRET)
            .await
            .unwrap();

        // Verify we got new tokens
        assert_ne!(new_tokens.access_token, old_access_token);
        assert_ne!(new_tokens.refresh_token, old_refresh_token);

        // Verify old refresh token is revoked
        let old_token_hash = RefreshToken::hash_token(&old_refresh_token);
        let old_token = db::refresh_token::get_by_token_hash(&pool, &old_token_hash)
            .await
            .unwrap()
            .unwrap();
        assert!(old_token.revoked);
    }

    #[tokio::test]
    async fn test_refresh_invalid_token() {
        let pool = get_test_pool().await;

        let result = execute(&pool, "invalid-refresh-token", TEST_JWT_SECRET).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RefreshError::InvalidToken => {}
            _ => panic!("Expected InvalidToken error"),
        }
    }

    #[tokio::test]
    async fn test_refresh_revoked_token() {
        let pool = get_test_pool().await;

        // Register a user
        let auth_response = register::execute(
            &pool,
            "revoked@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        let refresh_token = auth_response.tokens.refresh_token;

        // Revoke the token
        let token_hash = RefreshToken::hash_token(&refresh_token);
        db::refresh_token::revoke(&pool, &token_hash)
            .await
            .unwrap();

        // Try to refresh with revoked token
        let result = execute(&pool, &refresh_token, TEST_JWT_SECRET).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RefreshError::TokenRevoked => {}
            _ => panic!("Expected TokenRevoked error"),
        }
    }

    #[tokio::test]
    async fn test_refresh_inactive_user() {
        let pool = get_test_pool().await;

        // Register a user
        let auth_response = register::execute(
            &pool,
            "inactive_refresh@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Deactivate the user
        db::user::deactivate(&pool, &auth_response.user.id)
            .await
            .unwrap();

        // Try to refresh
        let result = execute(&pool, &auth_response.tokens.refresh_token, TEST_JWT_SECRET).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RefreshError::UserNotActive => {}
            _ => panic!("Expected UserNotActive error"),
        }
    }

    #[tokio::test]
    async fn test_refresh_cannot_reuse_old_token() {
        let pool = get_test_pool().await;

        // Register a user
        let auth_response = register::execute(
            &pool,
            "reuse@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        let refresh_token = auth_response.tokens.refresh_token;

        // Refresh once (should succeed)
        execute(&pool, &refresh_token, TEST_JWT_SECRET)
            .await
            .unwrap();

        // Try to use the same token again (should fail)
        let result = execute(&pool, &refresh_token, TEST_JWT_SECRET).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RefreshError::TokenRevoked => {}
            _ => panic!("Expected TokenRevoked error (token rotation)"),
        }
    }
}
