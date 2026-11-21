//! Remote MCP endpoint: Streamable HTTP transport at `POST /mcp` on the admin
//! host, for clients that can't launch `lh mcp serve` locally (claude.ai,
//! Claude Desktop and mobile as a custom connector; Claude Code in the cloud).
//!
//! Auth, either of:
//! - `Authorization: Bearer <admin token>` (Claude Code `--header`), or
//! - an OAuth access token issued by `crate::oauth` (hosted Claude apps).
//!
//! Anything else gets a `401` with `WWW-Authenticate: Bearer
//! resource_metadata=...`, which is how clients discover the OAuth flow.
//!
//! Tool calls reuse `crate::mcp`'s handling and `ApiClient` unchanged: the
//! client points at this same server over loopback, authenticated with the
//! per-process [`crate::auth::internal_token`]. So every validation, app lock
//! and side effect stays in the `/api` handlers, and the admin token never has
//! to exist in plaintext on the server.
//!
//! Responses are plain `application/json` (no SSE): litehouse never sends
//! server-initiated messages, and the spec lets a server answer a POST with a
//! single JSON body.

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api_client::ApiClient;
use crate::commands::server::AppState;
use crate::config::ClientConfig;
use crate::mcp::Mode;

/// Base URL (including `/api`, like a client config's `base_url`) the
/// server's own API client uses to reach the admin API over loopback.
fn loopback_base(host: &str, port: u16) -> String {
    let host = match host {
        "0.0.0.0" | "" => "127.0.0.1",
        "::" | "[::]" => "[::1]",
        h => h,
    };
    format!("http://{host}:{port}/api")
}

fn unauthorized(base: &str, invalid_token: bool) -> Response {
    let mut challenge = format!(
        "Bearer resource_metadata=\"{}\"",
        crate::oauth::resource_metadata_url(base)
    );
    if invalid_token {
        challenge.push_str(", error=\"invalid_token\"");
    }
    let mut resp = (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    if let Ok(v) = HeaderValue::from_str(&challenge) {
        resp.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    resp
}

/// The spec requires validating `Origin` (DNS rebinding). Server-to-server
/// callers (claude.ai's backend, Claude Code) send none; a browser page may
/// only call from the admin host itself or Claude's own sites.
fn origin_allowed(headers: &HeaderMap, base: &str) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) else {
        return true;
    };
    origin == base
        || origin == "https://claude.ai"
        || origin == "https://claude.com"
        || crate::auth::same_origin(headers)
}

/// `POST /mcp`
pub async fn mcp_post(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (pool, admin_hash, server_config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.admin_token_hash.clone(), s.server_config.clone())
    };
    let base = crate::oauth::public_base(&headers, &server_config);

    if !origin_allowed(&headers, &base) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }

    let Some(token) = crate::auth::bearer_token(&headers) else {
        return unauthorized(&base, false);
    };
    let mode = if crate::auth::token_authorized(Some(&token), &admin_hash) {
        Mode { remote: true, read_only: false }
    } else if let Some(grant) =
        crate::oauth::validate_access_token(&pool, &token, &admin_hash).await
    {
        Mode { remote: true, read_only: grant.read_only() }
    } else {
        return unauthorized(&base, true);
    };

    let message: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let err = serde_json::json!({
                "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("Parse error: {e}") }
            });
            return (StatusCode::BAD_REQUEST, axum::Json(err)).into_response();
        }
    };

    let config = ClientConfig {
        base_url: loopback_base(&server_config.host, server_config.port),
        api_token: Some(crate::auth::internal_token().to_string()),
        github_token: None,
    };
    let api = ApiClient::new_loopback(config.clone());

    let is_initialize = |m: &Value| m.get("method").and_then(|v| v.as_str()) == Some("initialize");
    let (response, initialize) = match message {
        Value::Array(batch) => {
            let init = batch.iter().any(is_initialize);
            let mut out = Vec::new();
            for m in batch {
                if let Some(r) = crate::mcp::handle_message(m, &api, &config, mode).await {
                    out.push(r);
                }
            }
            ((!out.is_empty()).then_some(Value::Array(out)), init)
        }
        m => {
            let init = is_initialize(&m);
            (crate::mcp::handle_message(m, &api, &config, mode).await, init)
        }
    };

    let Some(response) = response else {
        // Only notifications/responses: acknowledge without a body.
        return StatusCode::ACCEPTED.into_response();
    };
    let mut resp = axum::Json(response).into_response();
    if initialize {
        // Stateless: the id is issued because clients expect one, but no
        // per-session state hangs off it.
        if let Ok(v) = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()) {
            resp.headers_mut().insert("mcp-session-id", v);
        }
    }
    resp
}

