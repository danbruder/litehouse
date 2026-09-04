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

mod chart;

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
        "env-invalid" => Some("Environment variable key can't be empty."),
        "env-set-failed" => Some("Failed to save the environment variable — check the server logs."),
        "env-delete-failed" => Some("Failed to delete the environment variable — check the server logs."),
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
struct AppDetailQuery {
    flash: Option<String>,
    range: Option<String>,
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

struct DeployRow {
    id: String,
    status: String,
    image: String,
    sha: String,
    created_at: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "deploy_detail.html")]
struct DeployDetailTemplate {
    app_name: String,
    deploy_id: String,
    short_id: String,
    status: String,
    image: String,
    git_sha: Option<String>,
    created_at: String,
    error: Option<String>,
    is_latest: bool,
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
    metrics_range: String,
    cpu_chart: String,
    mem_chart: String,
    disk_chart: String,
}

struct BackupRow {
    app_name: String,
    s3_key: String,
    size: String,
    age: String,
}

#[derive(Template)]
#[template(path = "backups.html")]
struct BackupsTemplate {
    backups: Vec<BackupRow>,
}

// ---------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------

pub fn create_ui_router(state: Arc<RwLock<AppState>>) -> Router {
    let protected = Router::new()
        .route("/", get(spa_shell))
        .route("/apps/:name", get(app_detail))
        .route("/apps/:name/deploys/:deploy_id", get(deploy_detail))
        .route("/apps/:name/start", post(start_app_ui))
        .route("/apps/:name/stop", post(stop_app_ui))
        .route("/apps/:name/restart", post(restart_app_ui))
        .route("/apps/:name/redeploy", post(redeploy_app_ui))
        .route("/apps/:name/env/set", post(set_env_ui))
        .route("/apps/:name/env/delete", post(delete_env_ui))
        .route("/backup/run", post(run_backup_ui))
        .route("/backups", get(backups_page))
        .route("/logout", post(logout))
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

/// `GET /` — the React admin dashboard. Everything else in this module is
/// server-rendered Askama+HTMX; the dashboard is the one page that's been
/// migrated to a SPA (see `frontend/`, built once — never on the server —
/// and its output committed into `src/ui/spa/` like `htmx.min.js` and
/// `styles.css` above). It talks to the same JSON API the CLI uses
/// (`src/api.rs`), authenticated with the same `litehouse_token` cookie
/// this handler is already gated behind, plus a few SPA-only endpoints
/// (`/api/apps/summary`, `/api/apps/:name/restart`, `/api/metrics/server`).
///
/// Links off the dashboard (an app's detail page, backups) still go to the
/// existing Askama+HTMX pages below — full page navigations, not a client
/// router — until those are migrated too.
async fn spa_shell() -> impl IntoResponse {
    Html(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>litehouse</title>
  <link rel="icon" href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 100 100%22><text y=%22.9em%22 font-size=%2290%22>🏠</text></svg>">
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
    <a class="brand" href="/"><h1>🏠 litehouse</h1><span class="cursor">_</span></a>
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
</html>"#,
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

async fn build_app_metrics_charts(pool: &sqlx::Pool<sqlx::Sqlite>, scope: &str, range: &str) -> (String, String, String) {
    if range == "30d" {
        let since = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let rows = db::metrics::list_hourly_since(pool, scope, &since).await.unwrap_or_default();
        let cpu_avg: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_avg).collect();
        let cpu_min: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_min).collect();
        let cpu_max: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_max).collect();
        let mem_avg: Vec<Option<f64>> = rows.iter().map(|r| r.mem_avg.map(|v| v as f64)).collect();
        let mem_min: Vec<Option<f64>> = rows.iter().map(|r| r.mem_min.map(|v| v as f64)).collect();
        let mem_max: Vec<Option<f64>> = rows.iter().map(|r| r.mem_max.map(|v| v as f64)).collect();
        let disk_avg: Vec<Option<f64>> = rows.iter().map(|r| r.disk_avg.map(|v| v as f64)).collect();
        let disk_min: Vec<Option<f64>> = rows.iter().map(|r| r.disk_min.map(|v| v as f64)).collect();
        let disk_max: Vec<Option<f64>> = rows.iter().map(|r| r.disk_max.map(|v| v as f64)).collect();
        (
            chart::band_chart(&cpu_avg, &cpu_min, &cpu_max, chart::ChartUnit::Percent),
            chart::band_chart(&mem_avg, &mem_min, &mem_max, chart::ChartUnit::Bytes),
            chart::band_chart(&disk_avg, &disk_min, &disk_max, chart::ChartUnit::Bytes),
        )
    } else {
        let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let rows = db::metrics::list_samples_since(pool, scope, &since).await.unwrap_or_default();
        let cpu: Vec<Option<f64>> = rows.iter().map(|r| r.cpu_pct).collect();
        let mem: Vec<Option<f64>> = rows.iter().map(|r| r.mem_bytes.map(|v| v as f64)).collect();
        let disk: Vec<Option<f64>> = rows.iter().map(|r| r.disk_bytes.map(|v| v as f64)).collect();
        (
            chart::line_chart(&cpu, chart::ChartUnit::Percent),
            chart::line_chart(&mem, chart::ChartUnit::Bytes),
            chart::line_chart(&disk, chart::ChartUnit::Bytes),
        )
    }
}

