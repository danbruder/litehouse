use crate::commands::app_env;
use crate::commands::create;
use crate::commands::delete;
use crate::commands::domain;
use crate::commands::health_check;
use crate::commands::logs;
use crate::commands::server::AppState;
use crate::commands::{start, stop};
use crate::config::ServerConfig;
use crate::db;
use crate::db::env_var;
use crate::db::system_config as db_system_config;
use crate::models::{S3Config, S3ConfigRedacted, SystemConfig};
use axum::body::StreamBody;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::instrument;

pub fn create_api_router(state: Arc<RwLock<AppState>>) -> Router {
    // Everything under /api is protected by the single admin token, except
    // the deploy hook, which is authenticated per-app via its own deploy
    // token (see `hook_deploy`) rather than the admin token — GitHub Actions
    // never sees the admin token.
    let protected_routes = Router::new()
        .route("/apps", get(list_apps))
        .route("/apps", post(create_app))
        .route("/apps/summary", get(apps_summary))
        .route("/apps/:name/summary", get(app_summary))
        .route("/apps/:name/metrics", get(get_app_metrics))
        .route("/apps/:name", get(get_app))
        .route("/apps/:name", delete(delete_app))
        .route("/apps/:name/start", post(start_app))
        .route("/apps/:name/stop", post(stop_app))
        .route("/apps/:name/restart", post(restart_app))
        .route("/apps/:name/logs", get(get_logs))
        .route("/apps/:name/deploy", post(deploy_app))
        .route("/apps/:name/deploys", get(list_deploys))
        .route("/apps/:name/env", post(set_env))
        .route("/apps/:name/env", get(get_env))
        .route("/apps/:name/domains", get(list_domains))
        .route("/apps/:name/domains", post(add_domain))
        .route("/apps/:name/domains", delete(remove_domain))
        .route("/apps/:name/health-check", get(get_health_check))
        .route("/apps/:name/health-check", post(set_health_check))
        .route("/apps/:name/health-check", delete(unset_health_check))
        .route("/docker/version", get(get_docker_version))
        .route("/server/info", get(get_server_info))
        .route("/config/s3", post(set_s3_config))
        .route("/config/s3", get(get_s3_config))
        .route("/config/s3", delete(delete_s3_config))
        .route("/config/ghcr", post(set_ghcr_config))
        .route("/config/ghcr", get(get_ghcr_config))
        .route("/config/ghcr", delete(delete_ghcr_config))
        .route("/backups/run", post(run_backup_now))
        .route("/backups/status", get(get_backup_status))
        .route("/backups/catalog", get(get_backup_catalog))
        .route("/restore", post(run_restore))
        .route("/metrics/server", get(get_server_metrics))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::admin_auth_middleware,
        ))
        .with_state(state.clone());

    let public_routes = Router::new()
        .route("/hooks/deploy", post(hook_deploy))
        // App object storage, authenticated per app by its storage token
        // (LITEHOUSE_STORAGE_TOKEN) — see `crate::storage`.
        .route("/storage/presign", post(storage_presign))
        .route("/storage/objects", get(storage_list))
        .route(
            "/storage/objects/*key",
            get(storage_get).put(storage_put).delete(storage_delete),
        )
        .with_state(state.clone());

    Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        .layer(DefaultBodyLimit::max(500 * 1024 * 1024)) // 500MB limit for image uploads
}

#[instrument(skip(state))]
async fn list_apps(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match db::app::get_all(&pool).await {
        Ok(apps) => {
            let app_infos = apps
                .into_iter()
                .map(|app| AppInfo {
                    id: app.id.to_string(),
                    name: app.name,
                    state: app.state.to_string(),
                })
                .collect::<Vec<AppInfo>>();
            Json(app_infos).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to list apps: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list apps: {}", e),
            )
                .into_response()
        }
    }
}

/// `GET /api/apps/summary` — one call for everything the admin dashboard's
/// site-list view needs: live (not cached) container state, the best-effort
/// public URL, and the latest deploy's status/sha/time. Dedicated to the SPA
/// rather than folded into `list_apps`/`AppInfo` so the CLI/MCP JSON contract
/// those already serve stays untouched.
#[derive(Debug, Clone, serde::Serialize)]
struct AppSummary {
    id: String,
    name: String,
    state: String,
    url: String,
    last_deploy_status: Option<String>,
    last_deploy_sha: Option<String>,
    last_deploy_at: Option<String>,
}

#[instrument(skip(state))]
async fn apps_summary(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let (pool, config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone())
    };

    let apps = match db::app::get_all(&pool).await {
        Ok(apps) => apps,
        Err(e) => return internal(e).into_response(),
    };

    let mut summaries = Vec::with_capacity(apps.len());
    for app in apps {
        let (last_deploy_status, last_deploy_sha, last_deploy_at) =
            match db::deploy::latest_for_app(&pool, &app.id).await {
                Ok(Some(d)) => (
                    Some(d.status),
                    d.git_sha.as_deref().map(|s| s.chars().take(7).collect()),
                    Some(d.created_at),
                ),
                Ok(None) => (None, None, None),
                Err(_) => (None, None, None),
            };

        // Read-through: reflect the live Docker container state rather than
        // the (possibly stale) cached DB column, same as the HTMX dashboard.
        let live = crate::docker::live_state(&app.name)
            .await
            .unwrap_or(app.state);

        summaries.push(AppSummary {
            id: app.id,
            name: app.name.clone(),
            state: live.to_string(),
            url: app_url(&app.name, &config),
            last_deploy_status,
            last_deploy_sha,
            last_deploy_at,
        });
    }

    Json(summaries).into_response()
}

#[instrument(skip(state))]
async fn get_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => {
            let app_detail = AppDetailResponse {
                id: app.id.to_string(),
                name: app.name,
                state: app.state.to_string(),
                port: app.port,
                created_at: app.created_at.0.to_rfc3339(),
                updated_at: app.updated_at.0.to_rfc3339(),
                repo: app.repo,
                image: app.image,
            };

            Json(app_detail).into_response()
        }
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get app: {}", e),
            )
                .into_response()
        }
    }
}

