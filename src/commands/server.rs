use crate::caddy;
use crate::litestream;
use crate::podman;
use anyhow::{Context, Result};
use axum::Router;
use bollard::Docker;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, instrument};

use crate::admin_spa;
use crate::api;
use crate::config::ServerConfig;
use crate::db;

/// Shared state for the server
pub struct AppState {
    pub db_pool: sqlx::Pool<sqlx::Sqlite>,
    pub docker: Docker,
    pub jwt_secret: String,
}

/// Start the Litehouse server
#[instrument]
pub async fn execute(config: ServerConfig) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;
    let docker = podman::connect().await?;

    // NOTE: Caddy and Litestream containers are started by the init script
    // with restart=unless-stopped policy. Here we only sync their configurations.

    // Sync Caddy configuration with existing apps
    if let Err(e) = caddy::sync_configuration(&docker, &pool).await {
        tracing::warn!("Failed to sync Caddy configuration on startup: {}", e);
        // Don't fail startup if Caddy sync fails
    }

    // Sync Litestream configuration with existing apps and S3 config
    if let Err(e) = litestream::sync_configuration(&docker, &pool).await {
        tracing::warn!("Failed to sync Litestream configuration on startup: {}", e);
        // Don't fail startup if Litestream sync fails
    }

    // Get JWT secret from environment or use default (warning will be logged)
    let jwt_secret = crate::auth::jwt::get_jwt_secret();

    // Create shared state
    let state = Arc::new(RwLock::new(AppState {
        db_pool: pool.clone(),
        docker: docker.clone(),
        jwt_secret,
    }));

    // Build combined router: API routes under /api, SPA fallback for everything else
    let app = Router::new()
        .nest("/api", api::create_api_router(state.clone()))
        .fallback_service(admin_spa::create_admin_router());

    // Parse host and port for server
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .context(format!(
            "Invalid host or port: {}:{}",
            config.host, config.port
        ))?;

    info!("Starting Litehouse server on http://{}", addr);
    println!("Litehouse server running at http://{}", addr);
    println!("  API:  http://{}/api", addr);
    println!("  SPA:  http://{}/", addr);
    println!("Press Ctrl+C to stop");

    // Create server with graceful shutdown
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Server error")?;

    println!("Server stopped");

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    println!("\nShutting down...");
}
