use crate::commands::app_env;
use crate::commands::create;
use crate::commands::delete;
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
        .route("/apps/:name", get(get_app))
        .route("/apps/:name", delete(delete_app))
        .route("/apps/:name/start", post(start_app))
        .route("/apps/:name/stop", post(stop_app))
        .route("/apps/:name/logs", get(get_logs))
        .route("/apps/:name/deploy", post(deploy_app))
        .route("/apps/:name/deploys", get(list_deploys))
        .route("/apps/:name/env", post(set_env))
        .route("/apps/:name/env", get(get_env))
        .route("/docker/version", get(get_docker_version))
        .route("/config/s3", post(set_s3_config))
        .route("/config/s3", get(get_s3_config))
        .route("/config/s3", delete(delete_s3_config))
        .route("/config/ghcr", post(set_ghcr_config))
        .route("/config/ghcr", get(get_ghcr_config))
        .route("/config/ghcr", delete(delete_ghcr_config))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::admin_auth_middleware,
        ))
        .with_state(state.clone());

    let public_routes = Router::new()
        .route("/hooks/deploy", post(hook_deploy))
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

#[instrument(skip(state))]
async fn start_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

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

#[instrument(skip(_state))]
async fn stop_app(
    State(_state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match stop::execute(&name).await {
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

#[instrument(skip(name))]
async fn get_logs(
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
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
        match logs::execute(&name, lines, true).await {
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
        match logs::execute(&name, lines, false).await {
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
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
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
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
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

    if let Some(existing) = existing {
        if !payload.rotate_token {
            return (
                StatusCode::CONFLICT,
                format!("App '{}' already exists", payload.name),
            )
                .into_response();
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

    let app = match db::app::get_by_name(&pool, &payload.name).await {
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
        format!("https://{}.lh.danbruder.com", app_name)
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

async fn get_docker_version() -> impl IntoResponse {
    // TODO: Implement proper Docker version check
    "docker"
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