/// `GET /api/apps/:name/summary` — everything the admin dashboard's app
/// detail page needs about the app itself (its deploys and env vars have
/// their own endpoints): live container state, best-effort public URL, and
/// custom domains, alongside the same static fields `get_app` returns. Kept
/// separate from `get_app` (CLI/MCP-facing) so this SPA-only contract can
/// evolve independently — same split as `apps_summary` vs `list_apps`.
#[derive(Debug, Clone, serde::Serialize)]
struct AppDetailSummary {
    id: String,
    name: String,
    state: String,
    url: String,
    image: Option<String>,
    repo: Option<String>,
    port: Option<i64>,
    custom_domains: Vec<String>,
}

#[instrument(skip(state))]
async fn app_summary(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (pool, config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone())
    };

    let app = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("App '{}' not found", name)).into_response();
        }
        Err(e) => return internal(e).into_response(),
    };

    // Read-through: reflect the live Docker container state rather than the
    // (possibly stale) cached DB column, same as `apps_summary`.
    let live = crate::docker::live_state(&app.name)
        .await
        .unwrap_or(app.state);

    Json(AppDetailSummary {
        url: app_url(&app.name, &config),
        state: live.to_string(),
        custom_domains: app.custom_domains_list(),
        id: app.id,
        name: app.name.clone(),
        image: app.image,
        repo: app.repo,
        port: app.port,
    })
    .into_response()
}

/// `GET /api/apps/:name/metrics?hours=24` — raw resource samples scoped to
/// one app (`scope = app.id`), for the app detail page's sparkline charts.
/// Same shape and `hours` clamp as `/api/metrics/server`.
#[instrument(skip(state))]
async fn get_app_metrics(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    let app = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("App '{}' not found", name)).into_response();
        }
        Err(e) => return internal(e).into_response(),
    };

    let hours = params
        .get("hours")
        .and_then(|h| h.parse::<i64>().ok())
        .unwrap_or(24)
        .clamp(1, 720);
    let since = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();

    match db::metrics::list_samples_since(&pool, &app.id, &since).await {
        Ok(samples) => Json(samples).into_response(),
        Err(e) => {
            tracing::error!("Failed to list metrics for app '{}': {e:#}", name);
            internal(e).into_response()
        }
    }
}

#[instrument(skip(state))]
async fn start_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;

    match start::execute(&pool, &docker, &name).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("App '{}' started", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to start app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to start app: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state))]
async fn stop_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (pool, docker_conn, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;

    match stop::execute(&pool, &docker_conn, &name).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("App '{}' stopped", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to stop app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to stop app: {}", e),
            )
                .into_response()
        }
    }
}

/// `POST /api/apps/:name/restart` — stop then start under the same app
/// lock, mirroring the admin UI's restart button (`ui::restart_app_ui`) but
/// as a JSON endpoint for the SPA. Not a redeploy: same image, fresh
/// container.
#[instrument(skip(state))]
async fn restart_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let (pool, docker_conn, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };
    let _guard = crate::commands::server::lock_app(&locks, &name).await;

    if let Err(e) = stop::execute(&pool, &docker_conn, &name).await {
        tracing::error!("Failed to restart app '{}' (stop step): {}", name, e);
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to restart app: {}", e),
        )
            .into_response();
    }
    match start::execute(&pool, &docker_conn, &name).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("App '{}' restarted", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to restart app '{}' (start step): {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to restart app: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state, name))]
async fn get_logs(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let lines = params
        .get("lines")
        .and_then(|l| l.parse::<usize>().ok())
        .unwrap_or(50);
    let follow = params
        .get("follow")
        .and_then(|f| f.parse::<bool>().ok())
        .unwrap_or(false);

    if follow {
        // Stream logs directly from Docker to the HTTP response body
        match logs::execute(&pool, &name, lines, true).await {
            Ok(stream) => {
                let app_name = name.clone();
                let body_stream = stream.map(move |item| match item {
                    Ok(data) => Ok(Bytes::from(data.into_bytes())),
                    Err(e) => {
                        tracing::warn!("Error reading log stream for {}: {}", app_name, e);
                        Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                    }
                });
                let body = StreamBody::new(body_stream);
                axum::response::Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(axum::body::boxed(body))
                    .unwrap()
                    .into_response()
            }
            Err(e) => (StatusCode::NOT_FOUND, format!("Failed to get logs: {}", e)).into_response(),
        }
    } else {
        // Get logs as a single response
        match logs::execute(&pool, &name, lines, false).await {
            Ok(stream) => {
                let mut logs = String::new();
                let mut stream = stream;
                while let Some(result) = stream.next().await {
                    match result {
                        Ok(data) => logs.push_str(&data),
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Error reading logs: {}", e),
                            )
                                .into_response();
                        }
                    }
                }
                (StatusCode::OK, logs).into_response()
            }
            Err(e) => (StatusCode::NOT_FOUND, format!("Failed to get logs: {}", e)).into_response(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeployRequest {
    image: String,
    sha: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct DeployResponse {
    status: String,
    deploy_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Admin-triggered redeploy: `POST /api/apps/:name/deploy` with
/// `{"image": "...", "sha": "..."}`. Protected by the admin token.
#[instrument(skip(state, payload))]
async fn deploy_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Json(payload): Json<DeployRequest>,
) -> impl IntoResponse {
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };

    match db::app::get_by_name(&pool, &name).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("App '{}' not found", name)).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    }

    let _guard = crate::commands::server::lock_app(&locks, &name).await;

    match crate::deploy::deploy_app(&pool, &docker, &name, &payload.image, payload.sha.as_deref())
        .await
    {
        Ok(deploy) if deploy.status == "succeeded" => (
            StatusCode::OK,
            Json(DeployResponse {
                status: deploy.status,
                deploy_id: deploy.id,
                error: None,
            }),
        )
            .into_response(),
        Ok(deploy) => (
            StatusCode::BAD_GATEWAY,
            Json(DeployResponse {
                status: deploy.status,
                deploy_id: deploy.id,
                error: deploy.error,
            }),
        )
            .into_response(),
        Err(e) => internal(e).into_response(),
    }
}

#[derive(Debug, serde::Serialize)]
struct DeployListItem {
    id: String,
    image: String,
    git_sha: Option<String>,
    status: String,
    error: Option<String>,
    created_at: String,
    updated_at: String,
    log: String,
}

impl From<crate::models::Deploy> for DeployListItem {
    fn from(d: crate::models::Deploy) -> Self {
        Self {
            id: d.id,
            image: d.image,
            git_sha: d.git_sha,
            status: d.status,
            error: d.error,
            created_at: d.created_at,
            updated_at: d.updated_at,
            log: d.log,
        }
    }
}

