//! Server-rendered admin UI (Askama + HTMX), mounted at the root of the
//! server's router. This is a small, read-heavy dashboard on top of the JSON
//! API in `src/api.rs` — the CLI remains the primary interface.
//!
//! Auth: a single admin token, presented as a `litehouse_token` cookie
//! (set by `POST /login`). Protected routes are wrapped in
//! [`ui_auth_middleware`], which mirrors `crate::auth::admin_auth_middleware`
//! but redirects to `/login` on failure instead of returning a bare 401 —
//! browsers navigating to a protected page should land on the login form,
//! not a blank error page.
//!
//! Design choice: start/stop actions are plain HTML `<form>` POSTs that
//! redirect back to the referring page, not HTMX-driven partial swaps. This
//! keeps the state-changing endpoints trivially testable with
//! `tower::ServiceExt::oneshot` (a form POST is just a normal request/redirect
//! cycle) and keeps the "it worked" signal an ordinary full-page reload
//! rather than something that depends on JS running in a headless client.
//! HTMX is used where it actually earns its keep: polling the log tail on
//! the app detail page every 5 seconds without a full page reload.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use futures_util::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::commands::server::AppState;
use crate::db;

const COOKIE_NAME: &str = "litehouse_token";

fn is_local_dev() -> bool {
    std::env::var("LITEHOUSE_LOCAL_DEV").is_ok() || cfg!(debug_assertions)
}

/// Best-effort URL an app is reachable at, mirroring `api::app_url` (kept
/// separate to avoid making that function `pub`).
fn app_url(app_name: &str, config: &crate::config::ServerConfig) -> String {
    if is_local_dev() {
        format!("http://{}.localhost:9090", app_name)
    } else if let Some(domain) = &config.domain {
        format!("https://{}.{}", app_name, domain)
    } else {
        format!("https://{}.<configure-domain>", app_name)
    }
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

    if authorized {
        next.run(req).await
    } else {
        Redirect::to("/login").into_response()
    }
}

// ---------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

struct AppRow {
    name: String,
    state: String,
    state_class: String,
    url: String,
    last_deploy: String,
}

#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate {
    apps: Vec<AppRow>,
    backups_summary: String,
}

struct DeployRow {
    status: String,
    image: String,
    sha: String,
    created_at: String,
}

#[derive(Template)]
#[template(path = "app_detail.html")]
struct AppDetailTemplate {
    app_name: String,
    env_names: Vec<String>,
    deploys: Vec<DeployRow>,
}

// ---------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------

pub fn create_ui_router(state: Arc<RwLock<AppState>>) -> Router {
    let protected = Router::new()
        .route("/", get(apps_index))
        .route("/apps/:name", get(app_detail))
        .route("/apps/:name/start", post(start_app_ui))
        .route("/apps/:name/stop", post(stop_app_ui))
        .route("/apps/:name/log-tail", get(log_tail))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ui_auth_middleware,
        ))
        .with_state(state.clone());

    let public = Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/assets/htmx.min.js", get(serve_htmx))
        .route("/assets/styles.css", get(serve_styles))
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

// ---------------------------------------------------------------------
// Apps index
// ---------------------------------------------------------------------

async fn apps_index(State(state): State<Arc<RwLock<AppState>>>) -> Response {
    let (pool, config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone())
    };

    let apps = match db::app::get_all(&pool).await {
        Ok(apps) => apps,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list apps: {e}"),
            )
                .into_response();
        }
    };

    let mut rows = Vec::with_capacity(apps.len());
    for app in apps {
        let last_deploy = match db::deploy::latest_for_app(&pool, &app.id).await {
            Ok(Some(d)) => {
                let short_sha = d
                    .git_sha
                    .as_deref()
                    .map(|s| s.chars().take(7).collect::<String>())
                    .unwrap_or_else(|| "-".to_string());
                format!("{} ({}) at {}", d.status, short_sha, d.created_at)
            }
            Ok(None) => "no deploys".to_string(),
            Err(_) => "unknown".to_string(),
        };

        let state_str = app.state.to_string();
        let state_class = state_str.clone();
        rows.push(AppRow {
            name: app.name.clone(),
            state: state_str,
            state_class,
            url: app_url(&app.name, &config),
            last_deploy,
        });
    }

    let backups_summary = match db::system_config::get_last_backup_report(&pool).await {
        Ok(Some(report)) => format!(
            "{} succeeded, {} failed (last run {})",
            report.succeeded.len(),
            report.failed.len(),
            report.ran_at
        ),
        Ok(None) => "n/a".to_string(),
        Err(_) => "n/a".to_string(),
    };

    HtmlTemplate(AppsTemplate {
        apps: rows,
        backups_summary,
    })
    .into_response()
}

// ---------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------

async fn start_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    if let Err(e) = crate::commands::start::execute(&pool, &docker, &name).await {
        tracing::error!("ui: failed to start app '{}': {}", name, e);
    }
    Redirect::to("/")
}

async fn stop_app_ui(Path(name): Path<String>) -> impl IntoResponse {
    if let Err(e) = crate::commands::stop::execute(&name).await {
        tracing::error!("ui: failed to stop app '{}': {}", name, e);
    }
    Redirect::to("/")
}

// ---------------------------------------------------------------------
// App detail
// ---------------------------------------------------------------------

async fn app_detail(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> Response {
    let pool = state.read().await.db_pool.clone();

    let app = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app,
        Ok(None) => return (StatusCode::NOT_FOUND, "app not found").into_response(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load app: {e}"),
            )
                .into_response();
        }
    };

    let env_names = db::env_var::get_by_app(&pool, &app.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.key)
        .collect();

    let deploys = db::deploy::list_for_app(&pool, &app.id, 20)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|d| DeployRow {
            status: d.status,
            image: d.image,
            sha: d
                .git_sha
                .map(|s| s.chars().take(7).collect::<String>())
                .unwrap_or_else(|| "-".to_string()),
            created_at: d.created_at,
        })
        .collect();

    HtmlTemplate(AppDetailTemplate {
        app_name: app.name,
        env_names,
        deploys,
    })
    .into_response()
}

async fn log_tail(Path(name): Path<String>) -> impl IntoResponse {
    match crate::commands::logs::execute(&name, 100, false).await {
        Ok(mut stream) => {
            let mut logs = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(data) => logs.push_str(&data),
                    Err(e) => {
                        logs.push_str(&format!("\n[error reading logs: {e}]"));
                        break;
                    }
                }
            }
            (StatusCode::OK, logs).into_response()
        }
        Err(e) => (
            StatusCode::OK,
            format!("[failed to fetch logs: {e}]"),
        )
            .into_response(),
    }
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
    use crate::models::App;
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
        }))
    }

    fn router(state: Arc<RwLock<AppState>>) -> Router {
        create_ui_router(state)
    }

    fn body_string(bytes: axum::body::Bytes) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn get_root_without_cookie_redirects_to_login() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/login"
        );
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
    async fn get_root_with_valid_cookie_renders_apps_table() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("demo-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("<table"));
        assert!(body.contains("demo-app"));
    }

    #[tokio::test]
    async fn get_app_detail_with_valid_cookie_shows_app_name() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("detail-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/detail-app")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("detail-app"));
    }
}