async fn backups_page(State(state): State<Arc<RwLock<AppState>>>) -> Response {
    let pool = state.read().await.db_pool.clone();
    let records = match db::backup::list_all(&pool).await {
        Ok(records) => records,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list backups: {e}"),
            )
                .into_response();
        }
    };

    let backups = records
        .into_iter()
        .map(|b| BackupRow {
            app_name: b.app_name,
            s3_key: b.s3_key,
            size: chart::format_bytes(b.size_bytes),
            age: relative_time(&b.created_at),
        })
        .collect();

    HtmlTemplate(BackupsTemplate { backups }).into_response()
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
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;
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
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    form: Option<Form<ActionForm>>,
) -> impl IntoResponse {
    let next = form.as_ref().and_then(|f| f.next.as_deref()).map(str::to_string);
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;
    let error = match crate::commands::stop::execute(&pool, &docker, &name).await {
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
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;
    let error = if let Err(e) = crate::commands::stop::execute(&pool, &docker, &name).await {
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
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;

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
// Environment variables
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SetEnvForm {
    key: String,
    value: String,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeleteEnvForm {
    key: String,
    next: Option<String>,
}

async fn set_env_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Form(form): Form<SetEnvForm>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let key = form.key.trim();
    let error = if key.is_empty() {
        Some("env-invalid")
    } else {
        match crate::commands::app_env::set_env(&pool, &name, key, &form.value, false).await {
            Ok(()) => None,
            Err(e) => {
                tracing::error!("ui: failed to set env var '{}' for app '{}': {}", key, name, e);
                Some("env-set-failed")
            }
        }
    };
    redirect_after_action(form.next.as_deref(), error)
}

async fn delete_env_ui(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Form(form): Form<DeleteEnvForm>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let error = match crate::commands::app_env::set_env(&pool, &name, &form.key, "", true).await {
        Ok(()) => None,
        Err(e) => {
            tracing::error!(
                "ui: failed to delete env var '{}' for app '{}': {}",
                form.key,
                name,
                e
            );
            Some("env-delete-failed")
        }
    };
    redirect_after_action(form.next.as_deref(), error)
}

// ---------------------------------------------------------------------
// App detail
// ---------------------------------------------------------------------

async fn app_detail(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Query(q): Query<AppDetailQuery>,
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

    let deploys = db::deploy::list_for_app(&pool, &app.id, 8)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|d| DeployRow {
            id: d.id,
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

    let metrics_range = match q.range.as_deref() {
        Some("30d") => "30d".to_string(),
        _ => "24h".to_string(),
    };
    let (cpu_chart, mem_chart, disk_chart) = build_app_metrics_charts(&pool, &app.id, &metrics_range).await;

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
        metrics_range,
        cpu_chart,
        mem_chart,
        disk_chart,
    })
    .into_response()
}

async fn deploy_detail(
    State(state): State<Arc<RwLock<AppState>>>,
    Path((name, deploy_id)): Path<(String, String)>,
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

    let deploy = match db::deploy::get_by_id(&pool, &deploy_id).await {
        Ok(Some(d)) if d.app_id == app.id => d,
        Ok(_) => return (StatusCode::NOT_FOUND, "deploy not found").into_response(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load deploy: {e}"),
            )
                .into_response();
        }
    };

    // Only the app's most recent deploy corresponds to the currently running
    // container — earlier ones were replaced and their logs are gone.
    let is_latest = matches!(
        db::deploy::latest_for_app(&pool, &app.id).await,
        Ok(Some(latest)) if latest.id == deploy.id
    );

    HtmlTemplate(DeployDetailTemplate {
        app_name: app.name,
        deploy_id: deploy.id.clone(),
        short_id: deploy.id.chars().take(8).collect(),
        status: deploy.status,
        image: deploy.image,
        git_sha: deploy.git_sha,
        created_at: relative_time(&deploy.created_at),
        error: deploy.error,
        is_latest,
    })
    .into_response()
}

async fn log_tail(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match crate::commands::logs::execute(&pool, &name, 300, false).await {
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
            app_locks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }))
    }

    async fn state_with_app(name: &str) -> Arc<RwLock<AppState>> {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new(name).unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        state
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
    async fn get_root_with_valid_cookie_renders_spa_shell() {
        // `/` no longer renders an apps table server-side — it serves the
        // React dashboard's shell (see `spa_shell`), which fetches
        // /api/apps/summary itself. Assert on the mount point and script
        // tag rather than app data, which this handler no longer touches.
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

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains(r#"id="root""#));
        assert!(body.contains("/assets/spa.js"));
    }

    // The dashboard's data (backups summary, deploy status badges, server
    // resource charts, the empty-state hint) used to be asserted on here as
    // rendered HTML from `apps_index`. That handler is gone — `/` now
    // serves the React SPA (`spa_shell`) — so those assertions moved with
    // the data to `frontend/`, a plain fetch of already-tested JSON
    // (`apps_summary`, `get_backup_status`, `get_server_metrics` in
    // src/api.rs) rather than server-rendered markup this module can
    // exercise. `/backup/run` itself (below) still redirects the same way
    // for anyone hitting it directly.
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
    async fn deploy_detail_shows_metadata_and_logs_for_latest_deploy() {
        let state = test_state().await;
        let deploy_id;
        {
            let s = state.read().await;
            let app = App::new("deploy-detail-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            let deploy = crate::models::Deploy::new(&app.id, "ghcr.io/x/deploy-detail-app:sha", Some("abc1234def"));
            deploy_id = deploy.id.clone();
            db::deploy::insert(&s.db_pool, &deploy).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/apps/deploy-detail-app/deploys/{deploy_id}"))
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("ghcr.io/x/deploy-detail-app:sha"));
        assert!(body.contains("abc1234def"));
        assert!(body.contains("badge-deploy-in_progress"));
        // Latest deploy for the app -> logs are shown, not the "gone" hint.
        assert!(body.contains("live tail"));
        assert!(!body.contains("its container is gone"));
    }

    #[tokio::test]
    async fn deploy_detail_older_deploy_shows_no_logs_hint() {
        let state = test_state().await;
        let old_id;
        {
            let s = state.read().await;
            let app = App::new("deploy-detail-old-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            let old = crate::models::Deploy::new(&app.id, "ghcr.io/x/app:old", Some("aaaaaaaaaa"));
            old_id = old.id.clone();
            db::deploy::insert(&s.db_pool, &old).await.unwrap();
            db::deploy::set_status(&s.db_pool, &old.id, "succeeded", None).await.unwrap();
            let new = crate::models::Deploy::new(&app.id, "ghcr.io/x/app:new", Some("bbbbbbbbbb"));
            db::deploy::insert(&s.db_pool, &new).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/apps/deploy-detail-old-app/deploys/{old_id}"))
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("its container is gone"));
        assert!(!body.contains("live tail"));
    }

    #[tokio::test]
    async fn deploy_detail_unknown_id_returns_404() {
        let state = state_with_app("deploy-detail-404-app").await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/deploy-detail-404-app/deploys/does-not-exist")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn app_detail_deploy_row_links_to_deploy_detail() {
        let state = test_state().await;
        let deploy_id;
        {
            let s = state.read().await;
            let app = App::new("deploy-link-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            let deploy = crate::models::Deploy::new(&app.id, "ghcr.io/x/deploy-link-app:sha", Some("abc1234def"));
            deploy_id = deploy.id.clone();
            db::deploy::insert(&s.db_pool, &deploy).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/deploy-link-app")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains(&format!("/apps/deploy-link-app/deploys/{deploy_id}")));
        // Both list and detail pages poll now, not just while a deploy is in progress.
        assert!(body.contains(r#"id="deploys-table""#));
        assert!(body.contains(r#"id="app-state""#));
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
    async fn set_env_ui_persists_and_never_echoes_value() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("env-set-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        let pool = state.read().await.db_pool.clone();
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/env-set-app/env/set")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(
                        "key=API_KEY&value=super-secret&next=/apps/env-set-app",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/apps/env-set-app"
        );

        let app_row = db::app::get_by_name(&pool, "env-set-app")
            .await
            .unwrap()
            .unwrap();
        let vars = db::env_var::get_by_app(&pool, &app_row.id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "API_KEY");
        assert_eq!(vars[0].value, "super-secret");
    }

    #[tokio::test]
    async fn set_env_ui_rejects_empty_key() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("env-empty-key-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/env-empty-key-app/env/set")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(
                        "key=&value=x&next=/apps/env-empty-key-app",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/apps/env-empty-key-app?flash=env-invalid"
        );
    }

    #[tokio::test]
    async fn delete_env_ui_removes_var() {
        let state = test_state().await;
        {
            let s = state.read().await;
            let app = App::new("env-delete-app").unwrap();
            db::app::save(&s.db_pool, &app).await.unwrap();
            db::env_var::save(
                &s.db_pool,
                &crate::models::EnvVar::new(&app.id, "TO_DELETE", "whatever"),
            )
            .await
            .unwrap();
        }
        let pool = state.read().await.db_pool.clone();
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/apps/env-delete-app/env/delete")
                    .header(header::HOST, "admin.lh.example.com")
                    .header(header::ORIGIN, "https://admin.lh.example.com")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(axum::body::Body::from(
                        "key=TO_DELETE&next=/apps/env-delete-app",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);

        let app_row = db::app::get_by_name(&pool, "env-delete-app")
            .await
            .unwrap()
            .unwrap();
        let vars = db::env_var::get_by_app(&pool, &app_row.id).await.unwrap();
        assert!(vars.is_empty());
    }

    // `/` no longer renders `?flash=` (it's the SPA shell — see `spa_shell`
    // and `get_root_with_valid_cookie_renders_spa_shell`); `flash_message`
    // itself is still exercised on the pages that do render it
    // (`/apps/:name`, via `AppDetailQuery`), and its injection-safety
    // property — an arbitrary `?flash=` code never becomes rendered HTML —
    // is a property of the pure function, tested directly here rather than
    // through a page that happens to call it.
    #[test]
    fn flash_message_maps_known_codes_and_rejects_unknown_ones() {
        assert_eq!(
            flash_message("stop-failed"),
            Some("Failed to stop the app — check the server logs.")
        );
        assert_eq!(flash_message("<script>alert(1)</script>"), None);
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

    #[tokio::test]
    async fn backups_page_without_cookie_redirects_to_login() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/backups")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

    #[tokio::test]
    async fn backups_page_shows_empty_state_with_no_catalog_rows() {
        let state = test_state().await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/backups")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("No backups recorded yet"));
    }

    #[tokio::test]
    async fn backups_page_lists_catalog_rows() {
        let state = test_state().await;
        {
            let s = state.read().await;
            db::backup::record_upload(&s.db_pool, "demo-app", "apps/demo-app/2026-07-11.tar.gz", 123456)
                .await
                .unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/backups")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("demo-app"));
        assert!(body.contains("2026-07-11.tar.gz"));
        assert!(body.contains("120.6 KB"));
    }

    #[tokio::test]
    async fn app_detail_shows_resources_card_with_no_samples() {
        let state = state_with_app("metrics-app").await;
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/metrics-app")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("Resources"));
        assert!(body.contains("no data yet"));
    }

    #[tokio::test]
    async fn app_detail_range_toggle_switches_to_hourly_rollups() {
        let state = state_with_app("metrics-app-30d").await;
        {
            let s = state.read().await;
            let app_row = db::app::get_by_name(&s.db_pool, "metrics-app-30d").await.unwrap().unwrap();
            db::metrics::insert_sample(&s.db_pool, "2026-07-12T10:00:00+00:00", &app_row.id, Some(5.0), Some(1000), Some(2000))
                .await
                .unwrap();
            db::metrics::rollup_hour(&s.db_pool, "2026-07-12T10:00:00+00:00", "2026-07-12T11:00:00+00:00")
                .await
                .unwrap();
        }
        let app = router(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps/metrics-app-30d?range=30d")
                    .header(header::COOKIE, format!("litehouse_token={TEST_TOKEN}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_string(hyper::body::to_bytes(response.into_body()).await.unwrap());
        assert!(body.contains("5.0% avg"));
    }
}