/// `GET /api/apps/:name/deploys?limit=20` — deploy history for an app.
#[instrument(skip(state))]
async fn list_deploys(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let limit = params
        .get("limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(20);

    let app = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("App '{}' not found", name)).into_response();
        }
        Err(e) => return internal(e).into_response(),
    };

    match db::deploy::list_for_app(&pool, &app.id, limit).await {
        Ok(deploys) => Json(
            deploys
                .into_iter()
                .map(DeployListItem::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => internal(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct HookDeployRequest {
    app: String,
    image: String,
    sha: Option<String>,
}

/// Public, per-app-token-authenticated deploy hook for GitHub Actions:
/// `POST /api/hooks/deploy` with `Authorization: Bearer <app deploy token>`
/// and `{"app": "...", "image": "...", "sha": "..."}`.
///
/// This is the security boundary between GitHub and the server, so error
/// responses are deliberately uniform: a missing token, an unknown app, and
/// a wrong token all return the same generic 401 body — an attacker probing
/// this route learns nothing about which app names exist.
#[instrument(skip(state, headers, payload))]
async fn hook_deploy(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<HookDeployRequest>,
) -> impl IntoResponse {
    let (pool, docker, locks) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.app_locks.clone())
    };

    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response()
    };

    // Extract the bearer token before anything app-related is observable.
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    let token = match token {
        Some(t) => t,
        None => return unauthorized(),
    };

    let app = match db::app::get_by_name(&pool, &payload.app).await {
        Ok(app) => app,
        Err(e) => return internal(e).into_response(),
    };

    if !hook_authorized(token, app.as_ref()) {
        return unauthorized();
    }
    // Authorized implies the app exists (`hook_authorized` is false for None).
    let app = app.expect("authorized deploy hook implies the app exists");

    // Constrain the image to the app's own GHCR namespace. Without this, a
    // leaked per-app deploy token could deploy an arbitrary image (attacker
    // code) under this app's identity and data volume. Reported post-auth, so
    // the uniform-401 enumeration guarantee above is unaffected.
    if !crate::deploy::image_matches_repo(&payload.image, app.repo.as_deref()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "image does not belong to this app's repository"
            })),
        )
            .into_response();
    }

    let _guard = crate::commands::server::lock_app(&locks, &payload.app).await;

    match crate::deploy::deploy_app(
        &pool,
        &docker,
        &payload.app,
        &payload.image,
        payload.sha.as_deref(),
    )
    .await
    {
        Ok(deploy) if deploy.status == "succeeded" => (
            StatusCode::OK,
            Json(DeployResponse {
                status: deploy.status,
                deploy_id: deploy.id,
                error: None,
            }),
        )
            .into_response(),
        Ok(deploy) => (
            StatusCode::BAD_GATEWAY,
            Json(DeployResponse {
                status: deploy.status,
                deploy_id: deploy.id,
                error: deploy.error,
            }),
        )
            .into_response(),
        Err(e) => internal(e).into_response(),
    }
}

