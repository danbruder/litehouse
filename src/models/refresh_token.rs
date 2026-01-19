use chrono::Duration;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RefreshToken {
    pub id: String,
    pub user_id: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub expires_at: UtcDateTime,
    pub revoked: bool,
    pub created_at: UtcDateTime,
}

impl RefreshToken {
    /// Create a new refresh token
    /// Note: The token parameter should be the raw token, which will be hashed for storage
    pub fn new(user_id: &str, token: &str, ttl_days: i64) -> Self {
        let now_time = now();
        let expires_at =
            UtcDateTime::from(now_time.0 + Duration::try_days(ttl_days).unwrap_or_default());

        Self {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            token_hash: Self::hash_token(token),
            expires_at,
            revoked: false,
            created_at: now_time,
        }
    }

    /// Hash a token using SHA-256
    pub fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Check if the token is valid (not expired and not revoked)
    pub fn is_valid(&self) -> bool {
        !self.revoked && self.expires_at.0 > chrono::Utc::now()
    }

    /// Revoke this token
    pub fn revoke(&mut self) {
        self.revoked = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_refresh_token() {
        let token = "test-token-123";
        let refresh_token = RefreshToken::new("user-id", token, 7);

        assert_eq!(refresh_token.user_id, "user-id");
        assert_eq!(
            refresh_token.token_hash,
            RefreshToken::hash_token(token)
        );
        assert!(!refresh_token.revoked);
    }

    #[test]
    fn test_hash_token() {
        let token = "my-secret-token";
        let hash1 = RefreshToken::hash_token(token);
        let hash2 = RefreshToken::hash_token(token);

        // Same token should produce same hash
        assert_eq!(hash1, hash2);

        // Different tokens should produce different hashes
        let different_hash = RefreshToken::hash_token("different-token");
        assert_ne!(hash1, different_hash);

        // Hash should be SHA256 (64 hex characters)
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_is_valid() {
        let token = "test-token";
        let mut refresh_token = RefreshToken::new("user-id", token, 7);

        // New token should be valid
        assert!(refresh_token.is_valid());

        // Revoked token should be invalid
        refresh_token.revoke();
        assert!(!refresh_token.is_valid());
    }

    #[test]
    fn test_expired_token() {
        let token = "test-token";
        let mut refresh_token = RefreshToken::new("user-id", token, -1); // Expired yesterday

        // Expired token should be invalid
        assert!(!refresh_token.is_valid());
    }

    #[test]
    fn test_revoke() {
        let token = "test-token";
        let mut refresh_token = RefreshToken::new("user-id", token, 7);

        assert!(!refresh_token.revoked);
        refresh_token.revoke();
        assert!(refresh_token.revoked);
    }
}
