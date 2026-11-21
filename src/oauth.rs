//! OAuth 2.1 authorization server for the remote MCP endpoint (`POST /mcp`,
//! see `crate::mcp_http`).
//!
//! Hosted Claude clients (claude.ai, Desktop, mobile) only accept remote MCP
//! servers that use OAuth, so litehouse is its own authorization server.
//! There are no users to federate: the identity is "whoever holds the admin
//! token", and the consent page asks for it (or reuses the admin UI cookie).
//! The narrow subset implemented here:
//!
//! - RFC 9728 protected-resource metadata and RFC 8414 authorization-server
//!   metadata (`/.well-known/...`)
//! - RFC 7591 dynamic client registration (`POST /oauth/register`): open, as
//!   the spec requires, but only for Claude's hosted callback or a loopback
//!   redirect, so a registered client can't send codes anywhere else
//! - authorization code + PKCE S256 only, public clients only
//! - 1h access tokens, 30d refresh tokens rotated on every use
//! - two scopes: `mcp` (full) and `mcp:read` (read-only tools)
//!
//! Only sha256 hashes of codes and tokens are stored. Each grant remembers
//! the admin token hash it was approved under, so rotating the admin token
//! revokes every grant. Grants are listed and revoked from the settings page
//! (`/api/mcp/connections`).
//!
//! See `docs/superpowers/specs/2026-11-21-remote-mcp-server-design.md`.

use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::{constant_time_eq, hash_token, same_origin};
use crate::commands::server::AppState;
use crate::ui::HtmlTemplate;

pub const SCOPE_FULL: &str = "mcp";
pub const SCOPE_READ: &str = "mcp:read";

const CODE_TTL_SECS: i64 = 60;
const ACCESS_TTL_SECS: i64 = 3600;
const REFRESH_TTL_SECS: i64 = 30 * 24 * 3600;
const MAX_CLIENTS: i64 = 500;

/// Hosted Claude apps' OAuth callback(s). Claude Code uses a loopback
/// redirect instead, which `redirect_allowed` accepts separately.
const HOSTED_CALLBACKS: &[&str] = &[
    "https://claude.ai/api/mcp/auth_callback",
    "https://claude.com/api/mcp/auth_callback",
];

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn random_token(prefix: &str) -> String {
    format!("{prefix}{}", crate::auth::generate_token())
}

/// The externally visible origin of the admin host, e.g.
/// `https://admin.lh.example.com`. In production it comes from the
/// configured domain (never from the request, so a forged `Host` header
/// can't change the issuer); in local dev from the `Host` header.
pub fn public_base(headers: &HeaderMap, config: &crate::config::ServerConfig) -> String {
    if !crate::ui::is_local_dev()
        && let Some(domain) = &config.domain
    {
        return format!("https://{}.{}", config.admin_label(), domain);
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3030");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");
    format!("{proto}://{host}")
}

/// `{base}/mcp` — the MCP resource URL. Clients compare it exactly against
/// the URL they were given.
pub fn resource_url(base: &str) -> String {
    format!("{base}/mcp")
}

pub fn resource_metadata_url(base: &str) -> String {
    format!("{base}/.well-known/oauth-protected-resource/mcp")
}

/// Split a loopback `http://host[:port]/path` into (host, path).
fn loopback_parts(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = if authority.starts_with('[') {
        authority.split(']').next().map(|h| &authority[..h.len() + 1])?
    } else {
        authority.split(':').next()?
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]").then_some((host, path))
}

/// Redirect URIs a client may register: Claude's hosted callback, or any
/// loopback URI (native clients like Claude Code, RFC 8252).
pub fn redirect_allowed(uri: &str) -> bool {
    HOSTED_CALLBACKS.contains(&uri) || loopback_parts(uri).is_some()
}

/// Does the redirect URI presented at /authorize match a registered one?
/// Exact match, except that loopback ports may differ (RFC 8252 §7.3).
fn redirect_matches(registered: &[String], presented: &str) -> bool {
    registered.iter().any(|r| {
        r == presented
            || matches!(
                (loopback_parts(r), loopback_parts(presented)),
                (Some(a), Some(b)) if a == b
            )
    })
}

