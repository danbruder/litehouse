use anyhow::{Context, Result};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;

use crate::models::Claims;

/// Default access token expiration: 15 minutes (900 seconds)
pub const DEFAULT_ACCESS_TOKEN_TTL: i64 = 900;

/// Default refresh token expiration: 7 days
pub const DEFAULT_REFRESH_TOKEN_TTL_DAYS: i64 = 7;

/// Generate a JWT access token for a user
pub fn generate_access_token(user_id: &str, email: &str, secret: &str) -> Result<String> {
    generate_access_token_with_ttl(user_id, email, secret, DEFAULT_ACCESS_TOKEN_TTL)
}

/// Generate a JWT access token with custom TTL
pub fn generate_access_token_with_ttl(
    user_id: &str,
    email: &str,
    secret: &str,
    ttl_seconds: i64,
) -> Result<String> {
    let claims = Claims::new(user_id, email, ttl_seconds);
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .context("Failed to generate access token")?;

    Ok(token)
}

/// Generate a cryptographically secure random refresh token
pub fn generate_refresh_token() -> String {
    let random_bytes: [u8; 32] = rand::thread_rng().gen();
    hex::encode(random_bytes)
}

/// Verify and decode a JWT access token
pub fn verify_access_token(token: &str, secret: &str) -> Result<Claims> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .context("Failed to verify access token")?;

    Ok(token_data.claims)
}

/// Get JWT secret from environment or use default (insecure for development only)
pub fn get_jwt_secret() -> String {
    std::env::var("LITEHOUSE_JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("LITEHOUSE_JWT_SECRET not set, using default (INSECURE!)");
        "default-secret-change-me-in-production".to_string()
    })
}

/// Get access token TTL from environment or use default
pub fn get_access_token_ttl() -> i64 {
    std::env::var("LITEHOUSE_ACCESS_TOKEN_TTL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_ACCESS_TOKEN_TTL)
}

/// Get refresh token TTL from environment or use default
pub fn get_refresh_token_ttl_days() -> i64 {
    std::env::var("LITEHOUSE_REFRESH_TOKEN_TTL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_REFRESH_TOKEN_TTL_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SECRET: &str = "test-secret-key-for-testing-only";

    #[test]
    fn test_generate_access_token() {
        let token = generate_access_token("user-123", "test@example.com", TEST_SECRET);
        assert!(token.is_ok());
        let token = token.unwrap();
        assert!(!token.is_empty());
        // JWT tokens have 3 parts separated by dots
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_verify_access_token() {
        let user_id = "user-456";
        let email = "verify@example.com";
        let token = generate_access_token(user_id, email, TEST_SECRET).unwrap();

        let claims = verify_access_token(&token, TEST_SECRET);
        assert!(claims.is_ok());
        let claims = claims.unwrap();
        assert_eq!(claims.sub, user_id);
        assert_eq!(claims.email, email);
        assert!(!claims.is_expired());
    }

    #[test]
    fn test_verify_with_wrong_secret() {
        let token = generate_access_token("user-789", "wrong@example.com", TEST_SECRET).unwrap();
        let result = verify_access_token(&token, "wrong-secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_expired_token() {
        let token = generate_access_token_with_ttl("user-exp", "exp@example.com", TEST_SECRET, -100);
        assert!(token.is_ok());
        let token = token.unwrap();

        // Token should decode but be expired
        let claims = verify_access_token(&token, TEST_SECRET);
        // JWT library will reject expired tokens during verification
        // So this should fail
        assert!(claims.is_err());
    }

    #[test]
    fn test_generate_refresh_token() {
        let token1 = generate_refresh_token();
        let token2 = generate_refresh_token();

        // Tokens should be 64 characters (32 bytes hex encoded)
        assert_eq!(token1.len(), 64);
        assert_eq!(token2.len(), 64);

        // Tokens should be different
        assert_ne!(token1, token2);

        // Tokens should be valid hex
        assert!(hex::decode(&token1).is_ok());
        assert!(hex::decode(&token2).is_ok());
    }

    #[test]
    fn test_custom_ttl() {
        let ttl = 3600; // 1 hour
        let token = generate_access_token_with_ttl("user-ttl", "ttl@example.com", TEST_SECRET, ttl).unwrap();
        let claims = verify_access_token(&token, TEST_SECRET).unwrap();

        let now = chrono::Utc::now().timestamp();
        let expected_exp = now + ttl;

        // Allow 2 second margin for test execution time
        assert!((claims.exp - expected_exp).abs() <= 2);
    }
}
