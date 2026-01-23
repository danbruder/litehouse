use crate::commands::app_env;
use crate::commands::build;
use crate::commands::create;
use crate::commands::delete;
use crate::commands::logs;
use crate::commands::remote;
use crate::commands::server::AppState;
use crate::commands::{start, stop};
use crate::db;
use crate::db::system_config as db_system_config;
use crate::github;
use crate::litestream;
use crate::models::{GitHubConnection, S3Config, S3ConfigRedacted, SystemConfig};
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, post},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
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
        .route("/docker/version", get(get_docker_version))
        .route("/apps/:name/remote", post(add_remote))
        .route("/apps/:name/remote", delete(remove_remote))
        .route("/apps/:name/build", post(build_app))
        .route("/apps/:name/builds", get(list_builds))
        .route("/apps/:name/builds/:build_id/logs", get(get_build_logs))
        .route("/config/s3", post(set_s3_config))
        .route("/config/s3", get(get_s3_config))
        .route("/config/s3", delete(delete_s3_config))
        // GitHub OAuth routes
        .route("/github/connect/start", post(github_connect_start))
        .route("/github/connect/poll", post(github_connect_poll))
        .route("/github/connect/stream", get(github_connect_stream))
        .route("/github/connection", delete(github_disconnect))
        .route("/github/status", get(github_status))
        .route("/github/repos", get(github_list_repos))
        .route("/github/repos/search", get(github_search_repos))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::auth_middleware,
        ))
        .with_state(state.clone());

    // Combine routes
    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB limit
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
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list apps: {}", e),
        )
            .into_response(),
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
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get app: {}", e),
        )
            .into_response(),
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
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to start app: {}", e),
        )
            .into_response(),
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
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to stop app: {}", e),
        )
            .into_response(),
    }
}

#[instrument(skip(name, _state))]
async fn get_logs(
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(_state): State<Arc<RwLock<AppState>>>,
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
        // Stream logs using podman-api
        match logs::execute(&name, lines, true).await {
            Ok(stream) => {
                let sse_stream = stream.map(|result| match result {
                    Ok(data) => Ok(Event::default().data(data)),
                    Err(e) => Err(e),
                });
                Sse::new(sse_stream).into_response()
            }
            Err(e) => (StatusCode::NOT_FOUND, format!("Failed to get logs: {}", e)).into_response(),
        }
    } else {
        // Get logs as a single response using podman-api
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
    let _pool = state.read().await.db_pool.clone();
    // Get the binary file from the multipart form
    let mut binary_data: Option<Bytes> = None;

    // Process all fields in the multipart form
    while let Ok(Some(field)) = multipart.next_field().await {
        tracing::info!("Processing field: {:?}", field.name());
        if field.name() == Some("binary") {
            match field.bytes().await {
                Ok(bytes) => {
                    tracing::info!("Successfully read binary data");
                    binary_data = Some(bytes);
                    break;
                }
                Err(e) => {
                    tracing::error!("Error reading binary field: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Error reading binary file",
                    )
                        .into_response();
                }
            }
        }
    }

    let _binary_data = match binary_data {
        Some(data) => data,
        None => {
            return (StatusCode::BAD_REQUEST, "No binary file provided").into_response();
        }
    };

    tracing::info!("Passing binary data to deploy command");

    // TODO: Implement deploy functionality
    // For now, just return success
    (
        StatusCode::OK,
        format!("App '{}' deployment received (not yet implemented)", name),
    )
        .into_response()
}

