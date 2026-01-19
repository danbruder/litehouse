use anyhow::Result;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// Hash a password using Argon2id
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| anyhow::anyhow!("Password hashing failed: {}", e))
}

/// Verify a password against a hash
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash =
        PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("Invalid password hash: {}", e))?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password() {
        let password = "test_password_123";
        let hash = hash_password(password).unwrap();

        // Hash should start with argon2 identifier
        assert!(hash.starts_with("$argon2"));
        assert!(hash.len() > 50);
    }

    #[test]
    fn test_verify_password_success() {
        let password = "my_secure_password";
        let hash = hash_password(password).unwrap();

        assert!(verify_password(password, &hash).unwrap());
    }

    #[test]
    fn test_verify_password_failure() {
        let password = "correct_password";
        let wrong_password = "wrong_password";
        let hash = hash_password(password).unwrap();

        assert!(!verify_password(wrong_password, &hash).unwrap());
    }

    #[test]
    fn test_different_hashes_for_same_password() {
        let password = "same_password";
        let hash1 = hash_password(password).unwrap();
        let hash2 = hash_password(password).unwrap();

        // Different salts should produce different hashes
        assert_ne!(hash1, hash2);

        // But both should verify successfully
        assert!(verify_password(password, &hash1).unwrap());
        assert!(verify_password(password, &hash2).unwrap());
    }
}
