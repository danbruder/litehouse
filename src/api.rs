use crate::commands::app_command::app_env;
use crate::commands::app_command::create;
use crate::commands::app_command::delete;
use crate::commands::app_command::{start, stop};
use crate::commands::server_command::serve::ProxyState;
use crate::db;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::{
    extract::{Multipart, Path, Query, State},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::stream::unfold;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader, SeekFrom};
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
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB limit
        .with_state(state)
}

#[instrument(skip(state))]
async fn list_apps(State(state): State<Arc<RwLock<ProxyState>>>) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
    match db::apps::get_all(&pool).await {
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
    match db::apps::get_by_name(&pool, &name).await {
        Ok(Some(app)) => {
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

    match start::execute(&pool, &name).await {
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

    let log_path = match crate::config::get_app_log_path(&name) {
        Ok(path) => path,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("Log file not found: {}", e)).into_response();
        }
    };

    if !log_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            format!("Log file not found for app '{}'", name),
        )
            .into_response();
    }

    if follow {
        // Open the file and seek to the end
        let file = match File::open(&log_path).await {
            Ok(f) => f,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to open log file: {}", e),
                )
                    .into_response();
            }
        };
        let mut reader = BufReader::new(file);
        let _ = reader.seek(SeekFrom::End(0)).await;

        // Stream new lines as they are appended
        let stream = unfold(reader, |mut reader| async {
            let mut line = String::new();
            loop {
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                    Ok(_) => {
                        let out = line.clone();
                        line.clear();
                        return Some((
                            Ok::<_, std::convert::Infallible>(Event::default().data(out)),
                            reader,
                        ));
                    }
                    Err(_) => return None,
                }
            }
        });
        Sse::new(stream).into_response()
    } else {
        // Read last N lines from the log file
        match tokio::fs::read(&log_path).await {
            Ok(data) => {
                let content = String::from_utf8_lossy(&data);
                let lines_vec: Vec<&str> = content.lines().collect();
                let start = if lines_vec.len() > lines {
                    lines_vec.len() - lines
                } else {
                    0
                };
                let last_lines = lines_vec[start..].join("\n");
                (StatusCode::OK, last_lines).into_response()
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read log file: {}", e),
            )
                .into_response(),
        }
    }
}

#[instrument(skip(state, multipart))]
async fn deploy_app(
    State(state): State<Arc<RwLock<ProxyState>>>,
    Path(name): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let pool = state.read().await.db_pool.clone();
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

    let binary_data = match binary_data {
        Some(data) => data,
        None => {
            return (StatusCode::BAD_REQUEST, "No binary file provided").into_response();
        }
    };

    tracing::info!("Passing binary data to deploy command");

    // Pass the binary data to the deploy command
    match deploy::execute(&pool, &name, &binary_data).await {
        Ok(_) => (
            StatusCode::OK,
            format!("App '{}' deployed successfully", name),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to deploy app: {}", e),
        )
            .into_response(),
    }
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
    State(_state): State<Arc<RwLock<ProxyState>>>,
    Path(name): Path<String>,
    Json(payload): Json<SetEnvRequest>,
) -> impl IntoResponse {
    match app_env::set_env(
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