#[instrument(skip(state))]
async fn delete_app(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match delete::execute(&pool, &name).await {
        Ok(_) => (
            axum::http::StatusCode::OK,
            format!("App '{}' deleted", name),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete app: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct CreateAppRequest {
    name: String,
    from_github: Option<String>,
}

#[instrument(skip(state))]
async fn create_app(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Json(payload): Json<CreateAppRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Create the app
    if let Err(e) = create::execute(&pool, &payload.name).await {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create app: {}", e),
        )
            .into_response();
    }

    // If from_github is provided, add it as a remote
    if let Some(repo) = payload.from_github {
        // Get the GitHub connection for the user
        let connection = match db::github_connection::get_by_user_id(&pool, &auth_user.user_id).await {
            Ok(Some(conn)) => conn,
            Ok(None) => {
                return (
                    StatusCode::PRECONDITION_REQUIRED,
                    "GitHub account not connected. Connect GitHub first.",
                )
                    .into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to get GitHub connection: {}", e),
                )
                    .into_response();
            }
        };

        // Verify access to the repository
        let gh_client = github::GitHubClient::new(&connection.access_token);
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() != 2 {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid repository format. Use owner/repo",
            )
                .into_response();
        }

        let repo_info = match gh_client.get_repo(parts[0], parts[1]).await {
            Ok(info) => info,
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    format!("Repository not found or not accessible: {}", e),
                )
                    .into_response();
            }
        };

        // Add the remote (pass GitHub token for authentication)
        if let Err(e) = remote::add::execute(&pool, &payload.name, &repo_info.clone_url, Some(&connection.access_token)).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("App created but failed to add remote: {}", e),
            )
                .into_response();
        }
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
        Ok(None) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "App created but could not be retrieved",
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("App created but failed to retrieve: {}", e),
        )
            .into_response(),
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

#[derive(Debug, Deserialize)]
struct SetEnvRequest {
    key: String,
    value: String,
    delete: Option<bool>,
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
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to set environment variable: {}", e),
        )
            .into_response(),
    }
}

async fn get_docker_version() -> impl IntoResponse {
    // TODO: Implement proper Docker version check
    "docker"
}

#[derive(Debug, Deserialize)]
struct SetRemoteRequest {
    remote: String,
}

async fn add_remote(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Path(name): Path<String>,
    Json(payload): Json<SetRemoteRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Get GitHub token for the user (if connected)
    let github_token = match db::github_connection::get_by_user_id(&pool, &auth_user.user_id).await {
        Ok(Some(conn)) => Some(conn.access_token),
        _ => None,
    };

    match remote::add::execute(&pool, &name, &payload.remote, github_token.as_deref()).await {
        Ok(_) => (
            StatusCode::OK,
            format!("Remote configured for app '{}'", name),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to configure remote: {}", e),
        )
            .into_response(),
    }
}

async fn remove_remote(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match remote::remove::execute(&pool, &name).await {
        Ok(_) => (StatusCode::OK, format!("Remote removed for app '{}'", name)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to remove remote: {}", e),
        )
            .into_response(),
    }
}