/// Deploy-hook authorization: true only when the app exists AND the token
/// matches its stored deploy-token hash. An unknown app is indistinguishable
/// from a wrong token by construction.
fn hook_authorized(token: &str, app: Option<&crate::models::App>) -> bool {
    app.map(|a| crate::deploy::verify_deploy_token(token, a.deploy_token_hash.as_deref()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------
// App object storage (`/api/storage/...`)
// ---------------------------------------------------------------------

fn storage_error(status: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

/// Authenticate the app's storage token and open its namespaced view of the
/// bucket. Like the deploy hook, every auth failure is the same 401.
async fn storage_for_request(
    state: &Arc<RwLock<AppState>>,
    headers: &axum::http::HeaderMap,
) -> Result<crate::storage::AppStorage, axum::response::Response> {
    let pool = state.read().await.db_pool.clone();
    let unauthorized = || storage_error(StatusCode::UNAUTHORIZED, "unauthorized");
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(unauthorized)?;
    let app = match crate::storage::authenticate(&pool, token).await {
        Ok(Some(app)) => app,
        Ok(None) => return Err(unauthorized()),
        Err(e) => {
            tracing::error!("storage auth failed: {e:#}");
            return Err(storage_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"));
        }
    };
    match db_system_config::get_s3_config(&pool).await {
        Ok(Some(cfg)) => Ok(crate::storage::AppStorage::new(&cfg, &app.name)),
        Ok(None) => Err(storage_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "S3 is not configured on this litehouse server (lh config s3 set)",
        )),
        Err(e) => {
            tracing::error!("failed to load S3 config: {e:#}");
            Err(storage_error(StatusCode::INTERNAL_SERVER_ERROR, "internal error"))
        }
    }
}

fn storage_backend_error(e: anyhow::Error) -> axum::response::Response {
    tracing::error!("app storage S3 error: {e:#}");
    storage_error(StatusCode::BAD_GATEWAY, "storage backend error")
}

fn request_content_type(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
}

#[derive(Debug, Deserialize)]
struct StoragePresignRequest {
    key: String,
    method: crate::storage::PresignMethod,
    expires_in_secs: Option<u64>,
    /// PUT only: signed into the URL, so the uploader must send the same
    /// `Content-Type`.
    content_type: Option<String>,
}

/// `POST /api/storage/presign` — a presigned S3 URL for one object in the
/// caller's namespace, so large bodies (or browser uploads) go straight to
/// S3 instead of through the server.
#[instrument(skip(state, headers, req))]
async fn storage_presign(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<StoragePresignRequest>,
) -> impl IntoResponse {
    let storage = match storage_for_request(&state, &headers).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(msg) = crate::storage::validate_object_key(&req.key) {
        return storage_error(StatusCode::BAD_REQUEST, msg);
    }
    let expires = req
        .expires_in_secs
        .unwrap_or(crate::storage::DEFAULT_PRESIGN_EXPIRES_SECS);
    if expires == 0 || expires > crate::storage::MAX_PRESIGN_EXPIRES_SECS {
        return storage_error(
            StatusCode::BAD_REQUEST,
            format!(
                "expires_in_secs must be between 1 and {}",
                crate::storage::MAX_PRESIGN_EXPIRES_SECS
            ),
        );
    }
    match storage
        .presign(req.method, &req.key, expires, req.content_type.as_deref())
        .await
    {
        Ok(presigned) => Json(presigned).into_response(),
        Err(e) => storage_backend_error(e),
    }
}

/// `GET /api/storage/objects?prefix=...` — list the caller's objects
/// (app-relative keys).
#[instrument(skip(state, headers))]
async fn storage_list(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let storage = match storage_for_request(&state, &headers).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let prefix = params.get("prefix").map(String::as_str).unwrap_or("");
    if let Err(msg) = crate::storage::validate_list_prefix(prefix) {
        return storage_error(StatusCode::BAD_REQUEST, msg);
    }
    match storage.list(prefix).await {
        Ok(objects) => Json(serde_json::json!({ "objects": objects })).into_response(),
        Err(e) => storage_backend_error(e),
    }
}

/// `PUT /api/storage/objects/{key}` — upload the request body, proxied
/// through the server. Fine for small objects; use `presign` for big ones.
#[instrument(skip(state, headers, body))]
async fn storage_put(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(key): Path<String>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let storage = match storage_for_request(&state, &headers).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(msg) = crate::storage::validate_object_key(&key) {
        return storage_error(StatusCode::BAD_REQUEST, msg);
    }
    let size = body.len();
    match storage.put(&key, body, request_content_type(&headers)).await {
        Ok(()) => Json(serde_json::json!({ "key": key, "size": size })).into_response(),
        Err(e) => storage_backend_error(e),
    }
}

/// `GET /api/storage/objects/{key}` — download one object.
#[instrument(skip(state, headers))]
async fn storage_get(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(key): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let storage = match storage_for_request(&state, &headers).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(msg) = crate::storage::validate_object_key(&key) {
        return storage_error(StatusCode::BAD_REQUEST, msg);
    }
    match storage.get(&key).await {
        Ok(Some(obj)) => {
            let content_type = obj
                .content_type
                .unwrap_or_else(|| "application/octet-stream".to_string());
            ([(axum::http::header::CONTENT_TYPE, content_type)], obj.bytes).into_response()
        }
        Ok(None) => storage_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => storage_backend_error(e),
    }
}

/// `DELETE /api/storage/objects/{key}` — idempotent.
#[instrument(skip(state, headers))]
async fn storage_delete(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(key): Path<String>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let storage = match storage_for_request(&state, &headers).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(msg) = crate::storage::validate_object_key(&key) {
        return storage_error(StatusCode::BAD_REQUEST, msg);
    }
    match storage.delete(&key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => storage_backend_error(e),
    }
}

/// Uniform 500 mapping for internal errors — never echoes anything sensitive.
fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("internal error: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("internal error: {e}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::App;

    fn app_with_token(token: &str) -> App {
        let mut app = App::new("hooktest").unwrap();
        app.deploy_token_hash = Some(crate::auth::hash_token(token));
        app
    }

    #[test]
    fn new_app_names_must_be_dns_labels() {
        for ok in ["myapp", "my-app", "app2", "a", &"a".repeat(63)] {
            assert!(validate_new_app_name(ok, "admin").is_ok(), "{ok}");
        }
        for bad in ["", "MyApp", "my_app", "my.app", "-app", "app-", "my app", &"a".repeat(64)] {
            assert!(validate_new_app_name(bad, "admin").is_err(), "{bad}");
        }
        assert!(validate_new_app_name("admin", "admin").is_err());
        assert!(validate_new_app_name("admin", "ops").is_ok());
        assert!(validate_new_app_name("ops", "ops").is_err());
    }

    #[test]
    fn hook_authorized_correct_token() {
        let app = app_with_token("tok");
        assert!(hook_authorized("tok", Some(&app)));
    }

    #[test]
    fn hook_authorized_wrong_token() {
        let app = app_with_token("tok");
        assert!(!hook_authorized("nope", Some(&app)));
    }

    #[test]
    fn hook_authorized_unknown_app() {
        assert!(!hook_authorized("tok", None));
    }

    #[test]
    fn hook_authorized_app_without_token() {
        let app = App::new("hooktest2").unwrap();
        assert!(!hook_authorized("tok", Some(&app)));
    }
}

#[instrument(skip(state))]
async fn delete_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match delete::execute(&pool, &name).await {
        Ok(_) => {
            tracing::info!("Successfully deleted app '{}'", name);
            (
                axum::http::StatusCode::OK,
                format!("App '{}' deleted", name),
            )
                .into_response()
        }
        Err(crate::commands::delete::DeleteError::AppRunning(name)) => {
            tracing::warn!("Refused to delete running app '{}'", name);
            (
                axum::http::StatusCode::CONFLICT,
                format!(
                    "App '{}' is running. Stop it first (`lh stop {}`) before deleting.",
                    name, name
                ),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to delete app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete app: {}", e),
            )
                .into_response()
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct CreateAppRequest {
    name: String,
    /// GitHub repo this app deploys from, "owner/name" form.
    #[serde(default)]
    repo: Option<String>,
    /// If the app already exists, mint and return a fresh deploy token
    /// instead of returning 409. Supports idempotent-create in the CLI.
    #[serde(default)]
    rotate_token: bool,
}

#[derive(Debug, serde::Serialize)]
struct CreateAppResponse {
    id: String,
    name: String,
    state: String,
    /// Returned exactly once: at creation, or on an explicit token rotation.
    deploy_token: String,
    url: String,
}

/// App names become a DNS label (`{name}.{domain}`, with a Let's Encrypt
/// certificate) and are interpolated into backup scripts, so new ones must
/// be lowercase letters, digits and hyphens, 1–63 chars, not starting or
/// ending with a hyphen. `admin_label` (the admin UI's subdomain) is taken.
fn validate_new_app_name(name: &str, admin_label: &str) -> Result<(), String> {
    let valid_chars = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if name.is_empty()
        || name.len() > 63
        || !valid_chars
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err(format!(
            "Invalid app name '{name}': use 1-63 lowercase letters, digits and hyphens, \
             not starting or ending with a hyphen (it becomes the app's subdomain)"
        ));
    }
    if name == admin_label {
        return Err(format!("'{name}' is reserved for the admin UI's hostname"));
    }
    Ok(())
}

/// `POST /api/apps` — create a new app and mint its deploy token.
///
/// If `name` already exists: 409 Conflict, unless `rotate_token: true` is
/// set, in which case a fresh deploy token is minted and returned for the
/// existing app (idempotent-create support for the CLI).
#[instrument(skip(state))]
async fn create_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<CreateAppRequest>,
) -> impl IntoResponse {
    let (pool, server_config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone())
    };

    let existing = match db::app::get_by_name(&pool, &payload.name).await {
        Ok(existing) => existing,
        Err(e) => return internal(e).into_response(),
    };

    // Only new names are checked, so apps created before this rule keep working.
    if existing.is_none()
        && let Err(msg) = validate_new_app_name(&payload.name, server_config.admin_label())
    {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }

    if let Some(existing) = existing {
        if !payload.rotate_token {
            return (
                StatusCode::CONFLICT,
                format!("App '{}' already exists", payload.name),
            )
                .into_response();
        }

        // Order matters: persist the repo (full-row save) BEFORE minting the
        // token hash. save() writes every column, so doing it after
        // set_deploy_token_hash would silently revert the fresh hash and leave
        // the GitHub secret out of sync with the server.
        if let Some(repo) = &payload.repo {
            let mut updated = existing.clone();
            updated.repo = Some(repo.clone());
            if let Err(e) = db::app::save(&pool, &updated).await {
                return internal(e).into_response();
            }
        }

        let token = crate::auth::generate_token();
        let hash = crate::auth::hash_token(&token);
        if let Err(e) = db::app::set_deploy_token_hash(&pool, &existing.id, &hash).await {
            return internal(e).into_response();
        }

        return (
            StatusCode::OK,
            Json(CreateAppResponse {
                id: existing.id,
                name: existing.name.clone(),
                state: existing.state.to_string(),
                deploy_token: token,
                url: app_url(&existing.name, &server_config),
            }),
        )
            .into_response();
    }

    if let Err(e) = create::execute(&pool, &payload.name).await {
        tracing::error!("Failed to create app '{}': {}", payload.name, e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create app: {}", e),
        )
            .into_response();
    }

    let mut app = match db::app::get_by_name(&pool, &payload.name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            tracing::error!("App '{}' created but could not be retrieved", payload.name);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "App created but could not be retrieved",
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("App '{}' created but failed to retrieve: {}", payload.name, e);
            return internal(e).into_response();
        }
    };

    if let Some(repo) = &payload.repo {
        app.repo = Some(repo.clone());
        if let Err(e) = db::app::save(&pool, &app).await {
            tracing::error!("App '{}' created but failed to save repo: {}", app.name, e);
            return internal(e).into_response();
        }
    }

    let token = crate::auth::generate_token();
    let hash = crate::auth::hash_token(&token);
    if let Err(e) = db::app::set_deploy_token_hash(&pool, &app.id, &hash).await {
        tracing::error!("App '{}' created but failed to set deploy token: {}", app.name, e);
        return internal(e).into_response();
    }

    (
        StatusCode::CREATED,
        Json(CreateAppResponse {
            id: app.id,
            name: app.name.clone(),
            state: app.state.to_string(),
            deploy_token: token,
            url: app_url(&app.name, &server_config),
        }),
    )
        .into_response()
}

/// Best-effort URL an app will be reachable at, for display purposes only.
fn app_url(app_name: &str, config: &ServerConfig) -> String {
    let local_dev = std::env::var("LITEHOUSE_LOCAL_DEV").is_ok() || cfg!(debug_assertions);
    if local_dev {
        format!("http://{}.localhost:9090", app_name)
    } else if let Some(domain) = &config.domain {
        format!("https://{}.{}", app_name, domain)
    } else {
        format!("https://{}.<configure-domain>", app_name)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct AppInfo {
    id: String,
    name: String,
    state: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct AppDetailResponse {
    id: String,
    name: String,
    state: String,
    port: Option<i64>,
    created_at: String,
    updated_at: String,
    repo: Option<String>,
    image: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct EnvVarResponse {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct SetEnvRequest {
    key: String,
    value: String,
    delete: Option<bool>,
}

async fn get_env(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => {
            match env_var::get_by_app(&pool, &app.id).await {
                Ok(env_vars) => {
                    let response: Vec<EnvVarResponse> = env_vars
                        .iter()
                        .map(|ev| EnvVarResponse {
                            key: ev.key.clone(),
                            value: ev.value.clone(),
                        })
                        .collect();
                    Json(response).into_response()
                }
                Err(e) => {
                    tracing::error!("Failed to get environment variables for app '{}': {}", name, e);
                    (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to get environment variables: {}", e),
                    )
                        .into_response()
                }
            }
        }
        Ok(None) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get app '{}' for env vars: {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get app: {}", e),
            )
                .into_response()
        }
    }
}

async fn set_env(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Json(payload): Json<SetEnvRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match app_env::set_env(
        &pool,
        &name,
        &payload.key,
        &payload.value,
        payload.delete.unwrap_or(false),
    )
    .await
    {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!(
                "Environment variable {} set for app '{}'",
                payload.key, name
            ),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to set environment variable '{}' for app '{}': {}", payload.key, name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to set environment variable: {}", e),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct AddDomainRequest {
    domain: String,
}

/// `GET /api/apps/:name/domains` — list an app's custom top-level domains.
#[instrument(skip(state))]
async fn list_domains(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match domain::list(&pool, &name).await {
        Ok(domains) => Json(domains).into_response(),
        Err(domain::DomainError::AppNotFound(_)) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to list domains for app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list domains: {}", e),
            )
                .into_response()
        }
    }
}

/// `POST /api/apps/:name/domains` — add a custom top-level domain to an
/// app's Caddy route (alongside its derived `{name}.{server_domain}` host)
/// and resync Caddy.
#[instrument(skip(state, payload))]
async fn add_domain(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Json(payload): Json<AddDomainRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match domain::add(&pool, &docker, &name, &payload.domain).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("Domain '{}' added to app '{}'", payload.domain, name),
        )
            .into_response(),
        Err(domain::DomainError::AppNotFound(_)) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e @ domain::DomainError::InvalidDomain(_)) => {
            (axum::http::StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to add domain '{}' to app '{}': {}", payload.domain, name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to add domain: {}", e),
            )
                .into_response()
        }
    }
}

