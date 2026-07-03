use crate::commands::app_env;
use crate::commands::create;
use crate::commands::delete;
use crate::commands::logs;
use crate::commands::server::AppState;
use crate::commands::{start, stop};
use crate::db;
use crate::db::env_var;
use crate::db::system_config as db_system_config;
use crate::models::{S3Config, S3ConfigRedacted, SystemConfig};
use axum::body::StreamBody;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::{
    extract::{Multipart, Path, Query, State},
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
    // Public routes (no authentication required)
    let public_routes = Router::new()
        .route("/auth/status", get(auth_status_handler))
        .route("/auth/register", post(register_handler))
        .route("/auth/login", post(login_handler))
        .route("/auth/refresh", post(refresh_token_handler))
        .with_state(state.clone());

    // Protected routes (require authentication)
    let protected_routes = Router::new()
        .route("/auth/logout", post(logout_handler))
        .route("/auth/me", get(get_current_user))
        .route("/apps", get(list_apps))
        .route("/apps", post(create_app))
        .route("/apps/:name", get(get_app))
        .route("/apps/:name", delete(delete_app))
        .route("/apps/:name/start", post(start_app))
        .route("/apps/:name/stop", post(stop_app))
        .route("/apps/:name/logs", get(get_logs))
        .route("/apps/:name/deploy", post(deploy_app))
        .route("/apps/:name/env", post(set_env))
        .route("/apps/:name/env", get(get_env))
        .route("/docker/version", get(get_docker_version))
        .route("/config/s3", post(set_s3_config))
        .route("/config/s3", get(get_s3_config))
        .route("/config/s3", delete(delete_s3_config))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .with_state(state.clone());

    // Combine routes
    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
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
            // Try to get remote info for this app
            let remote_info = match db::remote::get_by_app(&pool, &app.id).await {
                Ok(remote) => Some(RemoteInfoResponse {
                    name: remote.name,
                    url: remote.remote,
                    branch: remote.branch,
                }),
                Err(_) => None,
            };

            let app_detail = AppDetailResponse {
                id: app.id.to_string(),
                name: app.name,
                state: app.state.to_string(),
                port: app.port,
                created_at: app.created_at.0.to_rfc3339(),
                updated_at: app.updated_at.0.to_rfc3339(),
                remote: remote_info,
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

#[instrument(skip(state, multipart))]
async fn deploy_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let (pool, docker) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone())
    };

    // Parse multipart form: image tarball + metadata fields
    let mut image_data: Option<Bytes> = None;
    let mut image_tag: Option<String> = None;
    let mut git_commit: Option<String> = None;
    let mut no_start = false;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("image") => match field.bytes().await {
                Ok(bytes) => {
                    tracing::info!("Received image tarball: {} bytes", bytes.len());
                    image_data = Some(bytes);
                }
                Err(e) => {
                    tracing::error!("Error reading image field: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Error reading image data")
                        .into_response();
                }
            },
            Some("image_tag") => {
                if let Ok(text) = field.text().await {
                    image_tag = Some(text);
                }
            }
            Some("git_commit") => {
                if let Ok(text) = field.text().await {
                    git_commit = Some(text);
                }
            }
            Some("no_start") => {
                if let Ok(text) = field.text().await {
                    no_start = text == "true";
                }
            }
            _ => {}
        }
    }

    let image_bytes = match image_data {
        Some(data) => data,
        None => {
            return (StatusCode::BAD_REQUEST, "No image tarball provided").into_response();
        }
    };

    // Verify app exists
    let app = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("App '{}' not found", name),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            )
                .into_response();
        }
    };

    // Save tarball to temp file
    let temp_dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create temp dir: {}", e),
            )
                .into_response();
        }
    };
    let tarball_path = temp_dir.path().join("image.tar");
    if let Err(e) = std::fs::write(&tarball_path, &image_bytes) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write tarball: {}", e),
        )
            .into_response();
    }

    // Load image into Docker
    let tarball_str = tarball_path.to_string_lossy().to_string();
    let loaded_tag = match crate::docker::load_image(&tarball_str).await {
        Ok(tag) => {
            tracing::info!("Loaded Docker image: {}", tag);
            tag
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load Docker image: {}", e),
            )
                .into_response();
        }
    };

    // Use provided image_tag or fall back to what docker load reported
    let final_tag = image_tag.unwrap_or(loaded_tag);

    // Detect exposed port from the loaded image
    let exposed_port = match crate::docker::get_exposed_port(&final_tag).await {
        Ok(port) => {
            tracing::info!("Detected exposed port {} for image {}", port, final_tag);
            Some(port)
        }
        Err(e) => {
            tracing::warn!("Failed to detect exposed port: {}", e);
            None
        }
    };

    // Create build record
    let mut build_record = crate::models::Build::new_success(
        app.id.clone(),
        final_tag.clone(),
        git_commit.unwrap_or_default(),
    );
    if let Some(port) = exposed_port {
        build_record.set_exposed_port(port);
    }

    if let Err(e) = db::build::save(&pool, &build_record).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save build record: {}", e),
        )
            .into_response();
    }

    tracing::info!("Created build record {} for app '{}'", build_record.id, name);

    // Auto-start unless --no-start
    if !no_start {
        // Stop existing container if running
        let _ = stop::execute(&name).await;

        match start::execute(&pool, &docker, &name).await {
            Ok(_) => {
                tracing::info!("App '{}' started after deploy", name);
            }
            Err(e) => {
                tracing::error!("Failed to auto-start app '{}': {}", name, e);
                return Json(serde_json::json!({
                    "message": format!("Image deployed but failed to start: {}", e),
                    "build_id": build_record.id,
                    "image_tag": final_tag,
                    "started": false
                }))
                .into_response();
            }
        }
    }

    Json(serde_json::json!({
        "message": format!("App '{}' deployed successfully", name),
        "build_id": build_record.id,
        "image_tag": final_tag,
        "started": !no_start
    }))
    .into_response()
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
}