async fn build_app(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Get GitHub token for the user (if connected)
    let github_token = match db::github_connection::get_by_user_id(&pool, &auth_user.user_id).await {
        Ok(Some(conn)) => Some(conn.access_token),
        _ => None,
    };

    match build::execute(&pool, &name, github_token.as_deref()).await {
        Ok(build_record) => Json(serde_json::json!({
            "message": format!("App '{}' built", name),
            "build_id": build_record.id
        })).into_response(),
        Err(build::BuildError::AlreadyBuilding(_)) => (
            StatusCode::CONFLICT,
            format!("App '{}' is already building", name),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to build app: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
struct BuildInfo {
    id: String,
    app_id: String,
    image_tag: Option<String>,
    git_commit: Option<String>,
    status: String,
    created_at: String,
}

#[instrument(skip(state))]
async fn list_builds(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Get app to verify it exists and get ID
    let app = match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("App '{}' not found", name)).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get app: {}", e),
            )
                .into_response();
        }
    };

    match db::build::get_all_by_app(&pool, &app.id).await {
        Ok(builds) => {
            let build_infos: Vec<BuildInfo> = builds
                .into_iter()
                .map(|b| BuildInfo {
                    id: b.id,
                    app_id: b.app_id,
                    image_tag: b.image_tag,
                    git_commit: b.git_commit,
                    status: b.status.to_string(),
                    created_at: b.created_at.0.to_rfc3339(),
                })
                .collect();
            Json(build_infos).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list builds: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct BuildLogsParams {
    name: String,
    build_id: String,
}

#[derive(Debug, Deserialize)]
struct BuildLogsQuery {
    follow: Option<bool>,
}

#[instrument(skip(state))]
async fn get_build_logs(
    State(state): State<Arc<RwLock<AppState>>>,
    Path(params): Path<BuildLogsParams>,
    Query(query): Query<BuildLogsQuery>,
) -> impl IntoResponse {
    use tokio::fs::File;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let pool = state.read().await.db_pool.clone();
    let follow = query.follow.unwrap_or(false);

    // Get app to verify it exists and get ID
    let app = match db::app::get_by_name(&pool, &params.name).await {
        Ok(Some(app)) => app,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, format!("App '{}' not found", params.name)).into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get app: {}", e),
            )
                .into_response();
        }
    };

    // Get the build
    let build = match db::build::get_by_id(&pool, &params.build_id).await {
        Ok(Some(build)) => build,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("Build '{}' not found", params.build_id),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get build: {}", e),
            )
                .into_response();
        }
    };

    // Verify build belongs to the app
    if build.app_id != app.id {
        return (
            StatusCode::NOT_FOUND,
            format!("Build '{}' not found for app '{}'", params.build_id, params.name),
        )
            .into_response();
    }

    // Get log path
    let log_path = match build.log_path {
        Some(path) => path,
        None => {
            return (StatusCode::NOT_FOUND, "Build logs not available").into_response();
        }
    };

    // Check if file exists
    if !std::path::Path::new(&log_path).exists() {
        return (StatusCode::NOT_FOUND, "Build log file not found").into_response();
    }

    if follow {
        // Stream logs using SSE
        let stream = async_stream::stream! {
            let file = match File::open(&log_path).await {
                Ok(f) => f,
                Err(e) => {
                    yield Ok::<_, std::convert::Infallible>(
                        Event::default().event("error").data(format!("Failed to open log file: {}", e))
                    );
                    return;
                }
            };

            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            // Read existing lines
            while let Ok(Some(line)) = lines.next_line().await {
                yield Ok::<_, std::convert::Infallible>(Event::default().data(line));
            }

            // Continue watching for new lines
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));
            let mut last_size = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);

            loop {
                interval.tick().await;

                // Check if file has grown
                let current_size = match std::fs::metadata(&log_path) {
                    Ok(m) => m.len(),
                    Err(_) => break,
                };

                if current_size > last_size {
                    // Re-open and seek to read new content
                    let file = match File::open(&log_path).await {
                        Ok(f) => f,
                        Err(_) => break,
                    };
                    let reader = BufReader::new(file);
                    let mut lines = reader.lines();

                    // Read new lines and send them
                    while let Ok(Some(line)) = lines.next_line().await {
                        yield Ok::<_, std::convert::Infallible>(Event::default().data(line));
                    }

                    last_size = current_size;
                }
            }
        };

        Sse::new(stream).into_response()
    } else {
        // Return all logs as plain text
        match tokio::fs::read_to_string(&log_path).await {
            Ok(content) => (StatusCode::OK, content).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read log file: {}", e),
            )
                .into_response(),
        }
    }
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
    let docker = state.read().await.docker.clone();

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
        Ok(_) => {
            // Sync Litestream configuration to apply new S3 settings
            match litestream::sync_configuration(&docker, &pool).await {
                Ok(_) => (
                    StatusCode::OK,
                    "S3 configuration saved and Litestream updated successfully",
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("S3 config saved but failed to update Litestream: {}", e),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save S3 config: {}", e),
        )
            .into_response(),
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get S3 config: {}", e),
        )
            .into_response(),
    }
}