/// `DELETE /api/apps/:name/domains` — remove a custom top-level domain from
/// an app's Caddy route and resync Caddy.
#[instrument(skip(state, payload))]
async fn remove_domain(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Json(payload): Json<AddDomainRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match domain::remove(&pool, &docker, &name, &payload.domain).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("Domain '{}' removed from app '{}'", payload.domain, name),
        )
            .into_response(),
        Err(domain::DomainError::AppNotFound(_)) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e @ domain::DomainError::DomainNotFound(_, _)) => {
            (axum::http::StatusCode::NOT_FOUND, e.to_string()).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to remove domain '{}' from app '{}': {}", payload.domain, name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to remove domain: {}", e),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct SetHealthCheckRequest {
    path: String,
}

/// `GET /api/apps/:name/health-check` — get an app's configured health
/// check path.
#[instrument(skip(state))]
async fn get_health_check(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match health_check::get(&pool, &name).await {
        Ok(path) => Json(path).into_response(),
        Err(health_check::HealthCheckError::AppNotFound(_)) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get health check for app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get health check: {}", e),
            )
                .into_response()
        }
    }
}

/// `POST /api/apps/:name/health-check` — set (or replace) an app's health
/// check path and resync Caddy.
#[instrument(skip(state, payload))]
async fn set_health_check(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    Json(payload): Json<SetHealthCheckRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match health_check::set(&pool, &docker, &name, &payload.path).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("Health check path '{}' set on app '{}'", payload.path, name),
        )
            .into_response(),
        Err(health_check::HealthCheckError::AppNotFound(_)) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e @ health_check::HealthCheckError::InvalidPath(_)) => {
            (axum::http::StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to set health check for app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to set health check: {}", e),
            )
                .into_response()
        }
    }
}

