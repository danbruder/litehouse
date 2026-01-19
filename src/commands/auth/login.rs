use sqlx::{Pool, Sqlite};
use tracing::{info, instrument};

use crate::auth::jwt::{generate_access_token, generate_refresh_token, get_refresh_token_ttl_days};
use crate::db;
use crate::models::{AuthResponse, RefreshToken, TokenPair};

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("Invalid email or password")]
    InvalidCredentials,
    #[error("User account is not active")]
    UserNotActive,
    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::DatabaseError),
    #[error("JWT error: {0}")]
    JwtError(#[from] anyhow::Error),
    #[error("User error: {0}")]
    UserError(#[from] crate::models::UserError),
}

pub type Result<T> = std::result::Result<T, LoginError>;

/// Login a user with email and password
#[instrument(skip(pool, password, jwt_secret))]
pub async fn execute(
    pool: &Pool<Sqlite>,
    email: &str,
    password: &str,
    jwt_secret: &str,
) -> Result<AuthResponse> {
    // Get user by email
    let user = db::user::get_by_email(pool, email)
        .await?
        .ok_or(LoginError::InvalidCredentials)?;

    // Check if user is active
    if !user.is_active {
        return Err(LoginError::UserNotActive);
    }

    // Verify password
    if !user.verify_password(password)? {
        return Err(LoginError::InvalidCredentials);
    }

    info!("User '{}' logged in successfully", user.email);

    // Generate tokens
    let access_token = generate_access_token(&user.id, &user.email, jwt_secret)?;
    let refresh_token_str = generate_refresh_token();
    let refresh_token = RefreshToken::new(&user.id, &refresh_token_str, get_refresh_token_ttl_days());
    db::refresh_token::save(pool, &refresh_token).await?;

    let tokens = TokenPair::new(access_token, refresh_token_str);

    // Get user organizations with roles
    let orgs = db::organization::get_user_organizations(pool, &user.id).await?;

    Ok(AuthResponse::new(user, tokens, orgs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::auth::register;
    use crate::db::test::get_test_pool;

    const TEST_JWT_SECRET: &str = "test-secret-for-login";

    #[tokio::test]
    async fn test_login_happy_path() {
        let pool = get_test_pool().await;

        // First register a user
        register::execute(
            &pool,
            "login@example.com",
            "password123",
            Some("Login User".to_string()),
            Some("Login Org".to_string()),
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Now login
        let response = execute(&pool, "login@example.com", "password123", TEST_JWT_SECRET)
            .await
            .unwrap();

        assert_eq!(response.user.email, "login@example.com");
        assert!(!response.tokens.access_token.is_empty());
        assert!(!response.tokens.refresh_token.is_empty());
        assert_eq!(response.organizations.len(), 1);
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let pool = get_test_pool().await;

        // Register a user
        register::execute(
            &pool,
            "wrong@example.com",
            "correct_password",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Try to login with wrong password
        let result = execute(&pool, "wrong@example.com", "wrong_password", TEST_JWT_SECRET).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            LoginError::InvalidCredentials => {}
            _ => panic!("Expected InvalidCredentials error"),
        }
    }

    #[tokio::test]
    async fn test_login_nonexistent_user() {
        let pool = get_test_pool().await;

        let result = execute(
            &pool,
            "nonexistent@example.com",
            "password123",
            TEST_JWT_SECRET,
        )
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            LoginError::InvalidCredentials => {}
            _ => panic!("Expected InvalidCredentials error"),
        }
    }

    #[tokio::test]
    async fn test_login_inactive_user() {
        let pool = get_test_pool().await;

        // Register a user
        register::execute(
            &pool,
            "inactive@example.com",
            "password123",
            None,
            None,
            TEST_JWT_SECRET,
        )
        .await
        .unwrap();

        // Deactivate the user
        let user = db::user::get_by_email(&pool, "inactive@example.com")
            .await
            .unwrap()
            .unwrap();
        db::user::deactivate(&pool, &user.id).await.unwrap();

        // Try to login
        let result = execute(&pool, "inactive@example.com", "password123", TEST_JWT_SECRET).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            LoginError::UserNotActive => {}
            _ => panic!("Expected UserNotActive error"),
        }
    }
}
