use crate::commands::app_env;
use crate::commands::build;
use crate::commands::create;
use crate::commands::delete;
use crate::commands::logs;
use crate::commands::remote;
use crate::commands::server::ProxyState;
use crate::commands::{start, stop};
use crate::db;
use crate::db::system_config as db_system_config;
use crate::litestream;
use crate::models::{S3Config, S3ConfigRedacted, SystemConfig};
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
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::instrument;

pub fn create_api_router(state: Arc<RwLock<ProxyState>>) -> Router {
    Router::new()
        .route("/apps", get(list_apps))
        .route("/apps", post(create_app))
        .route("/apps/:name", get(get_app))
        .route("/apps/:name", delete(delete_app))
        .route("/apps/:name/start", post(start_app))
        .route("/apps/:name/stop", post(stop_app))
        .route("/apps/:name/logs", get(get_logs))
        .route("/apps/:name/deploy", post(deploy_app))
        .route("/apps/:name/env", post(set_env))
        .route("/podman/version", get(get_podman_version))
        .route("/apps/:name/remote", post(add_remote))
        .route("/apps/:name/remote", delete(remove_remote))
        .route("/apps/:name/build", post(build_app))
        .route("/config/s3", post(set_s3_config))
        .route("/config/s3", get(get_s3_config))
        .route("/config/s3", delete(delete_s3_config))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB limit
        .with_state(state)
}

#[instrument(skip(state))]
async fn list_apps(State(state): State<Arc<RwLock<ProxyState>>>) -> impl IntoResponse {
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
    State(state): State<Arc<RwLock<ProxyState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();

    match db::app::get_by_name(&pool, &name).await {
        Ok(Some(app)) => {
            let input = crate::models::BuildInput {
                app_id: app.id.to_string(),
                image_id: "hello".into(),
                image_tag: "nginx:latest".into(),
                git_commit: "heyo".into(),
            };

            let build = crate::models::Build::new(input);
            db::build::save(&pool, &build).await.unwrap();

            let app_info = AppInfo {
                id: app.id.to_string(),
                name: app.name,
                state: app.state.to_string(),
            };

            Json(app_info).into_response()
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
    State(state): State<Arc<RwLock<ProxyState>>>,
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
    State(_state): State<Arc<RwLock<ProxyState>>>,
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
    State(_state): State<Arc<RwLock<ProxyState>>>,
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
    State(state): State<Arc<RwLock<ProxyState>>>,
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
    State(state): State<Arc<RwLock<ProxyState>>>,
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
}

#[instrument(skip(state))]
async fn create_app(
    State(state): State<Arc<RwLock<ProxyState>>>,
    Json(payload): Json<CreateAppRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match create::execute(&pool, &payload.name).await {
        Ok(_) => (
            axum::http::StatusCode::CREATED,
            format!("App '{}' created", payload.name),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create app: {}", e),
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

#[derive(Debug, Deserialize)]
struct SetEnvRequest {
    key: String,
    value: String,
    delete: Option<bool>,
}

async fn set_env(
    State(state): State<Arc<RwLock<ProxyState>>>,
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

async fn get_podman_version() -> impl IntoResponse {
    // match crate::providers::podman::get_podman_version().await {
    //     Ok(version) => (StatusCode::OK, version).into_response(),
    //     Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    // }
    "lol"
}

#[derive(Debug, Deserialize)]
struct SetRemoteRequest {
    remote: String,
}

async fn add_remote(
    State(state): State<Arc<RwLock<ProxyState>>>,
    Path(name): Path<String>,
    Json(payload): Json<SetRemoteRequest>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match remote::add::execute(&pool, &name, &payload.remote).await {
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
    State(state): State<Arc<RwLock<ProxyState>>>,
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
    State(state): State<Arc<RwLock<ProxyState>>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match build::execute(&pool, &name).await {
        Ok(_) => (StatusCode::OK, format!("App '{}' built", name)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to build app: {}", e),
        )
            .into_response(),
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
    State(state): State<Arc<RwLock<ProxyState>>>,
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
async fn get_s3_config(State(state): State<Arc<RwLock<ProxyState>>>) -> impl IntoResponse {
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
async fn delete_s3_config(State(state): State<Arc<RwLock<ProxyState>>>) -> impl IntoResponse {
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