/// `DELETE /api/apps/:name/health-check` — clear an app's health check path
/// and resync Caddy.
#[instrument(skip(state))]
async fn unset_health_check(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match health_check::unset(&pool, &docker, &name).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("Health check path cleared on app '{}'", name),
        )
            .into_response(),
        Err(health_check::HealthCheckError::AppNotFound(_)) => (
            axum::http::StatusCode::NOT_FOUND,
            format!("App '{}' not found", name),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to clear health check for app '{}': {}", name, e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to clear health check: {}", e),
            )
                .into_response()
        }
    }
}

async fn get_docker_version(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let docker = state.read().await.docker.clone();
    match docker.version().await {
        Ok(v) => v.version.unwrap_or_else(|| "unknown".to_string()),
        Err(e) => {
            tracing::warn!("Failed to read Docker version: {e}");
            "unknown".to_string()
        }
    }
}

/// `GET /api/server/info` — SPA-only. What the admin UI's settings page
/// shows about the server it is talking to: which litehouse build is
/// running, the wildcard domain apps are published under, the Docker
/// version underneath, and a one-line app census. Cheap enough to poll;
/// nothing here is secret (the S3/GHCR endpoints stay redacted).
#[derive(Debug, serde::Serialize)]
struct ServerInfoResponse {
    version: String,
    /// Wildcard domain apps are published under (`None` until `lh install
    /// --domain ...` has run), e.g. "lh.danbruder.com".
    domain: Option<String>,
    /// Host the admin UI itself is served from, when a domain is configured.
    admin_host: Option<String>,
    docker_version: Option<String>,
    /// Docker host platform as `os/arch` (e.g. "linux/arm64"). `lh create`
    /// renders the app's deploy workflow to build images for this platform.
    platform: Option<String>,
    apps_total: usize,
    apps_running: usize,
    local_dev: bool,
}

#[instrument(skip(state))]
async fn get_server_info(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let (pool, config, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.server_config.clone(), s.docker.clone())
    };

    let apps = db::app::get_all(&pool).await.unwrap_or_default();
    let apps_total = apps.len();
    let mut apps_running = 0;
    for app in &apps {
        let live = crate::docker::live_state(&app.name)
            .await
            .unwrap_or(app.state);
        if matches!(live, crate::models::AppState::Running) {
            apps_running += 1;
        }
    }

    let docker_version = docker.version().await.ok().and_then(|v| v.version);
    let platform = crate::docker::host_platform(&docker).await;

    Json(ServerInfoResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        admin_host: config
            .domain
            .as_ref()
            .map(|d| format!("{}.{}", config.admin_label(), d)),
        domain: config.domain.clone(),
        docker_version,
        platform,
        apps_total,
        apps_running,
        local_dev: std::env::var("LITEHOUSE_LOCAL_DEV").is_ok() || cfg!(debug_assertions),
    })
    .into_response()
}

#[derive(Debug, serde::Deserialize)]
struct SetS3ConfigRequest {
    access_key_id: String,
    secret_access_key: String,
    bucket: String,
    region: String,
    endpoint: Option<String>,
    path_prefix: Option<String>,
}