fn pkce_s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Normalize a requested scope string to one of ours. Read-only only when
/// asked for explicitly without the full scope.
fn normalize_scope(requested: Option<&str>) -> &'static str {
    let parts: Vec<&str> = requested.unwrap_or("").split_whitespace().collect();
    if parts.contains(&SCOPE_READ) && !parts.contains(&SCOPE_FULL) {
        SCOPE_READ
    } else {
        SCOPE_FULL
    }
}

// ---------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------

struct Client {
    name: String,
    redirect_uris: Vec<String>,
}

async fn get_client(pool: &Pool<Sqlite>, id: &str) -> Option<Client> {
    let row = sqlx::query("SELECT client_name, redirect_uris FROM oauth_client WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()??;
    let uris: String = row.get("redirect_uris");
    Some(Client {
        name: row.get("client_name"),
        redirect_uris: serde_json::from_str(&uris).unwrap_or_default(),
    })
}

/// Who an access token belongs to, once validated.
#[derive(Debug, Clone)]
pub struct Grant {
    pub id: String,
    pub scope: String,
}

impl Grant {
    pub fn read_only(&self) -> bool {
        self.scope == SCOPE_READ
    }
}

/// Look up a presented access token. `None` when unknown, expired, or
/// approved under a different (since rotated) admin token.
pub async fn validate_access_token(
    pool: &Pool<Sqlite>,
    token: &str,
    admin_hash: &str,
) -> Option<Grant> {
    let row = sqlx::query(
        "SELECT id, scope, access_expires_at, admin_hash FROM oauth_token WHERE access_hash = ?",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await
    .ok()??;
    let expires: i64 = row.get("access_expires_at");
    let granted_under: String = row.get("admin_hash");
    if expires <= now() || admin_hash.is_empty() || !constant_time_eq(&granted_under, admin_hash) {
        return None;
    }
    let id: String = row.get("id");
    let _ = sqlx::query("UPDATE oauth_token SET last_used_at = ? WHERE id = ?")
        .bind(now())
        .bind(&id)
        .execute(pool)
        .await;
    Some(Grant {
        id,
        scope: row.get("scope"),
    })
}

struct IssuedTokens {
    access: String,
    refresh: String,
}

async fn prune(pool: &Pool<Sqlite>) {
    let t = now();
    let _ = sqlx::query("DELETE FROM oauth_code WHERE expires_at <= ?")
        .bind(t)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM oauth_token WHERE refresh_expires_at <= ?")
        .bind(t)
        .execute(pool)
        .await;
    // Registrations that never turned into a grant (abandoned or spam).
    let _ = sqlx::query(
        "DELETE FROM oauth_client WHERE created_at <= ? \
         AND id NOT IN (SELECT client_id FROM oauth_token)",
    )
    .bind(t - 24 * 3600)
    .execute(pool)
    .await;
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

#[derive(Template)]
#[template(path = "oauth_error.html")]
struct ErrorTemplate {
    message: String,
}

fn error_page(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        HtmlTemplate(ErrorTemplate {
            message: message.to_string(),
        }),
    )
        .into_response()
}

fn with_query(uri: &str, params: &[(&str, &str)]) -> String {
    let mut out = uri.to_string();
    let mut sep = if uri.contains('?') { '&' } else { '?' };
    for (k, v) in params {
        out.push(sep);
        out.push_str(k);
        out.push('=');
        out.push_str(&urlencoding::encode(v));
        sep = '&';
    }
    out
}

// ---------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------

async fn protected_resource_metadata(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let base = public_base(&headers, &state.read().await.server_config);
    Json(json!({
        "resource": resource_url(&base),
        "authorization_servers": [base],
        "scopes_supported": [SCOPE_FULL, SCOPE_READ],
        "bearer_methods_supported": ["header"],
        "resource_name": "litehouse",
    }))
}

async fn authorization_server_metadata(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let base = public_base(&headers, &state.read().await.server_config);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": [SCOPE_FULL, SCOPE_READ],
    }))
}

