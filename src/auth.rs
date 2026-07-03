use axum::{
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::commands::server::AppState;

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Extract the presented token from `Authorization: Bearer <token>` or a
/// `litehouse_token` cookie. Bearer wins if both are present.
fn extract_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string);
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .map(str::trim)
                .find_map(|kv| kv.strip_prefix("litehouse_token="))
        })
        .map(str::to_string);
    bearer.or(cookie)
}

/// True when the presented token hashes to the expected (non-empty) hash.
fn token_authorized(provided: Option<&str>, expected_hash: &str) -> bool {
    match provided {
        Some(token) if !expected_hash.is_empty() => {
            constant_time_eq(&hash_token(token), expected_hash)
        }
        _ => false,
    }
}

/// Accepts `Authorization: Bearer <token>` or a `litehouse_token` cookie (for the UI).
pub async fn admin_auth_middleware<B>(
    State(state): State<Arc<RwLock<AppState>>>,
    req: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    let expected = state.read().await.admin_token_hash.clone();
    let provided = extract_token(req.headers());
    if token_authorized(provided.as_deref(), &expected) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_hex_sha256() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn generated_tokens_are_unique_64_hex() {
        let (a, b) = (generate_token(), generate_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    fn headers(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut map = axum::http::HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn extract_token_from_bearer_header() {
        let h = headers(&[("authorization", "Bearer tok123")]);
        assert_eq!(extract_token(&h).as_deref(), Some("tok123"));
    }

    #[test]
    fn extract_token_from_cookie() {
        let h = headers(&[("cookie", "theme=dark; litehouse_token=tok456; other=1")]);
        assert_eq!(extract_token(&h).as_deref(), Some("tok456"));
    }

    #[test]
    fn extract_token_bearer_wins_over_cookie() {
        let h = headers(&[
            ("authorization", "Bearer header-tok"),
            ("cookie", "litehouse_token=cookie-tok"),
        ]);
        assert_eq!(extract_token(&h).as_deref(), Some("header-tok"));
    }

    #[test]
    fn extract_token_rejects_malformed() {
        assert_eq!(extract_token(&headers(&[])), None);
        assert_eq!(extract_token(&headers(&[("authorization", "Basic abc")])), None);
        assert_eq!(extract_token(&headers(&[("cookie", "litehouse=nope")])), None);
    }

    #[test]
    fn token_authorized_semantics() {
        let expected = hash_token("secret");
        assert!(token_authorized(Some("secret"), &expected));
        assert!(!token_authorized(Some("wrong"), &expected));
        assert!(!token_authorized(None, &expected));
        // Empty expected hash must never authorize anything.
        assert!(!token_authorized(Some(""), ""));
        assert!(!token_authorized(Some("anything"), ""));
    }
}
