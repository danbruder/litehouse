use serde::{Deserialize, Serialize};

use crate::models::{OrgRole, Organization, User};

/// JWT Claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,  // user_id
    pub email: String,
    pub exp: i64, // Expiration time (as UTC timestamp)
    pub iat: i64, // Issued at (as UTC timestamp)
}

impl Claims {
    pub fn new(user_id: &str, email: &str, expiration_seconds: i64) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            sub: user_id.to_string(),
            email: email.to_string(),
            exp: now + expiration_seconds,
            iat: now,
        }
    }

    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() > self.exp
    }
}

/// Token pair returned during login/registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
}

impl TokenPair {
    pub fn new(access_token: String, refresh_token: String) -> Self {
        Self {
            access_token,
            refresh_token,
            token_type: Some("Bearer".to_string()),
            expires_in: Some(900), // 15 minutes in seconds
        }
    }
}

/// Authenticated user with their organizations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    pub user: User,
    pub organizations: Vec<OrganizationWithRole>,
}

/// Organization with user's role
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationWithRole {
    #[serde(flatten)]
    pub organization: Organization,
    pub role: OrgRole,
}

/// Login request
#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Registration request
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub full_name: Option<String>,
    pub organization_name: Option<String>,
}

/// Refresh token request
#[derive(Debug, Clone, Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

/// Login/registration response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub user: User,
    pub tokens: TokenPair,
    pub organizations: Vec<OrganizationWithRole>,
}

impl AuthResponse {
    pub fn new(user: User, tokens: TokenPair, organizations: Vec<OrganizationWithRole>) -> Self {
        Self {
            user,
            tokens,
            organizations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claims_creation() {
        let claims = Claims::new("user-123", "test@example.com", 900);
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.email, "test@example.com");
        assert!(!claims.is_expired());
    }

    #[test]
    fn test_claims_expiration() {
        let expired_claims = Claims::new("user-123", "test@example.com", -100);
        assert!(expired_claims.is_expired());
    }

    #[test]
    fn test_token_pair() {
        let tokens = TokenPair::new("access".to_string(), "refresh".to_string());
        assert_eq!(tokens.access_token, "access");
        assert_eq!(tokens.refresh_token, "refresh");
        assert_eq!(tokens.token_type, Some("Bearer".to_string()));
        assert_eq!(tokens.expires_in, Some(900));
    }
}