// ---------------------------------------------------------------------
// Dynamic client registration
// ---------------------------------------------------------------------

async fn register(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(body): Json<Value>,
) -> Response {
    let pool = state.read().await.db_pool.clone();
    prune(&pool).await;

    let uris: Vec<String> = body
        .get("redirect_uris")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if uris.is_empty() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris is required",
        );
    }
    if let Some(bad) = uris.iter().find(|u| !redirect_allowed(u)) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            &format!("redirect URI not allowed: {bad} (only Claude's hosted callback or a loopback URI)"),
        );
    }
    let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM oauth_client")
        .fetch_one(&pool)
        .await
        .map(|r| r.get("n"))
        .unwrap_or(0);
    if count >= MAX_CLIENTS {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "too many registered clients; try again later",
        );
    }
    let name: String = body
        .get("client_name")
        .and_then(|v| v.as_str())
        .unwrap_or("MCP client")
        .chars()
        .take(100)
        .collect();
    let id = uuid::Uuid::new_v4().to_string();
    let created = now();
    let stored = sqlx::query(
        "INSERT INTO oauth_client (id, client_name, redirect_uris, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(serde_json::to_string(&uris).unwrap_or_default())
    .bind(created)
    .execute(&pool)
    .await;
    if let Err(e) = stored {
        tracing::error!("oauth client registration failed: {e}");
        return oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "registration failed");
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": id,
            "client_id_issued_at": created,
            "client_name": name,
            "redirect_uris": uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------
// Authorization (consent page)
// ---------------------------------------------------------------------

#[derive(Template)]
#[template(path = "oauth_authorize.html")]
struct AuthorizeTemplate {
    client_name: String,
    client_id: String,
    redirect_uri: String,
    state: String,
    code_challenge: String,
    resource: String,
    read_only: bool,
    signed_in: bool,
    error: Option<String>,
}

fn param<'a>(p: &'a HashMap<String, String>, k: &str) -> &'a str {
    p.get(k).map(String::as_str).unwrap_or("")
}

/// Validate the client and redirect URI. Failures here must NOT redirect
/// (the redirect target is the thing that's untrusted), so they render an
/// error page instead.
async fn checked_client(
    pool: &Pool<Sqlite>,
    p: &HashMap<String, String>,
) -> Result<Client, Response> {
    let Some(client) = get_client(pool, param(p, "client_id")).await else {
        return Err(error_page(
            "Unknown client. Remove the connector and add it again.",
        ));
    };
    if !redirect_matches(&client.redirect_uris, param(p, "redirect_uri")) {
        return Err(error_page("The redirect URI doesn't match this client's registration."));
    }
    Ok(client)
}

/// Errors after the client is validated go back to the client.
fn redirect_error(p: &HashMap<String, String>, error: &str, description: &str) -> Response {
    let mut params = vec![("error", error), ("error_description", description)];
    if !param(p, "state").is_empty() {
        params.push(("state", param(p, "state")));
    }
    Redirect::to(&with_query(param(p, "redirect_uri"), &params)).into_response()
}

fn signed_in(headers: &HeaderMap, admin_hash: &str) -> bool {
    crate::ui::cookie_token(headers)
        .map(|t| crate::auth::token_authorized(Some(&t), admin_hash))
        .unwrap_or(false)
}

async fn authorize_page(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: HeaderMap,
    Query(p): Query<HashMap<String, String>>,
) -> Response {
    let (pool, admin_hash) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.admin_token_hash.clone())
    };
    let client = match checked_client(&pool, &p).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if param(&p, "response_type") != "code" {
        return redirect_error(&p, "unsupported_response_type", "only response_type=code is supported");
    }
    if param(&p, "code_challenge").is_empty() || param(&p, "code_challenge_method") != "S256" {
        return redirect_error(&p, "invalid_request", "PKCE with code_challenge_method=S256 is required");
    }
    HtmlTemplate(AuthorizeTemplate {
        client_name: client.name,
        client_id: param(&p, "client_id").to_string(),
        redirect_uri: param(&p, "redirect_uri").to_string(),
        state: param(&p, "state").to_string(),
        code_challenge: param(&p, "code_challenge").to_string(),
        resource: param(&p, "resource").to_string(),
        read_only: normalize_scope(p.get("scope").map(String::as_str)) == SCOPE_READ,
        signed_in: signed_in(&headers, &admin_hash),
        error: None,
    })
    .into_response()
}

