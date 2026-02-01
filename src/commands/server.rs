use crate::docker;
use crate::message_bus::{Message, MessageBus};
use crate::reconciler::Reconciler;
use anyhow::{Context, Result};
use axum::Router;
use bollard::Docker;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;
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
    pub message_bus: Arc<MessageBus>,
    /// Track active log streaming tasks per app name
    pub log_streaming_tasks: Arc<RwLock<HashMap<String, (JoinHandle<()>, oneshot::Sender<()>)>>>,
    /// Webhook base URL (e.g., "https://admin.yourdomain.com")
    pub webhook_url: Option<String>,
}

/// Start the Litehouse server
#[instrument]
pub async fn execute(config: ServerConfig) -> Result<()> {
    // Attempt to restore all databases from S3 if needed
    // This must happen before db::init_pool() so the database is available
    if let Err(e) = crate::litestream::restore_all_databases_if_needed().await {
        tracing::warn!("Database restore check failed: {}", e);
        // Continue anyway - system may start with fresh database
    }

    // Connect to database
    let pool = db::init_pool().await?;
    let docker_conn = docker::connect().await?;

    // Create reconciler and run initial reconciliation
    let reconciler = Reconciler::new(pool.clone(), docker_conn.clone());
    let report = reconciler.reconcile_all(&config).await;
    Reconciler::log_report(&report);

    // Spawn background reconciliation loop if interval > 0
    let reconcile_interval = config.reconcile_interval_secs;
    if reconcile_interval > 0 {
        let reconciler_clone = reconciler.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(reconcile_interval);
            loop {
                tokio::time::sleep(interval).await;
                let report = reconciler_clone.reconcile_all(&config_clone).await;
                // Only log if something changed (fixes or failures)
                if report.has_fixes_or_failures() {
                    Reconciler::log_report(&report);
                }
            }
        });
        info!(
            "Background reconciliation enabled (interval: {}s)",
            reconcile_interval
        );
    } else {
        info!("Background reconciliation disabled");
    }

    // Get JWT secret from environment or use default (warning will be logged)
    let jwt_secret = crate::auth::jwt::get_jwt_secret();

    // GitHub OAuth client ID - default to litehouse's app, allow override
    const DEFAULT_GITHUB_CLIENT_ID: &str = "Ov23liTp4hQb5j4lzQfh";
    let github_client_id = Some(
        std::env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| DEFAULT_GITHUB_CLIENT_ID.to_string()),
    );

    // Initialize message bus for real-time messaging
    let message_bus = Arc::new(MessageBus::new());

    // Spawn background task for periodic heartbeat
    {
        let bus = message_bus.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                bus.publish(Message::Heartbeat);
            }
        });
    }

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
        message_bus,
        log_streaming_tasks: Arc::new(RwLock::new(HashMap::new())),
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
