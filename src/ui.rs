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
    extract::{Form, Path, Query, State},
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

/// "3m ago"-style rendering of an RFC 3339 timestamp, relative to `now`.
/// Unparseable input is returned verbatim rather than erased — a raw
/// timestamp is still more useful on the dashboard than a blank cell.
fn relative_time_at(rfc3339: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.to_string();
    };
    let secs = (now - ts.with_timezone(&chrono::Utc)).num_seconds().max(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

fn relative_time(rfc3339: &str) -> String {
    relative_time_at(rfc3339, chrono::Utc::now())
}

/// Fixed flash-code -> message map. Codes travel in the `?flash=` query
/// param; anything not in this map renders nothing, so the param is not an
/// injection surface and needs no encoding.
fn flash_message(code: &str) -> Option<&'static str> {
    match code {
        "start-failed" => Some("Failed to start the app — check the server logs."),
        "stop-failed" => Some("Failed to stop the app — check the server logs."),
        "restart-failed" => Some("Failed to restart the app — check the server logs."),
        "redeploy-failed" => Some("Redeploy could not be started — check the server logs."),
        "no-image" => Some("This app has no deployed image yet — push to its repo first."),
        "backup-started" => Some("Backup started — refresh in a minute for the report."),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct ActionForm {
    next: Option<String>,
}

/// Only same-site absolute paths are honored; anything else ("//evil…",
/// "https://…", relative paths) falls back to "/".
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(p) if p.starts_with('/') && !p.starts_with("//") => p.to_string(),
        _ => "/".to_string(),
    }
}

fn redirect_after_action(next: Option<&str>, error_code: Option<&str>) -> Redirect {
    let base = safe_next(next);
    match error_code {
        Some(code) => Redirect::to(&format!("{base}?flash={code}")),
        None => Redirect::to(&base),
    }
}