#[instrument(skip(state))]
async fn create_app(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(_auth_user): axum::Extension<crate::auth::AuthUser>,
    Json(payload): Json<CreateAppRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Create the app
    if let Err(e) = create::execute(&pool, &payload.name).await {
        tracing::error!("Failed to create app '{}': {}", payload.name, e);
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create app: {}", e),
        )
            .into_response();
    }

    // Fetch the newly created app to return its info
    match db::app::get_by_name(&pool, &payload.name).await {
        Ok(Some(app)) => (
            StatusCode::CREATED,
            Json(AppInfo {
                id: app.id,
                name: app.name,
                state: app.state.to_string(),
            }),
        )
            .into_response(),
        Ok(None) => {
            tracing::error!("App '{}' created but could not be retrieved", payload.name);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "App created but could not be retrieved",
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("App '{}' created but failed to retrieve: {}", payload.name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("App created but failed to retrieve: {}", e),
            )
                .into_response()
        }
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
    remote: Option<RemoteInfoResponse>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RemoteInfoResponse {
    name: String,
    url: String,
    branch: String,
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

// ===== AUTH ENDPOINTS =====

#[derive(Debug, serde::Serialize)]
struct AuthStatusResponse {
    initialized: bool,
    version: String,
}

#[instrument(skip(state))]
async fn auth_status_handler(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match crate::db::user::count(&pool).await {
        Ok(count) => {
            let response = AuthStatusResponse {
                initialized: count > 0,
                version: env!("CARGO_PKG_VERSION").to_string(),
            };
            Json(response).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to check server status: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to check server status: {}", e),
            )
                .into_response()
        }
    }
}

#[instrument(skip(state))]
async fn register_handler(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<crate::models::RegisterRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let jwt_secret = state.read().await.jwt_secret.clone();

    match crate::commands::auth::register::execute(
        &pool,
        &payload.email,
        &payload.password,
        payload.full_name,
        payload.organization_name,
        &jwt_secret,
    )
    .await
    {
        Ok(response) => Json(response).into_response(),
        Err(e) => match e {
            crate::commands::auth::register::RegisterError::UserAlreadyExists(_) => {
                (StatusCode::CONFLICT, format!("{}", e)).into_response()
            }
            crate::commands::auth::register::RegisterError::OrganizationAlreadyExists(_) => {
                (StatusCode::CONFLICT, format!("{}", e)).into_response()
            }
            crate::commands::auth::register::RegisterError::UserError(_) => {
                (StatusCode::BAD_REQUEST, format!("{}", e)).into_response()
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)).into_response(),
        },
    }
}

#[instrument(skip(state))]
async fn login_handler(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<crate::models::LoginRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let jwt_secret = state.read().await.jwt_secret.clone();

    match crate::commands::auth::login::execute(
        &pool,
        &payload.email,
        &payload.password,
        &jwt_secret,
    )
    .await
    {
        Ok(response) => Json(response).into_response(),
        Err(e) => match e {
            crate::commands::auth::login::LoginError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, format!("{}", e)).into_response()
            }
            crate::commands::auth::login::LoginError::UserNotActive => {
                (StatusCode::FORBIDDEN, format!("{}", e)).into_response()
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)).into_response(),
        },
    }
}

#[instrument(skip(state))]
async fn logout_handler(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<crate::models::RefreshTokenRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match crate::commands::auth::logout::execute(&pool, &payload.refresh_token).await {
        Ok(_) => (StatusCode::OK, "Logged out successfully").into_response(),
        Err(e) => match e {
            crate::commands::auth::logout::LogoutError::InvalidToken => {
                (StatusCode::UNAUTHORIZED, format!("{}", e)).into_response()
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)).into_response(),
        },
    }
}

#[instrument(skip(state))]
async fn refresh_token_handler(
    State(state): State<Arc<RwLock<AppState>>>,
    Json(payload): Json<crate::models::RefreshTokenRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let jwt_secret = state.read().await.jwt_secret.clone();

    match crate::commands::auth::refresh::execute(&pool, &payload.refresh_token, &jwt_secret).await
    {
        Ok(tokens) => Json(tokens).into_response(),
        Err(e) => match e {
            crate::commands::auth::refresh::RefreshError::InvalidToken
            | crate::commands::auth::refresh::RefreshError::TokenRevoked => {
                (StatusCode::UNAUTHORIZED, format!("{}", e)).into_response()
            }
            crate::commands::auth::refresh::RefreshError::UserNotActive => {
                (StatusCode::FORBIDDEN, format!("{}", e)).into_response()
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)).into_response(),
        },
    }
}

#[instrument(skip(state))]
async fn get_current_user(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match crate::db::user::get_by_id(&pool, &auth_user.user_id).await {
        Ok(Some(user)) => {
            // Get user's organizations
            match crate::db::organization::get_user_organizations(&pool, &user.id).await {
                Ok(orgs) => Json(crate::models::AuthenticatedUser {
                    user,
                    organizations: orgs,
                })
                .into_response(),
                Err(e) => {
                    tracing::error!("Failed to get organizations for user '{}': {}", user.id, e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to get user organizations: {}", e),
                    )
                        .into_response()
                }
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => {
            tracing::error!("Failed to get current user: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get user: {}", e),
            )
                .into_response()
        }
    }
}

