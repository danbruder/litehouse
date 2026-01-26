use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("Invalid signature format")]
    InvalidFormat,
    #[error("Invalid secret")]
    InvalidSecret,
    #[error("Signature mismatch")]
    SignatureMismatch,
}

/// Verify GitHub webhook signature using HMAC-SHA256
///
/// GitHub sends the signature in the X-Hub-Signature-256 header as "sha256=<hex_signature>"
/// We compute HMAC-SHA256 of the payload with the secret and compare
pub fn verify_github_signature(
    payload: &[u8],
    signature_header: &str,
    secret: &str,
) -> Result<bool, VerificationError> {
    // GitHub signature format: "sha256=<hex_encoded_signature>"
    let signature = signature_header
        .strip_prefix("sha256=")
        .ok_or(VerificationError::InvalidFormat)?;

    // Create HMAC instance with the secret
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| VerificationError::InvalidSecret)?;

    // Compute HMAC of the payload
    mac.update(payload);
    let expected = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison to prevent timing attacks
    if constant_time_compare(&expected, signature) {
        Ok(true)
    } else {
        Err(VerificationError::SignatureMismatch)
    }
}

/// Constant-time string comparison to prevent timing attacks
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    let mut result = 0u8;
    for i in 0..a_bytes.len() {
        result |= a_bytes[i] ^ b_bytes[i];
    }

    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_valid_signature() {
        let payload = b"test payload";
        let secret = "my-secret";

        // Generate a valid signature
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());
        let signature_header = format!("sha256={}", signature);

        assert!(verify_github_signature(payload, &signature_header, secret).unwrap());
    }

    #[test]
    fn test_verify_invalid_signature() {
        let payload = b"test payload";
        let secret = "my-secret";
        let signature_header = "sha256=invalid_signature";

        assert!(verify_github_signature(payload, signature_header, secret).is_err());
    }

    #[test]
    fn test_verify_missing_prefix() {
        let payload = b"test payload";
        let secret = "my-secret";
        let signature_header = "invalid_format";

        assert!(matches!(
            verify_github_signature(payload, signature_header, secret),
            Err(VerificationError::InvalidFormat)
        ));
    }

    #[test]
    fn test_constant_time_compare() {
        assert!(constant_time_compare("hello", "hello"));
        assert!(!constant_time_compare("hello", "world"));
        assert!(!constant_time_compare("hello", "hello2"));
        assert!(!constant_time_compare("hello", "hell"));
    }
}