#[derive(Debug, Deserialize)]
struct FlashQuery {
    flash: Option<String>,
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
    if req.method() != axum::http::Method::GET {
        let our_host = req
            .headers()
            .get(axum::http::header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let source = req
            .headers()
            .get(axum::http::header::ORIGIN)
            .or_else(|| req.headers().get(axum::http::header::REFERER))
            .and_then(|h| h.to_str().ok());
        let source_host = source
            .and_then(|s| s.split("//").nth(1))
            .map(|rest| rest.split('/').next().unwrap_or(rest));
        match source_host {
            Some(host) if host == our_host && !our_host.is_empty() => {}
            _ => {
                return (
                    axum::http::StatusCode::FORBIDDEN,
                    "cross-origin request rejected",
                )
                    .into_response();
            }
        }
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

struct AppRow {
    name: String,
    state: String,
    state_class: String,
    url: String,
    deploy_status: Option<String>,
    last_deploy: String,
}

#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate {
    apps: Vec<AppRow>,
    backup_line: String,
    backup_failures: Vec<(String, String)>,
    flash: Option<String>,
    any_in_progress: bool,
}

struct DeployRow {
    status: String,
    image: String,
    sha: String,
    created_at: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "app_detail.html")]
struct AppDetailTemplate {
    app_name: String,
    state: String,
    state_class: String,
    url: String,
    image: Option<String>,
    repo: Option<String>,
    port: Option<i64>,
    custom_domains: Vec<String>,
    env_names: Vec<String>,
    deploys: Vec<DeployRow>,
    flash: Option<String>,
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
        .route("/apps/:name/restart", post(restart_app_ui))
        .route("/apps/:name/redeploy", post(redeploy_app_ui))
        .route("/backup/run", post(run_backup_ui))
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

async fn run_backup_ui(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    // Backups VACUUM every app DB and upload to S3 — far too slow to hold a
    // request open for. Fire and forget; run_backup persists its own report,
    // which the index card shows on the next load.
    tokio::spawn(async move {
        if let Err(e) = crate::backup::run_backup(&pool, &docker).await {
            tracing::error!("ui: manual backup run failed: {e:#}");
        }
    });
    Redirect::to("/?flash=backup-started")
}

async fn apps_index(
    State(state): State<Arc<RwLock<AppState>>>,
    Query(q): Query<FlashQuery>,
) -> Response {
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

    let mut any_in_progress = false;
    let mut rows = Vec::with_capacity(apps.len());
    for app in apps {
        let (deploy_status, last_deploy) = match db::deploy::latest_for_app(&pool, &app.id).await {
            Ok(Some(d)) => {
                if d.status == "in_progress" {
                    any_in_progress = true;
                }
                let short_sha = d
                    .git_sha
                    .as_deref()
                    .map(|s| s.chars().take(7).collect::<String>())
                    .unwrap_or_else(|| "-".to_string());
                (
                    Some(d.status),
                    format!("{} · {}", short_sha, relative_time(&d.created_at)),
                )
            }
            Ok(None) => (None, "no deploys".to_string()),
            Err(_) => (None, "unknown".to_string()),
        };

        // Read-through: reflect the live Docker container state rather than
        // the (possibly stale) cached DB column. Fall back to the cached
        // desired state if Docker can't be reached.
        let live = crate::docker::live_state(&app.name)
            .await
            .unwrap_or(app.state);
        let state_str = live.to_string();
        let state_class = state_str.clone();
        rows.push(AppRow {
            name: app.name.clone(),
            state: state_str,
            state_class,
            url: app_url(&app.name, &config),
            deploy_status,
            last_deploy,
        });
    }

    let (backup_line, backup_failures) =
        match db::system_config::get_last_backup_report(&pool).await {
            Ok(Some(report)) => (
                format!(
                    "{} succeeded, {} failed (last run {})",
                    report.succeeded.len(),
                    report.failed.len(),
                    relative_time(&report.ran_at)
                ),
                report.failed,
            ),
            _ => ("no backup has run yet".to_string(), Vec::new()),
        };

    HtmlTemplate(AppsTemplate {
        apps: rows,
        backup_line,
        backup_failures,
        flash: q
            .flash
            .as_deref()
            .and_then(flash_message)
            .map(str::to_string),
        any_in_progress,
    })
    .into_response()
}

// ---------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------

async fn start_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    let error = match crate::commands::start::execute(&pool, &docker, &name).await {
        Ok(()) => None,
        Err(e) => {
            tracing::error!("ui: failed to start app '{}': {}", name, e);
            Some("start-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}

async fn stop_app_ui(
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let error = match crate::commands::stop::execute(&name).await {
        Ok(()) => None,
        Err(e) => {
            tracing::error!("ui: failed to stop app '{}': {}", name, e);
            Some("stop-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}

async fn restart_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };
    let error = if let Err(e) = crate::commands::stop::execute(&name).await {
        tracing::error!("ui: failed to restart app '{}' (stop step): {}", name, e);
        Some("restart-failed")
    } else if let Err(e) = crate::commands::start::execute(&pool, &docker, &name).await {
        tracing::error!("ui: failed to restart app '{}' (start step): {}", name, e);
        Some("restart-failed")
    } else {
        None
    };
    redirect_after_action(next.as_deref(), error)
}

async fn redeploy_app_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };

    let image = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app.image,
        _ => None,
    };
    let Some(image) = image else {
        return redirect_after_action(next.as_deref(), Some("no-image"));
    };

    // deploy_app records success/failure in the deploy table; the detail
    // page's deploy history is the primary feedback channel, so only a
    // failure to even start the deploy warrants a flash.
    let error = match crate::deploy::deploy_app(&pool, &docker, &name, &image, None).await {
        Ok(_) => None,
        Err(e) => {
            tracing::error!("ui: failed to redeploy app '{}': {}", name, e);
            Some("redeploy-failed")
        }
    };
    redirect_after_action(next.as_deref(), error)
}

// ---------------------------------------------------------------------
// App detail
// ---------------------------------------------------------------------

async fn app_detail(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Query(q): Query<FlashQuery>,
) -> Response {
    let (pool, config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone())
    };

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

    let live = crate::docker::live_state(&app.name)
        .await
        .unwrap_or(app.state);
    let state_str = live.to_string();

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
            created_at: relative_time(&d.created_at),
            error: d.error,
        })
        .collect();

    HtmlTemplate(AppDetailTemplate {
        url: app_url(&app.name, &config),
        state_class: state_str.clone(),
        state: state_str,
        image: app.image.clone(),
        repo: app.repo.clone(),
        port: app.port,
        custom_domains: app.custom_domains_list(),
        app_name: app.name,
        env_names,
        deploys,
        flash: q
            .flash
            .as_deref()
            .and_then(flash_message)
            .map(str::to_string),
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

    #[test]
    fn relative_time_formats_ago_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-11T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(relative_time_at("2026-07-11T11:59:40Z", now), "just now");
        assert_eq!(relative_time_at("2026-07-11T11:55:00Z", now), "5m ago");
        assert_eq!(relative_time_at("2026-07-11T09:00:00Z", now), "3h ago");
        assert_eq!(relative_time_at("2026-07-08T12:00:00Z", now), "3d ago");
    }

    #[test]
    fn relative_time_passes_through_unparseable_input() {
        let now = chrono::Utc::now();
        assert_eq!(relative_time_at("not-a-date", now), "not-a-date");
    }

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
    async fn index_backups_card_lists_failed_apps() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let report = crate::backup::BackupReport {
                succeeded: vec!["good-app".to_string()],
                failed: vec![("bad-app".to_string(), "S3 upload timed out".to_string())],
                ran_at: "2026-07-10T02:00:00Z".to_string(),
            };
            db::system_config::set_last_backup_report(&s.db_pool, &report)
                .await
                .unwrap();
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

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("bad-app"));
        assert!(body.contains("S3 upload timed out"));
    }