async fn authorize_submit(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: HeaderMap,
    Form(p): Form<HashMap<String, String>>,
) -> Response {
    // Same CSRF guard as the admin API: tenant apps are same-site siblings.
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin request rejected").into_response();
    }
    let (pool, admin_hash) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.admin_token_hash.clone())
    };
    let client = match checked_client(&pool, &p).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if param(&p, "code_challenge").is_empty() {
        return redirect_error(&p, "invalid_request", "PKCE code_challenge is required");
    }
    if param(&p, "decision") != "allow" {
        return redirect_error(&p, "access_denied", "the admin denied access");
    }

    let has_session = signed_in(&headers, &admin_hash);
    let token_ok = has_session
        || crate::auth::token_authorized(Some(param(&p, "token").trim()), &admin_hash);
    if !token_ok {
        // Slow down guessing. (The admin token is 256 random bits, so this
        // is belt and braces.)
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        return HtmlTemplate(AuthorizeTemplate {
            client_name: client.name,
            client_id: param(&p, "client_id").to_string(),
            redirect_uri: param(&p, "redirect_uri").to_string(),
            state: param(&p, "state").to_string(),
            code_challenge: param(&p, "code_challenge").to_string(),
            resource: param(&p, "resource").to_string(),
            read_only: param(&p, "access") == "read",
            signed_in: false,
            error: Some("Invalid admin token".to_string()),
        })
        .into_response();
    }

    let scope = if param(&p, "access") == "read" { SCOPE_READ } else { SCOPE_FULL };
    let code = random_token("lhc_");
    let stored = sqlx::query(
        "INSERT INTO oauth_code (code_hash, client_id, redirect_uri, code_challenge, scope, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(hash_token(&code))
    .bind(param(&p, "client_id"))
    .bind(param(&p, "redirect_uri"))
    .bind(param(&p, "code_challenge"))
    .bind(scope)
    .bind(now() + CODE_TTL_SECS)
    .execute(&pool)
    .await;
    if let Err(e) = stored {
        tracing::error!("storing oauth code failed: {e}");
        return redirect_error(&p, "server_error", "could not issue a code");
    }
    tracing::info!(client = %client.name, scope, "mcp oauth: access approved");

    let base = public_base(&headers, &state.read().await.server_config);
    let mut params = vec![("code", code.as_str()), ("iss", base.as_str())];
    if !param(&p, "state").is_empty() {
        params.push(("state", param(&p, "state")));
    }
    Redirect::to(&with_query(param(&p, "redirect_uri"), &params)).into_response()
}

// ---------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------

fn token_response(tokens: &IssuedTokens, scope: &str) -> Response {
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")],
        Json(json!({
            "access_token": tokens.access,
            "token_type": "Bearer",
            "expires_in": ACCESS_TTL_SECS,
            "refresh_token": tokens.refresh,
            "scope": scope,
        })),
    )
        .into_response()
}

async fn token(
    State(state): State<Arc<RwLock<AppState>>>,
    Form(p): Form<HashMap<String, String>>,
) -> Response {
    let (pool, admin_hash) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.admin_token_hash.clone())
    };
    match param(&p, "grant_type") {
        "authorization_code" => exchange_code(&pool, &admin_hash, &p).await,
        "refresh_token" => refresh(&pool, &admin_hash, &p).await,
        _ => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "use authorization_code or refresh_token",
        ),
    }
}

