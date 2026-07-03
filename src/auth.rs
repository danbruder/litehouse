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

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Accepts `Authorization: Bearer <token>` or a `litehouse_token` cookie (for the UI).
pub async fn admin_auth_middleware<B>(
    State(state): State<Arc<RwLock<AppState>>>,
    req: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    let expected = state.read().await.admin_token_hash.clone();
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string);
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .map(str::trim)
                .find_map(|kv| kv.strip_prefix("litehouse_token="))
        })
        .map(str::to_string);
    let provided = bearer.or(cookie).ok_or(StatusCode::UNAUTHORIZED)?;
    if constant_time_eq(&hash_token(&provided), &expected) {
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
}
