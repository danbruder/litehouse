use anyhow::Result;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::models::{now, UtcDateTime};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub full_name: Option<String>,
    pub is_active: bool,
    pub email_verified: bool,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
}

#[derive(Debug, thiserror::Error)]
pub enum UserError {
    #[error("Invalid email address: {0}")]
    InvalidEmail(String),
    #[error("Password hashing failed: {0}")]
    HashError(String),
    #[error("Password verification failed")]
    VerificationError,
    #[error("Password too short (minimum 8 characters)")]
    PasswordTooShort,
}

impl User {
    /// Create a new user with hashed password
    pub fn new(email: &str, password: &str, full_name: Option<String>) -> Result<Self, UserError> {
        if !is_valid_email(email) {
            return Err(UserError::InvalidEmail(email.to_string()));
        }

        if password.len() < 8 {
            return Err(UserError::PasswordTooShort);
        }

        let password_hash = Self::hash_password(password)?;
        let now = now();

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            email: email.to_lowercase(),
            password_hash,
            full_name,
            is_active: true,
            email_verified: false,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// Hash a password using Argon2
    pub fn hash_password(password: &str) -> Result<String, UserError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| UserError::HashError(e.to_string()))
    }

    /// Verify a password against the stored hash
    pub fn verify_password(&self, password: &str) -> Result<bool, UserError> {
        let parsed_hash = PasswordHash::new(&self.password_hash)
            .map_err(|e| UserError::HashError(e.to_string()))?;

        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    /// Update the user's password
    pub fn update_password(&mut self, new_password: &str) -> Result<(), UserError> {
        if new_password.len() < 8 {
            return Err(UserError::PasswordTooShort);
        }

        self.password_hash = Self::hash_password(new_password)?;
        self.updated_at = now();
        Ok(())
    }
}

/// Simple email validation
fn is_valid_email(email: &str) -> bool {
    email.contains('@') && email.len() >= 3 && email.len() <= 254
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_user() {
        let user = User::new("test@example.com", "password123", Some("Test User".to_string()));
        assert!(user.is_ok());
        let user = user.unwrap();
        assert_eq!(user.email, "test@example.com");
        assert_eq!(user.full_name, Some("Test User".to_string()));
        assert!(user.is_active);
        assert!(!user.email_verified);
    }

    #[test]
    fn test_email_lowercase() {
        let user = User::new("Test@Example.COM", "password123", None).unwrap();
        assert_eq!(user.email, "test@example.com");
    }

    #[test]
    fn test_invalid_email() {
        let result = User::new("invalid-email", "password123", None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UserError::InvalidEmail(_)));
    }

    #[test]
    fn test_password_too_short() {
        let result = User::new("test@example.com", "short", None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UserError::PasswordTooShort));
    }

    #[test]
    fn test_password_hashing() {
        let hash = User::hash_password("testpassword123");
        assert!(hash.is_ok());
        let hash = hash.unwrap();
        assert!(hash.starts_with("$argon2"));
    }

    #[test]
    fn test_password_verification() {
        let user = User::new("test@example.com", "password123", None).unwrap();
        assert!(user.verify_password("password123").unwrap());
        assert!(!user.verify_password("wrongpassword").unwrap());
    }

    #[test]
    fn test_update_password() {
        let mut user = User::new("test@example.com", "password123", None).unwrap();
        let old_hash = user.password_hash.clone();

        user.update_password("newpassword456").unwrap();
        assert_ne!(user.password_hash, old_hash);
        assert!(user.verify_password("newpassword456").unwrap());
        assert!(!user.verify_password("password123").unwrap());
    }
}