async fn exchange_code(
    pool: &Pool<Sqlite>,
    admin_hash: &str,
    p: &HashMap<String, String>,
) -> Response {
    let invalid = |why: &str| oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", why);
    let code_hash = hash_token(param(p, "code"));
    // Single use: delete it whatever happens next.
    let row = sqlx::query(
        "DELETE FROM oauth_code WHERE code_hash = ? \
         RETURNING client_id, redirect_uri, code_challenge, scope, expires_at",
    )
    .bind(&code_hash)
    .fetch_optional(pool)
    .await;
    let Ok(Some(row)) = row else {
        return invalid("unknown or already used code");
    };
    let client_id: String = row.get("client_id");
    let redirect_uri: String = row.get("redirect_uri");
    let challenge: String = row.get("code_challenge");
    let scope: String = row.get("scope");
    let expires: i64 = row.get("expires_at");
    if expires <= now() {
        return invalid("code expired");
    }
    if client_id != param(p, "client_id") {
        return invalid("code was issued to a different client");
    }
    if !p.get("redirect_uri").is_none_or(|r| r == &redirect_uri) {
        return invalid("redirect_uri mismatch");
    }
    let verifier = param(p, "code_verifier");
    if verifier.is_empty() || !constant_time_eq(&pkce_s256(verifier), &challenge) {
        return invalid("PKCE verification failed");
    }

    let tokens = IssuedTokens {
        access: random_token("lho_"),
        refresh: random_token("lhr_"),
    };
    let t = now();
    let stored = sqlx::query(
        "INSERT INTO oauth_token (id, client_id, scope, access_hash, access_expires_at, \
         refresh_hash, refresh_expires_at, admin_hash, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&client_id)
    .bind(&scope)
    .bind(hash_token(&tokens.access))
    .bind(t + ACCESS_TTL_SECS)
    .bind(hash_token(&tokens.refresh))
    .bind(t + REFRESH_TTL_SECS)
    .bind(admin_hash)
    .bind(t)
    .execute(pool)
    .await;
    if let Err(e) = stored {
        tracing::error!("storing oauth token failed: {e}");
        return oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "could not issue tokens");
    }
    prune(pool).await;
    token_response(&tokens, &scope)
}

async fn refresh(pool: &Pool<Sqlite>, admin_hash: &str, p: &HashMap<String, String>) -> Response {
    let invalid = || oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "refresh token is invalid or expired");
    let row = sqlx::query(
        "SELECT id, client_id, scope, refresh_expires_at, admin_hash FROM oauth_token WHERE refresh_hash = ?",
    )
    .bind(hash_token(param(p, "refresh_token")))
    .fetch_optional(pool)
    .await;
    let Ok(Some(row)) = row else {
        return invalid();
    };
    let id: String = row.get("id");
    let client_id: String = row.get("client_id");
    let scope: String = row.get("scope");
    let expires: i64 = row.get("refresh_expires_at");
    let granted_under: String = row.get("admin_hash");
    if expires <= now()
        || client_id != param(p, "client_id")
        || admin_hash.is_empty()
        || !constant_time_eq(&granted_under, admin_hash)
    {
        return invalid();
    }
    let tokens = IssuedTokens {
        access: random_token("lho_"),
        refresh: random_token("lhr_"),
    };
    let t = now();
    let updated = sqlx::query(
        "UPDATE oauth_token SET access_hash = ?, access_expires_at = ?, refresh_hash = ?, \
         refresh_expires_at = ? WHERE id = ?",
    )
    .bind(hash_token(&tokens.access))
    .bind(t + ACCESS_TTL_SECS)
    .bind(hash_token(&tokens.refresh))
    .bind(t + REFRESH_TTL_SECS)
    .bind(&id)
    .execute(pool)
    .await;
    if let Err(e) = updated {
        tracing::error!("rotating oauth token failed: {e}");
        return oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "could not issue tokens");
    }
    token_response(&tokens, &scope)
}

// ---------------------------------------------------------------------
// Connections (admin API, for the settings page)
// ---------------------------------------------------------------------