/// `/mcp` plus the OAuth routes that guard it, mounted at the server root.
pub fn router(state: Arc<RwLock<AppState>>) -> axum::Router {
    axum::Router::new()
        .route(
            "/mcp",
            axum::routing::post(mcp_post)
                .get(mcp_method_not_allowed)
                .delete(mcp_method_not_allowed),
        )
        .with_state(state.clone())
        .merge(crate::oauth::router(state))
}

/// `GET`/`DELETE /mcp`: no server-to-client stream and no session state.
pub async fn mcp_method_not_allowed() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        "this MCP endpoint only accepts POST",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use serde_json::json;

    const ADMIN: &str = "test-admin-token";
    const CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";

    /// `:memory:` is per connection, so pin the pool to one connection or
    /// requests may land on an empty database.
    async fn one_connection_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// A real server on a random loopback port (the MCP endpoint calls `/api`
    /// over loopback, so in-process `oneshot` isn't enough).
    async fn spawn_server() -> (String, Arc<RwLock<AppState>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port,
            ..ServerConfig::default()
        };
        let state = Arc::new(RwLock::new(AppState {
            db_pool: one_connection_pool().await,
            docker: crate::docker::connect().await.expect("docker client"),
            admin_token_hash: crate::auth::hash_token(ADMIN),
            server_config: config,
            app_locks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }));
        let app = axum::Router::new()
            .nest("/api", crate::api::create_api_router(state.clone()))
            .merge(router(state.clone()));
        tokio::spawn(async move {
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        (format!("http://127.0.0.1:{port}"), state)
    }

    fn no_redirects() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    async fn rpc(base: &str, token: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{base}/mcp"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    /// Register, approve on the consent page, and exchange the code. Returns
    /// the token response.
    async fn oauth_flow(base: &str, access: &str) -> Value {
        let http = no_redirects();
        let reg: Value = http
            .post(format!("{base}/oauth/register"))
            .json(&json!({ "client_name": "Claude", "redirect_uris": [CALLBACK] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let client_id = reg["client_id"].as_str().unwrap().to_string();

        let verifier = "a-very-long-pkce-verifier-0123456789-abcdefghijklmnop";
        let challenge = crate::oauth::tests_support::pkce(verifier);
        let page = http
            .get(format!("{base}/oauth/authorize"))
            .query(&[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", CALLBACK),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("state", "xyz"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), 200);
        assert!(page.text().await.unwrap().contains("Claude"));

        let host = base.trim_start_matches("http://");
        let approved = http
            .post(format!("{base}/oauth/authorize"))
            .header("origin", base)
            .header("host", host)
            .form(&[
                ("client_id", client_id.as_str()),
                ("redirect_uri", CALLBACK),
                ("code_challenge", &challenge),
                ("state", "xyz"),
                ("access", access),
                ("token", ADMIN),
                ("decision", "allow"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(approved.status(), 303);
        let location = approved.headers()["location"].to_str().unwrap().to_string();
        assert!(location.starts_with(CALLBACK), "{location}");
        assert!(location.contains("state=xyz"));
        let code = location
            .split(['?', '&'])
            .find_map(|kv| kv.strip_prefix("code="))
            .unwrap()
            .to_string();

        // Wrong verifier fails and burns the code.
        let bad = http
            .post(format!("{base}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("client_id", &client_id),
                ("redirect_uri", CALLBACK),
                ("code_verifier", "wrong"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 400);
        let body: Value = bad.json().await.unwrap();
        assert_eq!(body["error"], "invalid_grant");
        oauth_flow_second_code(base, &http, &client_id, &challenge, verifier, access).await
    }

    /// The first code was burned by the failed exchange; approve again and
    /// exchange properly.
    async fn oauth_flow_second_code(
        base: &str,
        http: &reqwest::Client,
        client_id: &str,
        challenge: &str,
        verifier: &str,
        access: &str,
    ) -> Value {
        let host = base.trim_start_matches("http://");
        let approved = http
            .post(format!("{base}/oauth/authorize"))
            .header("origin", base)
            .header("host", host)
            .form(&[
                ("client_id", client_id),
                ("redirect_uri", CALLBACK),
                ("code_challenge", challenge),
                ("access", access),
                ("token", ADMIN),
                ("decision", "allow"),
            ])
            .send()
            .await
            .unwrap();
        let location = approved.headers()["location"].to_str().unwrap().to_string();
        let code = location
            .split(['?', '&'])
            .find_map(|kv| kv.strip_prefix("code="))
            .unwrap()
            .to_string();
        let resp = http
            .post(format!("{base}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("client_id", client_id),
                ("redirect_uri", CALLBACK),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let mut tokens: Value = resp.json().await.unwrap();
        tokens["client_id"] = json!(client_id);
        tokens
    }

    #[tokio::test]
    async fn unauthenticated_gets_401_with_resource_metadata() {
        let (base, _) = spawn_server().await;
        let resp = reqwest::Client::new()
            .post(format!("{base}/mcp"))
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        let www = resp.headers()["www-authenticate"].to_str().unwrap().to_string();
        assert!(www.contains("/.well-known/oauth-protected-resource/mcp"), "{www}");

        let meta: Value = reqwest::get(format!("{base}/.well-known/oauth-protected-resource/mcp"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(meta["resource"], format!("{base}/mcp"));
        let asm: Value = reqwest::get(format!("{base}/.well-known/oauth-authorization-server"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(asm["token_endpoint"], format!("{base}/oauth/token"));
    }

    #[tokio::test]
    async fn admin_token_works_and_tools_reach_the_api_over_loopback() {
        let (base, _) = spawn_server().await;
        let init = rpc(&base, ADMIN, json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-06-18"}})).await;
        assert_eq!(init.status(), 200);
        assert!(init.headers().contains_key("mcp-session-id"));
        let init: Value = init.json().await.unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

        let notif = rpc(&base, ADMIN, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
        assert_eq!(notif.status(), 202);

        let list: Value = rpc(&base, ADMIN, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .await.json().await.unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter()
            .map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"deploy"));
        assert!(!names.contains(&"create_app"), "create_app is local-only");

        // backup_status goes MCP -> ApiClient -> loopback /api with the
        // internal token -> DB.
        let call: Value = rpc(&base, ADMIN, json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"backup_status","arguments":{}}})).await.json().await.unwrap();
        assert_eq!(call["result"]["isError"], false, "{call}");
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("last_backup_date"));

        let wrong = rpc(&base, "nope", json!({"jsonrpc":"2.0","id":4,"method":"tools/list"})).await;
        assert_eq!(wrong.status(), 401);
    }

    #[tokio::test]
    async fn oauth_read_only_grant_hides_write_tools_and_can_be_revoked() {
        let (base, _) = spawn_server().await;
        let tokens = oauth_flow(&base, "read").await;
        assert_eq!(tokens["scope"], "mcp:read");
        let access = tokens["access_token"].as_str().unwrap();

        let list: Value = rpc(&base, access, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .await.json().await.unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter()
            .map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_apps"));
        assert!(!names.contains(&"delete_app"));
        let denied: Value = rpc(&base, access, json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"delete_app","arguments":{"app_name":"x"}}})).await.json().await.unwrap();
        assert_eq!(denied["result"]["isError"], true);

        // Refresh rotates: the old refresh token stops working.
        let http = reqwest::Client::new();
        let refresh = |rt: String| {
            let http = http.clone();
            let base = base.clone();
            let client_id = tokens["client_id"].as_str().unwrap().to_string();
            async move {
                http.post(format!("{base}/oauth/token"))
                    .form(&[("grant_type", "refresh_token"), ("refresh_token", rt.as_str()),
                        ("client_id", client_id.as_str())])
                    .send().await.unwrap()
            }
        };
        let old_rt = tokens["refresh_token"].as_str().unwrap().to_string();
        let r1 = refresh(old_rt.clone()).await;
        assert_eq!(r1.status(), 200);
        let r1: Value = r1.json().await.unwrap();
        assert_eq!(refresh(old_rt).await.status(), 400);
        let new_access = r1["access_token"].as_str().unwrap().to_string();
        assert_eq!(rpc(&base, access, json!({"jsonrpc":"2.0","id":3,"method":"ping"})).await.status(), 401);
        assert_eq!(rpc(&base, &new_access, json!({"jsonrpc":"2.0","id":3,"method":"ping"})).await.status(), 200);

        // Listed in the admin API and revocable.
        let conns: Value = http.get(format!("{base}/api/mcp/connections")).bearer_auth(ADMIN)
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(conns.as_array().unwrap().len(), 1);
        assert_eq!(conns[0]["client_name"], "Claude");
        let id = conns[0]["id"].as_str().unwrap();
        let del = http.delete(format!("{base}/api/mcp/connections/{id}")).bearer_auth(ADMIN)
            .send().await.unwrap();
        assert_eq!(del.status(), 204);
        assert_eq!(rpc(&base, &new_access, json!({"jsonrpc":"2.0","id":4,"method":"ping"})).await.status(), 401);
    }

    #[tokio::test]
    async fn rotating_the_admin_token_revokes_grants() {
        let (base, state) = spawn_server().await;
        let tokens = oauth_flow(&base, "full").await;
        let access = tokens["access_token"].as_str().unwrap();
        assert_eq!(rpc(&base, access, json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.status(), 200);
        state.write().await.admin_token_hash = crate::auth::hash_token("rotated");
        assert_eq!(rpc(&base, access, json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.status(), 401);
    }

    #[tokio::test]
    async fn registration_rejects_foreign_redirects_and_consent_needs_same_origin() {
        let (base, _) = spawn_server().await;
        let http = no_redirects();
        let bad = http.post(format!("{base}/oauth/register"))
            .json(&json!({"redirect_uris": ["https://evil.example/cb"]}))
            .send().await.unwrap();
        assert_eq!(bad.status(), 400);

        let reg: Value = http.post(format!("{base}/oauth/register"))
            .json(&json!({"redirect_uris": ["http://localhost/callback"]}))
            .send().await.unwrap().json().await.unwrap();
        let client_id = reg["client_id"].as_str().unwrap();
        let cross = http.post(format!("{base}/oauth/authorize"))
            .header("origin", "https://evil.example")
            .form(&[("client_id", client_id), ("redirect_uri", "http://localhost:9999/callback"),
                ("code_challenge", "c"), ("token", ADMIN), ("decision", "allow")])
            .send().await.unwrap();
        assert_eq!(cross.status(), 403);

        // An unregistered redirect never gets a redirect, just an error page.
        let page = http.get(format!("{base}/oauth/authorize"))
            .query(&[("response_type","code"),("client_id", client_id),
                ("redirect_uri","https://evil.example/cb"),("code_challenge","c"),
                ("code_challenge_method","S256")])
            .send().await.unwrap();
        assert_eq!(page.status(), 400);
    }

    #[test]
    fn loopback_base_maps_wildcards() {
        assert_eq!(loopback_base("0.0.0.0", 3030), "http://127.0.0.1:3030/api");
        assert_eq!(loopback_base("127.0.0.1", 4000), "http://127.0.0.1:4000/api");
        assert_eq!(loopback_base("::", 3030), "http://[::1]:3030/api");
    }

    #[test]
    fn origin_rules() {
        let base = "https://admin.example.com";
        let mut h = HeaderMap::new();
        assert!(origin_allowed(&h, base));
        h.insert(header::ORIGIN, HeaderValue::from_static("https://claude.ai"));
        assert!(origin_allowed(&h, base));
        h.insert(header::ORIGIN, HeaderValue::from_static("https://evil.example.com"));
        assert!(!origin_allowed(&h, base));
        h.insert(header::ORIGIN, HeaderValue::from_static("https://admin.example.com"));
        assert!(origin_allowed(&h, base));
    }
}
