//! Server-rendered admin UI, mounted at the root of the server's router.
//! Only login is still classic Askama+HTML — the admin dashboard itself
//! (`/`) and the rest of the admin pages (`/apps/:name`, deploy detail,
//! `/backups`) are a React SPA (see `frontend/`), built once — never on the
//! server — and its output embedded here with `include_str!`, the same way
//! `htmx.min.js` and `styles.css` already are. Every one of those routes
//! serves the exact same HTML shell (`spa_shell`); react-router owns which
//! page actually renders once the bundle loads. See `frontend/README.md`
//! for the full split and how the SPA is built/shipped.
//!
//! Auth: a single admin token, presented as a `litehouse_token` cookie
//! (set by `POST /login`). Protected routes are wrapped in
//! [`ui_auth_middleware`], which mirrors `crate::auth::admin_auth_middleware`
//! but redirects to `/login` on failure instead of returning a bare 401 —
//! browsers navigating to a protected page should land on the login form,
//! not a blank error page.

use askama::Template;
use axum::{
    extract::{Form, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::commands::server::AppState;

const COOKIE_NAME: &str = "litehouse_token";

fn is_local_dev() -> bool {
    std::env::var("LITEHOUSE_LOCAL_DEV").is_ok() || cfg!(debug_assertions)
}

/// Thin wrapper turning any `askama::Template` into an axum response.
/// (`askama_axum` was not used here: its 0.4 line requires axum 0.7, but
/// this project is still on axum 0.6 — see Cargo.toml.)
struct HtmlTemplate<T>(T);

impl<T: Template> IntoResponse for HtmlTemplate<T> {
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("template render error: {e}"),
            )
                .into_response(),
        }
    }
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .map(str::trim)
                .find_map(|kv| kv.strip_prefix(&format!("{COOKIE_NAME}=")))
        })
        .map(str::to_string)
}

/// Cookie-only variant of `crate::auth::admin_auth_middleware`: on failure,
/// redirects to `/login` instead of returning a bare 401, since this guards
/// browser-facing HTML pages rather than API clients.
async fn ui_auth_middleware<B: Send + 'static>(
    State(state): State<Arc<RwLock<AppState>>>,
    req: axum::http::Request<B>,
    next: Next<B>,
) -> Response {
    let expected = state.read().await.admin_token_hash.clone();
    let provided = cookie_token(req.headers());
    let authorized = match provided {
        Some(token) if !expected.is_empty() => {
            crate::auth::constant_time_eq(&crate::auth::hash_token(&token), &expected)
        }
        _ => false,
    };

    if !authorized {
        return Redirect::to("/login").into_response();
    }

    // CSRF guard for state-changing requests. SameSite=Lax does not protect
    // against *same-site* origins — tenant apps live on sibling subdomains of
    // the admin UI, so a malicious deployed app could POST here with the
    // cookie attached. Require the Origin (or Referer) host to match ours.
    // (Shared with the JSON API's `admin_auth_middleware`.)
    if req.method() != axum::http::Method::GET && !crate::auth::same_origin(req.headers()) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            "cross-origin request rejected",
        )
            .into_response();
    }

    next.run(req).await
}

// ---------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

// ---------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------

pub fn create_ui_router(state: Arc<RwLock<AppState>>) -> Router {
    let protected = Router::new()
        .route("/", get(spa_shell))
        .route("/apps/:name", get(spa_shell))
        .route("/apps/:name/deploys/:deploy_id", get(spa_shell))
        .route("/backups", get(spa_shell))
        .route("/logout", post(logout))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ui_auth_middleware,
        ))
        .with_state(state.clone());

    let public = Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/assets/htmx.min.js", get(serve_htmx))
        .route("/assets/styles.css", get(serve_styles))
        .route("/assets/spa.js", get(serve_spa_js))
        .route("/assets/spa.css", get(serve_spa_css))
        .with_state(state.clone());

    Router::new().merge(protected).merge(public)
}

async fn serve_htmx() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("ui/htmx.min.js"),
    )
}

async fn serve_styles() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("ui/styles.css"),
    )
}

async fn serve_spa_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("ui/spa/spa.js"),
    )
}

async fn serve_spa_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], include_str!("ui/spa/spa.css"))
}