    #[tokio::test]
    async fn run_backup_now_redirects_with_started_flash() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/backup/run")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/?flash=backup-started"
        );
    }

    #[tokio::test]
    async fn index_shows_deploy_status_badge_and_polls_while_in_progress() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("deploying-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            // Deploy::new inserts with status "in_progress".
            let deploy = crate::models::Deploy::new(&app.id, "ghcr.io/x/deploying-app:sha", Some("abc1234def"));
            db::deploy::insert(&s.db_pool, &deploy).await.unwrap();
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

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("badge-deploy-in_progress"));
        assert!(body.contains("hx-trigger=\"every 5s\""));
    }

    #[tokio::test]
    async fn index_without_apps_shows_create_hint() {
        let state = test_state().await;
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

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("lh create"));
        // No polling attribute when nothing is deploying.
        assert!(!body.contains("hx-trigger=\"every 5s\""));
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

    #[tokio::test]
    async fn app_detail_header_card_shows_state_url_and_domains() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let mut app = App::new("card-app").unwrap();
            app.repo = Some("danbruder/card-app".to_string());
            app.image = Some("ghcr.io/danbruder/card-app:abc".to_string());
            app.custom_domains = Some(r#"["cardapp.example.com"]"#.to_string());
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/card-app")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("badge-"));                       // state badge
        assert!(body.contains("http://card-app.localhost"));    // app URL (local dev)
        assert!(body.contains("github.com/danbruder/card-app")); // repo link
        assert!(body.contains("ghcr.io/danbruder/card-app:abc")); // image
        assert!(body.contains("cardapp.example.com"));           // custom domain
        assert!(body.contains("/apps/card-app/start"));          // action form
    }

    #[tokio::test]
    async fn app_detail_shows_deploy_error() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("erroring-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            let deploy = crate::models::Deploy::new(&app.id, "ghcr.io/x/erroring-app:sha", Some("abc1234def"));
            db::deploy::insert(&s.db_pool, &deploy).await.unwrap();
            db::deploy::set_status(&s.db_pool, &deploy.id, "failed", Some("failed to pull image: manifest unknown"))
                .await
                .unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/erroring-app")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("manifest unknown"));
        assert!(body.contains("badge-deploy-failed"));
        assert!(body.contains("ago") || body.contains("just now"));
    }

    #[tokio::test]
    async fn app_detail_never_renders_env_values() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("envapp").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            db::env_var::save(
                &s.db_pool,
                &crate::models::EnvVar::new(&app.id, "SECRET_KEY", "super-secret-value"),
            )
            .await
            .unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/envapp")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("SECRET_KEY"));
        assert!(!body.contains("super-secret-value"));
    }

    #[tokio::test]
    async fn log_tail_without_cookie_redirects_to_login() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/whatever/log-tail")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

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
                    .uri("/apps/whatever/stop")
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
                    .uri("/apps/whatever/stop")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The guard lets it through to the handler, which (even when the app
        // doesn't exist) logs the error and redirects home with a flash —
        // NOT 403, NOT /login.
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/?flash=stop-failed"
        );
    }

    #[tokio::test]
    async fn stop_failure_redirects_back_to_next_with_flash() {
        let state = test_state().await;
        let app = router(state);

        // "whatever" doesn't exist, so stop fails -> flash code appended.
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/whatever/stop")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from("next=/apps/whatever"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/apps/whatever?flash=stop-failed"
        );
    }

    #[tokio::test]
    async fn restart_unknown_app_redirects_with_flash() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/whatever/restart")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from("next=/apps/whatever"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/apps/whatever?flash=restart-failed"
        );
    }

    #[tokio::test]
    async fn redeploy_app_without_image_redirects_with_no_image_flash() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("noimage-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/noimage-app/redeploy")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from("next=/apps/noimage-app"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/apps/noimage-app?flash=no-image"
        );
    }

    #[tokio::test]
    async fn index_renders_flash_message_for_known_code() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/?flash=stop-failed")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("Failed to stop the app"));
    }

    #[tokio::test]
    async fn unknown_flash_code_renders_nothing() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/?flash=%3Cscript%3Ealert(1)%3C%2Fscript%3E")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(!body.contains("alert(1)"));
        assert!(!body.contains("class=\"flash\""));
    }

    #[tokio::test]
    async fn next_field_rejects_offsite_redirects() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/whatever/stop")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from("next=//evil.example.com/phish"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Offsite `next` falls back to "/".
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/?flash=stop-failed"
        );
    }

    #[tokio::test]
    async fn state_changing_post_with_no_origin_or_referer_is_rejected() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/whatever/stop")
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
