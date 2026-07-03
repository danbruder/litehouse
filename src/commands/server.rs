use crate::caddy;
use crate::docker;
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
    pub github_client_id: Option<String>,
    /// Webhook base URL (e.g., "https://admin.yourdomain.com")
    pub webhook_url: Option<String>,
}

/// Ensure Caddy is running and its configuration matches the database.
///
/// Container liveness for app containers is handled by Docker's own restart
/// policy (`--restart unless-stopped`) — this only takes care of the reverse
/// proxy, which the server itself is responsible for managing.
#[instrument(skip(docker, pool, config))]
pub async fn sync_on_boot(
    docker: &Docker,
    pool: &sqlx::Pool<sqlx::Sqlite>,
    config: &ServerConfig,
) -> Result<()> {
    caddy::start(docker, config)
        .await
        .context("Failed to start Caddy container")?;
    caddy::sync_configuration(docker, pool)
        .await
        .context("Failed to sync Caddy configuration")?;
    info!("Caddy started and configuration synced");
    Ok(())
}

/// Start the Litehouse server
#[instrument]
pub async fn execute(config: ServerConfig) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;
    let docker_conn = docker::connect().await?;

    // Ensure Caddy is running and configured. A Caddy failure must not take the
    // admin API down with it — stay up so the operator can diagnose and fix.
    if let Err(e) = sync_on_boot(&docker_conn, &pool, &config).await {
        tracing::error!("caddy sync on boot failed (admin API starting anyway): {e:#}");
    }

    // Get JWT secret from environment or use default (warning will be logged)
    let jwt_secret = crate::auth::jwt::get_jwt_secret();

    // GitHub OAuth client ID - default to litehouse's app, allow override
    const DEFAULT_GITHUB_CLIENT_ID: &str = "Ov23liTp4hQb5j4lzQfh";
    let github_client_id = Some(
        std::env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| DEFAULT_GITHUB_CLIENT_ID.to_string()),
    );

    // Construct webhook URL from domain if available
    let webhook_url = config
        .domain
        .as_ref()
        .map(|d| format!("https://admin.{}", d));

    // Create shared state
    let state = Arc::new(RwLock::new(AppState {
        db_pool: pool.clone(),
        docker: docker_conn.clone(),
        jwt_secret,
        github_client_id,
        webhook_url,
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
        //.with_graceful_shutdown(shutdown_signal())
        .await
        .context("Server error")?;

    println!("Server stopped");

    Ok(())
}

#[allow(dead_code)]
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
    println!("\nShutting down...");
}