#[instrument(skip(state))]
async fn delete_s3_config(State(state): State<Arc<RwLock<AppState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    let docker = state.read().await.docker.clone();

    match db_system_config::delete_s3_config(&pool).await {
        Ok(_) => {
            // Sync Litestream configuration to remove S3 settings
            match litestream::sync_configuration(&docker, &pool).await {
                Ok(_) => (
                    StatusCode::OK,
                    "S3 configuration deleted and Litestream updated successfully",
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("S3 config deleted but failed to update Litestream: {}", e),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete S3 config: {}", e),
        )
            .into_response(),
    }
}

// ===== AUTH ENDPOINTS =====

#[derive(Debug, serde::Serialize)]
struct AuthStatusResponse {
    initialized: bool,
    version: String,
}

#[instrument(skip(state))]
async fn auth_status_handler(
    State(state): State<Arc<RwLock<AppState>>>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match crate::db::user::count(&pool).await {
        Ok(count) => {
            let response = AuthStatusResponse {
                initialized: count > 0,
                version: env!("CARGO_PKG_VERSION").to_string(),
            };
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to check server status: {}", e),
        )
            .into_response(),
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

    match crate::commands::auth::login::execute(&pool, &payload.email, &payload.password, &jwt_secret).await {
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

    match crate::commands::auth::refresh::execute(&pool, &payload.refresh_token, &jwt_secret).await {
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
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to get user organizations: {}", e),
                )
                    .into_response(),
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, "User not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get user: {}", e),
        )
            .into_response(),
    }
}

// ===== GITHUB ENDPOINTS =====

#[derive(Debug, Serialize)]
struct DeviceFlowStartResponse {
    user_code: String,
    verification_uri: String,
    device_code: String,
    expires_in: u64,
    interval: u64,
}

#[instrument(skip(state))]
async fn github_connect_start(
    State(state): State<Arc<RwLock<AppState>>>,
) -> impl IntoResponse {
    let client_id = match state.read().await.github_client_id.clone() {
        Some(id) => id,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "GitHub OAuth is not configured (GITHUB_CLIENT_ID not set)",
            )
                .into_response();
        }
    };

    match github::start_device_flow(&client_id).await {
        Ok(response) => Json(DeviceFlowStartResponse {
            user_code: response.user_code,
            verification_uri: response.verification_uri,
            device_code: response.device_code,
            expires_in: response.expires_in,
            interval: response.interval,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to start GitHub device flow: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct DeviceFlowPollRequest {
    device_code: String,
    interval: u64,
    expires_in: u64,
}

#[derive(Debug, Serialize)]
struct GitHubConnectResponse {
    username: String,
    email: Option<String>,
}

#[instrument(skip(state))]
async fn github_connect_poll(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Json(payload): Json<DeviceFlowPollRequest>,
) -> impl IntoResponse {
    let (client_id, pool) = {
        let state = state.read().await;
        let client_id = match state.github_client_id.clone() {
            Some(id) => id,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "GitHub OAuth is not configured",
                )
                    .into_response();
            }
        };
        (client_id, state.db_pool.clone())
    };

    // Poll for token
    let (access_token, scopes) = match github::poll_for_token(
        &client_id,
        &payload.device_code,
        payload.interval,
        payload.expires_in,
    )
    .await
    {
        Ok(result) => result,
        Err(github::OAuthError::AuthorizationTimeout) => {
            return (StatusCode::REQUEST_TIMEOUT, "Authorization timed out").into_response();
        }
        Err(github::OAuthError::AccessDenied) => {
            return (StatusCode::FORBIDDEN, "Authorization was denied").into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to poll for token: {}", e),
            )
                .into_response();
        }
    };

    // Get GitHub user info
    let gh_client = github::GitHubClient::new(&access_token);
    let gh_user = match gh_client.get_user().await {
        Ok(user) => user,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get GitHub user: {}", e),
            )
                .into_response();
        }
    };

    // Save connection to database
    let connection = GitHubConnection::new(
        &auth_user.user_id,
        gh_user.id,
        &gh_user.login,
        gh_user.email.clone(),
        &access_token,
        &scopes,
    );

    if let Err(e) = db::github_connection::save(&pool, &connection).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save GitHub connection: {}", e),
        )
            .into_response();
    }

    Json(GitHubConnectResponse {
        username: gh_user.login,
        email: gh_user.email,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
struct DeviceFlowStreamQuery {
    device_code: String,
    interval: Option<u64>,
    expires_in: Option<u64>,
}

#[instrument(skip(state))]
async fn github_connect_stream(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Query(query): Query<DeviceFlowStreamQuery>,
) -> impl IntoResponse {
    let (client_id, pool) = {
        let state = state.read().await;
        let client_id = match state.github_client_id.clone() {
            Some(id) => id,
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "GitHub OAuth is not configured",
                )
                    .into_response();
            }
        };
        (client_id, state.db_pool.clone())
    };

    let device_code = query.device_code;
    let interval = query.interval.unwrap_or(5).max(5); // At least 5 seconds
    let expires_in = query.expires_in.unwrap_or(900); // Default 15 minutes
    let user_id = auth_user.user_id.clone();

    // Create an async stream that polls GitHub
    let stream = async_stream::stream! {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(expires_in);
        let poll_interval = std::time::Duration::from_secs(interval);

        loop {
            if start.elapsed() > timeout {
                yield Ok::<_, std::convert::Infallible>(Event::default().event("error").data("Authorization timed out"));
                break;
            }

            // Poll GitHub
            match github::poll_once(&client_id, &device_code).await {
                Ok(github::PollResult::Pending) => {
                    yield Ok::<_, std::convert::Infallible>(Event::default().event("pending").data("Waiting for authorization..."));
                }
                Ok(github::PollResult::Success { access_token, scope }) => {
                    // Get GitHub user info
                    let gh_client = github::GitHubClient::new(&access_token);
                    match gh_client.get_user().await {
                        Ok(gh_user) => {
                            // Save connection to database
                            let connection = GitHubConnection::new(
                                &user_id,
                                gh_user.id,
                                &gh_user.login,
                                gh_user.email.clone(),
                                &access_token,
                                &scope,
                            );

                            if let Err(e) = db::github_connection::save(&pool, &connection).await {
                                yield Ok::<_, std::convert::Infallible>(Event::default().event("error").data(format!("Failed to save connection: {}", e)));
                            } else {
                                // Send success with username as JSON
                                let response = serde_json::json!({
                                    "username": gh_user.login,
                                    "email": gh_user.email
                                });
                                yield Ok::<_, std::convert::Infallible>(Event::default().event("success").data(response.to_string()));
                            }
                        }
                        Err(e) => {
                            yield Ok::<_, std::convert::Infallible>(Event::default().event("error").data(format!("Failed to get GitHub user: {}", e)));
                        }
                    }
                    break;
                }
                Err(github::OAuthError::AuthorizationTimeout) => {
                    yield Ok::<_, std::convert::Infallible>(Event::default().event("error").data("Authorization timed out"));
                    break;
                }
                Err(github::OAuthError::AccessDenied) => {
                    yield Ok::<_, std::convert::Infallible>(Event::default().event("error").data("Authorization was denied"));
                    break;
                }
                Err(e) => {
                    yield Ok::<_, std::convert::Infallible>(Event::default().event("error").data(format!("Error: {}", e)));
                    break;
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    };

    Sse::new(stream).into_response()
}

#[instrument(skip(state))]
async fn github_disconnect(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db::github_connection::delete_by_user_id(&pool, &auth_user.user_id).await {
        Ok(_) => (StatusCode::OK, "GitHub connection removed").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to remove GitHub connection: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
struct GitHubStatusResponse {
    connected: bool,
    username: Option<String>,
    email: Option<String>,
    scopes: Option<String>,
}

#[instrument(skip(state))]
async fn github_status(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db::github_connection::get_by_user_id(&pool, &auth_user.user_id).await {
        Ok(Some(connection)) => Json(GitHubStatusResponse {
            connected: true,
            username: Some(connection.github_username),
            email: connection.github_email,
            scopes: Some(connection.scopes),
        })
        .into_response(),
        Ok(None) => Json(GitHubStatusResponse {
            connected: false,
            username: None,
            email: None,
            scopes: None,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get GitHub status: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct ListReposQuery {
    limit: Option<u32>,
}

#[instrument(skip(state))]
async fn github_list_repos(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Query(query): Query<ListReposQuery>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Get GitHub connection
    let connection = match db::github_connection::get_by_user_id(&pool, &auth_user.user_id).await {
        Ok(Some(conn)) => conn,
        Ok(None) => {
            return (
                StatusCode::PRECONDITION_REQUIRED,
                "GitHub account not connected. Run 'lh github connect' first.",
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get GitHub connection: {}", e),
            )
                .into_response();
        }
    };

    let gh_client = github::GitHubClient::new(&connection.access_token);
    let limit = query.limit.unwrap_or(30);

    match gh_client.list_repos(limit).await {
        Ok(repos) => Json(repos).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list repositories: {}", e),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct SearchReposQuery {
    q: String,
}

#[instrument(skip(state))]
async fn github_search_repos(
    State(state): State<Arc<RwLock<AppState>>>,
    axum::Extension(auth_user): axum::Extension<crate::auth::AuthUser>,
    Query(query): Query<SearchReposQuery>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    // Get GitHub connection
    let connection = match db::github_connection::get_by_user_id(&pool, &auth_user.user_id).await {
        Ok(Some(conn)) => conn,
        Ok(None) => {
            return (
                StatusCode::PRECONDITION_REQUIRED,
                "GitHub account not connected. Run 'lh github connect' first.",
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get GitHub connection: {}", e),
            )
                .into_response();
        }
    };

    let gh_client = github::GitHubClient::new(&connection.access_token);

    match gh_client.search_repos(&query.q).await {
        Ok(repos) => Json(repos).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to search repositories: {}", e),
        )
            .into_response(),
    }
}
