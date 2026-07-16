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

/// How the caller presented their token. Bearer is the CLI/API path; Cookie
/// is the browser UI path. This distinction drives CSRF handling: only
/// cookie-authenticated requests are a CSRF vector (the browser attaches the
/// cookie automatically), so only they get the same-origin guard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TokenSource {
    Bearer,
    Cookie,
}

/// Extract the presented token from `Authorization: Bearer <token>` or a
/// `litehouse_token` cookie, along with which source it came from. Bearer wins
/// if both are present.
fn extract_token(headers: &axum::http::HeaderMap) -> Option<(String, TokenSource)> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string);
    if let Some(token) = bearer {
        return Some((token, TokenSource::Bearer));
    }
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .map(str::trim)
                .find_map(|kv| kv.strip_prefix("litehouse_token="))
        })
        .map(str::to_string);
    cookie.map(|token| (token, TokenSource::Cookie))
}

/// True when the request's `Origin` (or `Referer`) host matches its `Host`
/// header. Used as a CSRF guard for cookie-authenticated state-changing
/// requests: `SameSite=Lax` does NOT protect against *same-site* origins, and
/// litehouse hosts tenant apps on sibling subdomains of the admin UI, so a
/// malicious deployed app is same-site and would otherwise ride the cookie.
pub(crate) fn same_origin(headers: &axum::http::HeaderMap) -> bool {
    let our_host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if our_host.is_empty() {
        return false;
    }
    let source = headers
        .get(header::ORIGIN)
        .or_else(|| headers.get(header::REFERER))
        .and_then(|h| h.to_str().ok());
    match source
        .and_then(|s| s.split("//").nth(1))
        .map(|rest| rest.split('/').next().unwrap_or(rest))
    {
        Some(host) => host == our_host,
        None => false,
    }
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
    let extracted = extract_token(req.headers());
    let token = extracted.as_ref().map(|(t, _)| t.as_str());
    if !token_authorized(token, &expected) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // CSRF guard: a cookie-authenticated non-GET request must be same-origin.
    // Bearer auth (the CLI) carries no browser Origin and is not a CSRF vector
    // — the token is a secret header a cross-site page cannot attach — so it is
    // exempt. Without this, a malicious tenant app on a sibling subdomain could
    // ride the Lax cookie to fire body-less admin POSTs (e.g. /api/restore).
    let via_cookie = matches!(extracted, Some((_, TokenSource::Cookie)));
    if via_cookie
        && req.method() != axum::http::Method::GET
        && !same_origin(req.headers())
    {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
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
        assert_eq!(extract_token(&h), Some(("tok123".into(), TokenSource::Bearer)));
    }

    #[test]
    fn extract_token_from_cookie() {
        let h = headers(&[("cookie", "theme=dark; litehouse_token=tok456; other=1")]);
        assert_eq!(extract_token(&h), Some(("tok456".into(), TokenSource::Cookie)));
    }

    #[test]
    fn extract_token_bearer_wins_over_cookie() {
        let h = headers(&[
            ("authorization", "Bearer header-tok"),
            ("cookie", "litehouse_token=cookie-tok"),
        ]);
        assert_eq!(
            extract_token(&h),
            Some(("header-tok".into(), TokenSource::Bearer))
        );
    }

    #[test]
    fn extract_token_rejects_malformed() {
        assert_eq!(extract_token(&headers(&[])), None);
        assert_eq!(extract_token(&headers(&[("authorization", "Basic abc")])), None);
        assert_eq!(extract_token(&headers(&[("cookie", "litehouse=nope")])), None);
    }

    #[test]
    fn same_origin_matches_host() {
        let h = headers(&[
            ("host", "admin.lh.danbruder.com"),
            ("origin", "https://admin.lh.danbruder.com"),
        ]);
        assert!(same_origin(&h));
    }

    #[test]
    fn same_origin_rejects_sibling_subdomain() {
        // A tenant app POSTing to the admin host: same-site, so Lax sends the
        // cookie, but the origin host differs — must be rejected.
        let h = headers(&[
            ("host", "admin.lh.danbruder.com"),
            ("origin", "https://evil.lh.danbruder.com"),
        ]);
        assert!(!same_origin(&h));
    }

    #[test]
    fn same_origin_rejects_missing_and_empty() {
        // No Origin/Referer at all (a cross-site form post often omits them).
        assert!(!same_origin(&headers(&[("host", "admin.lh.danbruder.com")])));
        // No Host to compare against.
        assert!(!same_origin(&headers(&[("origin", "https://admin.lh.danbruder.com")])));
    }

    #[test]
    fn same_origin_honors_referer_when_no_origin() {
        let h = headers(&[
            ("host", "admin.lh.danbruder.com"),
            ("referer", "https://admin.lh.danbruder.com/apps"),
        ]);
        assert!(same_origin(&h));
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