/// `GET /api/mcp/connections` — active OAuth grants.
pub async fn list_connections(State(state): State<Arc<RwLock<AppState>>>) -> Response {
    let pool = state.read().await.db_pool.clone();
    let rows = sqlx::query(
        "SELECT t.id, COALESCE(c.client_name, 'unknown client') AS client_name, t.scope, \
         t.created_at, t.last_used_at FROM oauth_token t \
         LEFT JOIN oauth_client c ON c.id = t.client_id \
         WHERE t.refresh_expires_at > ? ORDER BY t.created_at DESC",
    )
    .bind(now())
    .fetch_all(&pool)
    .await;
    match rows {
        Ok(rows) => {
            let ts = |v: Option<i64>| {
                v.and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                    .map(|d| d.to_rfc3339())
            };
            let out: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.get::<String, _>("id"),
                        "client_name": r.get::<String, _>("client_name"),
                        "scope": r.get::<String, _>("scope"),
                        "read_only": r.get::<String, _>("scope") == SCOPE_READ,
                        "created_at": ts(Some(r.get::<i64, _>("created_at"))),
                        "last_used_at": ts(r.get::<Option<i64>, _>("last_used_at")),
                    })
                })
                .collect();
            Json(out).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `DELETE /api/mcp/connections/:id` — revoke a grant.
pub async fn revoke_connection(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(id): Path<String>,
) -> Response {
    let pool = state.read().await.db_pool.clone();
    match sqlx::query("DELETE FROM oauth_token WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => (StatusCode::NOT_FOUND, "no such connection").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------

/// Public OAuth routes, mounted at the server root (the well-known paths
/// can't live under `/api`).
pub fn router(state: Arc<RwLock<AppState>>) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server_metadata),
        )
        .route("/oauth/register", post(register))
        .route("/oauth/authorize", get(authorize_page).post(authorize_submit))
        .route("/oauth/token", post(token))
        .with_state(state)
}

#[cfg(test)]
pub(crate) mod tests_support {
    pub fn pkce(verifier: &str) -> String {
        super::pkce_s256(verifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_allowlist() {
        assert!(redirect_allowed("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_allowed("http://localhost:53122/callback"));
        assert!(redirect_allowed("http://127.0.0.1/callback"));
        assert!(redirect_allowed("http://[::1]:8080/cb"));
        assert!(!redirect_allowed("https://evil.example/callback"));
        assert!(!redirect_allowed("http://localhost.evil.example/callback"));
        assert!(!redirect_allowed("https://claude.ai/other"));
    }

    #[test]
    fn loopback_port_is_ignored_when_matching() {
        let reg = vec!["http://localhost/callback".to_string()];
        assert!(redirect_matches(&reg, "http://localhost:4567/callback"));
        assert!(!redirect_matches(&reg, "http://localhost:4567/other"));
        assert!(!redirect_matches(&reg, "http://127.0.0.1:4567/callback"));
        let hosted = vec!["https://claude.ai/api/mcp/auth_callback".to_string()];
        assert!(redirect_matches(&hosted, "https://claude.ai/api/mcp/auth_callback"));
        assert!(!redirect_matches(&hosted, "https://claude.ai/api/mcp/auth_callback?x=1"));
    }

    #[test]
    fn pkce_is_base64url_sha256() {
        assert_eq!(
            pkce_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r7wW1gFWFOEjXk"),
            "bwWFMyPfdG9qreDhH2lmftFx_dFeLDalzcT1gb_j68g"
        );
    }

    #[test]
    fn scope_defaults_to_full() {
        assert_eq!(normalize_scope(None), SCOPE_FULL);
        assert_eq!(normalize_scope(Some("mcp")), SCOPE_FULL);
        assert_eq!(normalize_scope(Some("mcp:read")), SCOPE_READ);
        assert_eq!(normalize_scope(Some("mcp mcp:read")), SCOPE_FULL);
    }

    #[test]
    fn with_query_appends_and_encodes() {
        assert_eq!(
            with_query("https://x/cb", &[("code", "a b"), ("state", "s&1")]),
            "https://x/cb?code=a%20b&state=s%261"
        );
        assert_eq!(with_query("https://x/cb?k=v", &[("code", "c")]), "https://x/cb?k=v&code=c");
    }
}