#[instrument(skip(state))]
async fn set_s3_config(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<SetS3ConfigRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Create S3 config
    let s3_config = S3Config {
        access_key_id: payload.access_key_id,
        secret_access_key: payload.secret_access_key,
        bucket: payload.bucket,
        region: payload.region,
        endpoint: payload.endpoint,
        path_prefix: payload.path_prefix,
    };

    // Create system config
    let system_config = SystemConfig::new_s3_config(&s3_config);

    // Save to database
    match db_system_config::save_s3_config(&pool, &system_config).await {
        Ok(_) => (StatusCode::OK, "S3 configuration saved successfully").into_response(),
        Err(e) => {
            tracing::error!("Failed to save S3 config: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save S3 config: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state))]
async fn get_s3_config(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db_system_config::get_s3_config(&pool).await {
        Ok(Some(config)) => {
            let redacted = S3ConfigRedacted::from(&config);
            Json(redacted).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "No S3 configuration found").into_response(),
        Err(e) => {
            tracing::error!("Failed to get S3 config: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get S3 config: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state))]
async fn delete_s3_config(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db_system_config::delete_s3_config(&pool).await {
        Ok(_) => (StatusCode::OK, "S3 configuration deleted successfully").into_response(),
        Err(e) => {
            tracing::error!("Failed to delete S3 config: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete S3 config: {}", e),
            )
                .into_response()
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct SetGhcrConfigRequest {
    token: String,
}

#[derive(Debug, serde::Serialize)]
struct GhcrConfigResponse {
    configured: bool,
    token: Option<String>,
}

/// Redact a token, keeping only its *type* prefix (e.g. "ghp_", "github_pat_")
/// so operators can tell what kind of token is configured without any secret
/// material crossing the API.
fn redact_token(token: &str) -> String {
    let head: String = token.chars().take(11).collect();
    match head.rfind('_') {
        Some(idx) => format!("{}****", &head[..=idx]),
        None => "****".to_string(),
    }
}

#[instrument(skip(state, payload))]
async fn set_ghcr_config(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<SetGhcrConfigRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db_system_config::set_ghcr_token(&pool, &payload.token).await {
        Ok(_) => (StatusCode::OK, "GHCR token saved successfully").into_response(),
        Err(e) => {
            tracing::error!("Failed to save GHCR token: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save GHCR token: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state))]
async fn get_ghcr_config(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db_system_config::get_ghcr_token(&pool).await {
        Ok(Some(token)) => Json(GhcrConfigResponse {
            configured: true,
            token: Some(redact_token(&token)),
        })
        .into_response(),
        Ok(None) => Json(GhcrConfigResponse {
            configured: false,
            token: None,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("Failed to get GHCR token: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get GHCR token: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state))]
async fn delete_ghcr_config(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db_system_config::delete_ghcr_token(&pool).await {
        Ok(_) => (StatusCode::OK, "GHCR token deleted successfully").into_response(),
        Err(e) => {
            tracing::error!("Failed to delete GHCR token: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete GHCR token: {}", e),
            )
                .into_response()
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct BackupStatusResponse {
    last_backup_date: Option<String>,
    last_backup_report: Option<crate::backup::BackupReport>,
}

/// `POST /api/backups/run` — run a full backup synchronously and return its
/// report. Distinct from the hourly scheduler in `commands::server::execute`;
/// this is for operators who want an on-demand backup (or a fresh one right
/// after configuring S3) without waiting for the next tick.
#[instrument(skip(state))]
async fn run_backup_now(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match crate::backup::run_backup(&pool, &docker).await {
        Ok(report) => {
            // Same gate as the hourly scheduler: a run with any failures
            // doesn't count as "today's backup done" — the scheduler should
            // still retry it next hour.
            if report.failed.is_empty() {
                let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                if let Err(e) = db_system_config::set_last_backup_date(&pool, &today).await {
                    tracing::warn!("failed to record last_backup_date after manual run: {e:#}");
                }
            }
            Json(report).into_response()
        }
        Err(e) => {
            tracing::error!("Manual backup run failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Backup failed: {e:#}"),
            )
                .into_response()
        }
    }
}

/// `GET /api/backups/status` — the last recorded backup date and report,
/// without triggering a new run.
#[instrument(skip(state))]
async fn get_backup_status(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    let last_backup_date = match db_system_config::get_last_backup_date(&pool).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to get last_backup_date: {e:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get backup status: {e:#}"),
            )
                .into_response();
        }
    };
    let last_backup_report = match db_system_config::get_last_backup_report(&pool).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to get last_backup_report: {e:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get backup status: {e:#}"),
            )
                .into_response();
        }
    };

    Json(BackupStatusResponse {
        last_backup_date,
        last_backup_report,
    })
    .into_response()
}

/// `GET /api/backups/catalog` — every catalogued backup artifact (app,
/// object key, size, age), for the admin dashboard's backups page. Distinct
/// from `/api/backups/status` (today's pass/fail summary): this is the full
/// historical listing, SPA-only since neither the CLI nor MCP expose it yet.
#[instrument(skip(state))]
async fn get_backup_catalog(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match db::backup::list_all(&pool).await {
        Ok(records) => Json(records).into_response(),
        Err(e) => {
            tracing::error!("Failed to list backup catalog: {e:#}");
            internal(e).into_response()
        }
    }
}

/// `POST /api/restore` — run a full disaster-recovery restore from S3 and
/// return its report. See `backup::restore_all`.
#[instrument(skip(state))]
async fn run_restore(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match crate::backup::restore_all(&pool, &docker).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => {
            tracing::error!("Restore failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Restore failed: {e:#}"),
            )
                .into_response()
        }
    }
}

/// `GET /api/metrics/server?hours=24` — raw server resource samples
/// (`scope = "server"`), oldest first, for the admin dashboard's sparkline
/// cards. `hours` defaults to 24 and is clamped to [1, 720] (30 days) so a
/// bad query param can't trigger an unbounded scan.
#[instrument(skip(state))]
async fn get_server_metrics(
    State(state): State<Arc<RwLock<AppState>>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let hours = params
        .get("hours")
        .and_then(|h| h.parse::<i64>().ok())
        .unwrap_or(24)
        .clamp(1, 720);
    let since = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();

    match db::metrics::list_samples_since(&pool, "server", &since).await {
        Ok(samples) => Json(samples).into_response(),
        Err(e) => {
            tracing::error!("Failed to list server metrics: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list server metrics: {e:#}"),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod storage_api_tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::App;
    use axum::body::Body;
    use axum::http::{header, Method, Request};
    use tower::ServiceExt;

    async fn test_state() -> Arc<RwLock<AppState>> {
        Arc::new(RwLock::new(AppState {
            db_pool: get_test_pool().await,
            docker: crate::docker::connect().await.expect("connect to docker"),
            admin_token_hash: crate::auth::hash_token("admin"),
            server_config: ServerConfig::default(),
            app_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }))
    }

    async fn send(
        state: &Arc<RwLock<AppState>>,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Body,
    ) -> (StatusCode, Bytes) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if uri.ends_with("/presign") {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let resp = create_api_router(state.clone())
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        (status, hyper::body::to_bytes(resp.into_body()).await.unwrap())
    }

    async fn app_and_token(state: &Arc<RwLock<AppState>>, name: &str) -> (App, String) {
        let pool = state.read().await.db_pool.clone();
        let app = App::new(name).unwrap();
        db::app::save(&pool, &app).await.unwrap();
        let token = crate::storage::token_for_app(&pool, &app).await.unwrap();
        (app, token)
    }

    #[tokio::test]
    async fn storage_rejects_missing_and_bad_tokens_uniformly() {
        let state = test_state().await;
        let (app, token) = app_and_token(&state, "store-auth").await;

        let (s, _) = send(&state, Method::GET, "/storage/objects", None, Body::empty()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // The admin token is not a storage token.
        let (s, _) = send(&state, Method::GET, "/storage/objects", Some("admin"), Body::empty()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        // A token minted with a different secret is rejected.
        let forged = crate::storage::mint_token("not-the-secret", &app.id);
        let (s, _) = send(&state, Method::GET, "/storage/objects", Some(&forged), Body::empty()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);

        // A deleted app's token stops working.
        let pool = state.read().await.db_pool.clone();
        db::app::delete_by_app_id(&pool, &app.id).await.unwrap();
        let (s, _) = send(&state, Method::GET, "/storage/objects", Some(&token), Body::empty()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn storage_is_unavailable_without_s3_config() {
        let state = test_state().await;
        let (_, token) = app_and_token(&state, "store-nos3").await;
        let (s, _) = send(&state, Method::GET, "/storage/objects", Some(&token), Body::empty()).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// End to end against a real S3 API: two apps share the bucket but only
    /// see their own `storage/{app}/` namespace, and nothing lands under the
    /// backup roots.
    ///
    /// Uses `LITEHOUSE_TEST_S3_ENDPOINT` (minioadmin/minioadmin credentials)
    /// when set, otherwise starts MinIO in Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test test_app_storage_roundtrip -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_app_storage_roundtrip() {
        let minio_container = "litehouse-storage-test-minio";
        let endpoint = match std::env::var("LITEHOUSE_TEST_S3_ENDPOINT") {
            Ok(e) => e,
            Err(_) => {
                let _ = std::process::Command::new("docker").args(["rm", "-f", minio_container]).output();
                let status = std::process::Command::new("docker")
                    .args([
                        "run", "-d", "--rm", "--name", minio_container, "-p", "19001:9000",
                        "-e", "MINIO_ROOT_USER=minioadmin", "-e", "MINIO_ROOT_PASSWORD=minioadmin",
                        "minio/minio", "server", "/data",
                    ])
                    .status()
                    .expect("start minio");
                assert!(status.success(), "failed to start minio container");
                for _ in 0..60 {
                    if std::net::TcpStream::connect(("127.0.0.1", 19001)).is_ok() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                "http://localhost:19001".to_string()
            }
        };

        let state = test_state().await;
        let pool = state.read().await.db_pool.clone();
        let s3_config = S3Config {
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            bucket: "litehouse-storage-test".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some(endpoint),
            path_prefix: Some("lh".to_string()),
        };
        let client = crate::backup::s3_client(&s3_config);
        let _ = client.create_bucket().bucket(&s3_config.bucket).send().await;
        db_system_config::save_s3_config(&pool, &SystemConfig::new_s3_config(&s3_config))
            .await
            .unwrap();

        let (_, token_a) = app_and_token(&state, "store-a").await;
        let (_, token_b) = app_and_token(&state, "store-b").await;

        // Proxied upload + download.
        let (s, _) = send(
            &state,
            Method::PUT,
            "/storage/objects/avatars/1.txt",
            Some(&token_a),
            Body::from("hello from a"),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let (s, body) =
            send(&state, Method::GET, "/storage/objects/avatars/1.txt", Some(&token_a), Body::empty()).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(&body[..], b"hello from a");

        // App B can't see A's object.
        let (s, _) =
            send(&state, Method::GET, "/storage/objects/avatars/1.txt", Some(&token_b), Body::empty()).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, body) = send(&state, Method::GET, "/storage/objects", Some(&token_b), Body::empty()).await;
        let listed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(listed["objects"].as_array().unwrap().len(), 0);

        // Escapes are rejected (axum percent-decodes the wildcard).
        let (s, _) = send(
            &state,
            Method::PUT,
            "/storage/objects/..%2F..%2Fapps%2Fstore-a%2F2099-01-01.tar.gz",
            Some(&token_a),
            Body::from("x"),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // The object really lives under storage/{app}/, nowhere near backups.
        let raw = client
            .list_objects_v2()
            .bucket(&s3_config.bucket)
            .send()
            .await
            .unwrap();
        let keys: Vec<&str> = raw.contents().iter().filter_map(|o| o.key()).collect();
        assert_eq!(keys, vec!["lh/storage/store-a/avatars/1.txt"]);

        // Listing returns app-relative keys, filtered by prefix.
        let (_, body) =
            send(&state, Method::GET, "/storage/objects?prefix=avatars/", Some(&token_a), Body::empty()).await;
        let listed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(listed["objects"][0]["key"], "avatars/1.txt");
        assert_eq!(listed["objects"][0]["size"], 12);

        // Presigned PUT, then presigned GET, straight against S3.
        let (s, body) = send(
            &state,
            Method::POST,
            "/storage/presign",
            Some(&token_a),
            Body::from(r#"{"key":"docs/report.txt","method":"PUT","content_type":"text/plain"}"#),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let presigned: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(presigned["method"], "PUT");
        let http = reqwest::Client::new();
        let put = http
            .put(presigned["url"].as_str().unwrap())
            .header("content-type", "text/plain")
            .body("presigned body")
            .send()
            .await
            .unwrap();
        assert!(put.status().is_success(), "presigned PUT failed: {}", put.status());

        let (_, body) = send(
            &state,
            Method::POST,
            "/storage/presign",
            Some(&token_a),
            Body::from(r#"{"key":"docs/report.txt","method":"GET","expires_in_secs":60}"#),
        )
        .await;
        let presigned: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let got = http.get(presigned["url"].as_str().unwrap()).send().await.unwrap();
        assert_eq!(got.text().await.unwrap(), "presigned body");

        // Out-of-range expiry is rejected.
        let (s, _) = send(
            &state,
            Method::POST,
            "/storage/presign",
            Some(&token_a),
            Body::from(r#"{"key":"x","method":"GET","expires_in_secs":999999999}"#),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // Delete is idempotent.
        for _ in 0..2 {
            let (s, _) =
                send(&state, Method::DELETE, "/storage/objects/avatars/1.txt", Some(&token_a), Body::empty())
                    .await;
            assert_eq!(s, StatusCode::NO_CONTENT);
        }
        let (s, _) =
            send(&state, Method::GET, "/storage/objects/avatars/1.txt", Some(&token_a), Body::empty()).await;
        assert_eq!(s, StatusCode::NOT_FOUND);

        let _ = std::process::Command::new("docker").args(["rm", "-f", minio_container]).output();
    }
}