/// `GET /`, `/apps/:name`, `/apps/:name/deploys/:deploy_id`, `/backups` —
/// the React admin SPA's shell (see `frontend/`, built once — never on the
/// server — and its output committed into `src/ui/spa/` like `htmx.min.js`
/// and `styles.css` above). Every one of these routes returns the exact
/// same HTML; react-router (client-side) decides which page component to
/// mount from `location.pathname`, and the SPA talks to the same JSON API
/// the CLI uses (`src/api.rs`), authenticated with the same
/// `litehouse_token` cookie this handler is already gated behind, plus a
/// handful of SPA-only endpoints (`/api/apps/summary`,
/// `/api/apps/:name/summary`, `/api/apps/:name/metrics`,
/// `/api/apps/:name/restart`, `/api/backups/catalog`,
/// `/api/metrics/server`).
async fn spa_shell() -> impl IntoResponse {
    Html(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>litehouse</title>
  <link rel="icon" href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 100 100%22><polygon points=%2260,24 96,6 96,22%22 fill=%22%23ffd54a%22/><polygon points=%2228,94 72,94 62,84 38,84%22 fill=%22%234a4a4a%22/><polygon points=%2240,84 60,84 55,30 45,30%22 fill=%22%23f4f1e8%22/><polygon points=%2242.5,62 57.5,62 56,50 44,50%22 fill=%22%23d94f4f%22/><rect x=%2242%22 y=%2218%22 width=%2216%22 height=%2212%22 fill=%22%232b2b2b%22/><polygon points=%2240,18 60,18 50,6%22 fill=%22%23d94f4f%22/></svg>">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;600;700&family=Archivo:ital,wght@0,400;0,500;0,600;1,400&display=swap" rel="stylesheet">
  <link rel="stylesheet" href="/assets/styles.css">
  <link rel="stylesheet" href="/assets/spa.css">
  <script>
    (function () {
      var t = localStorage.theme ||
        (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
      document.documentElement.dataset.theme = t;
    })();
  </script>
</head>
<body>
  <header>
    <a class="brand" href="/"><h1><svg class="brand-icon" viewBox="0 0 100 100" aria-hidden="true"><polygon points="60,24 96,6 96,22" fill="#ffd54a"/><polygon points="28,94 72,94 62,84 38,84" fill="#4a4a4a"/><polygon points="40,84 60,84 55,30 45,30" fill="#f4f1e8"/><polygon points="42.5,62 57.5,62 56,50 44,50" fill="#d94f4f"/><rect x="42" y="18" width="16" height="12" fill="#2b2b2b"/><polygon points="40,18 60,18 50,6" fill="#d94f4f"/></svg> litehouse</h1><span class="cursor">_</span></a>
    <div class="header-actions">
      <button id="theme-toggle" type="button" class="btn-outline btn-small"></button>
      <form class="inline" method="post" action="/logout">
        <button type="submit" class="btn-outline btn-small">sign out</button>
      </form>
    </div>
  </header>
  <main>
    <div id="root"></div>
  </main>
  <script>
    (function () {
      var btn = document.getElementById("theme-toggle");
      if (!btn) return;
      var sync = function () {
        btn.textContent = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
      };
      sync();
      btn.addEventListener("click", function () {
        var next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
        document.documentElement.dataset.theme = next;
        localStorage.theme = next;
        sync();
      });
    })();
  </script>
  <script type="module" src="/assets/spa.js"></script>
</body>
</html>"##,
    )
}

// ---------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------

async fn login_page() -> impl IntoResponse {
    HtmlTemplate(LoginTemplate { error: None })
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    token: String,
}

async fn login_submit(
    State(state): State<Arc<RwLock<AppState>>>,
    Form(form): Form<LoginForm>,
) -> Response {
    let expected = state.read().await.admin_token_hash.clone();
    let ok = !expected.is_empty()
        && crate::auth::constant_time_eq(&crate::auth::hash_token(&form.token), &expected);

    if !ok {
        return HtmlTemplate(LoginTemplate {
            error: Some("Invalid token".to_string()),
        })
        .into_response();
    }

    let secure_attr = if is_local_dev() { "" } else { "; Secure" };
    let cookie_value = format!(
        "{COOKIE_NAME}={}; HttpOnly; SameSite=Lax; Path=/{}",
        form.token, secure_attr
    );

    let mut response = Redirect::to("/").into_response();
    if let Ok(header_value) = axum::http::HeaderValue::from_str(&cookie_value) {
        response.headers_mut().insert(header::SET_COOKIE, header_value);
    }
    response
}

async fn logout() -> Response {
    let mut response = Redirect::to("/login").into_response();
    let cookie = format!("{COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    if let Ok(header_value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, header_value);
    }
    response
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::db::test::get_test_pool;
    use crate::docker;
    use tower::ServiceExt;

    const TEST_TOKEN: &str = "test-admin-token";

    async fn test_state() -> Arc<RwLock<AppState>> {
        let pool = get_test_pool().await;
        let docker_conn = docker::connect().await.expect("connect to docker");
        Arc::new(RwLock::new(AppState {
            db_pool: pool,
            docker: docker_conn,
            admin_token_hash: crate::auth::hash_token(TEST_TOKEN),
            server_config: ServerConfig::default(),
            app_locks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }))
    }

    fn router(state: Arc<RwLock<AppState>>) -> Router {
        create_ui_router(state)
    }

    fn body_string(bytes: axum::body::Bytes) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn get(app: Router, uri: &str, cookie: Option<&str>) -> Response {
        let mut builder = axum::http::Request::builder().uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        app.oneshot(builder.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn get_root_without_cookie_redirects_to_login() {
        let state = test_state().await;
        let response = get(router(state), "/", None).await;

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/login"
        );
    }

    // The three other admin pages (`/apps/:name`, its deploy detail, and
    // `/backups`) are now client routes served by the exact same shell as
    // `/` — see `spa_shell`. They still sit behind the same cookie auth, and
    // an unknown app/deploy id is a React-side "not found" rather than a
    // routing 404, since the router only knows path shapes, not app names.
    #[tokio::test]
    async fn spa_routes_without_cookie_redirect_to_login() {
        let state = test_state().await;
        let app = router(state);

        for uri in ["/", "/apps/whatever", "/apps/whatever/deploys/abc", "/backups"] {
            let response = get(app.clone(), uri, None).await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER, "uri: {uri}");
            assert_eq!(
                response.headers().get(header::LOCATION).unwrap(),
                "/login",
                "uri: {uri}"
            );
        }
    }

    #[tokio::test]
    async fn spa_routes_with_valid_cookie_render_the_same_shell() {
        let state = test_state().await;
        let app = router(state);
        let cookie = format!("litehouse_token={TEST_TOKEN}");

        for uri in ["/", "/apps/whatever", "/apps/whatever/deploys/abc", "/backups"] {
            let response = get(app.clone(), uri, Some(&cookie)).await;
            assert_eq!(response.status(), StatusCode::OK, "uri: {uri}");
            let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
            assert!(body.contains(r#"id="root""#), "uri: {uri}");
            assert!(body.contains("/assets/spa.js"), "uri: {uri}");
        }
    }

    #[tokio::test]
    async fn post_login_correct_token_redirects_and_sets_cookie() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(format!("token={TEST_TOKEN}")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/");
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set_cookie.contains("litehouse_token"));
    }

    #[tokio::test]
    async fn post_login_wrong_token_shows_error_without_cookie() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from("token=wrong"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("Invalid token"));
    }

    #[tokio::test]
    async fn logout_expires_cookie_and_redirects_to_login() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/logout")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set_cookie.contains("litehouse_token=;"));
        assert!(set_cookie.contains("Max-Age=0"));
    }

    // `/logout` is the one remaining state-changing (POST) route in this
    // router, so it's what exercises `ui_auth_middleware`'s CSRF guard here
    // — the same guard `state_changing_post_*` used to test against the
    // since-removed `/apps/:name/stop` HTMX form action.
    #[tokio::test]
    async fn state_changing_post_without_matching_origin_is_rejected() {
        let state = test_state().await;
        let app = router(state);

        // Authenticated cookie, but Origin is a tenant app on a sibling
        // subdomain (same-site!) — the CSRF guard must reject it.
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/logout")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://evil.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn state_changing_post_with_matching_origin_passes_guard() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/logout")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The guard lets it through to the handler, which always
        // redirects to /login regardless of prior auth state.
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

    #[tokio::test]
    async fn state_changing_post_with_no_origin_or_referer_is_rejected() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/logout")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
