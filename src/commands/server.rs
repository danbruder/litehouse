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
use crate::backup;
use crate::config::ServerConfig;
use crate::db;

/// Shared state for the server
pub struct AppState {
    pub db_pool: sqlx::Pool<sqlx::Sqlite>,
    pub docker: Docker,
    /// sha256 hex hash of the single admin token
    pub admin_token_hash: String,
    pub server_config: ServerConfig,
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

    // Single-operator admin token: use the hash stored in server config, or
    // generate a fresh ephemeral token for this run if none is configured yet.
    let admin_token_hash = match &config.admin_token_hash {
        Some(hash) => hash.clone(),
        None => {
            let token = crate::auth::generate_token();
            let hash = crate::auth::hash_token(&token);
            println!("================================================================");
            println!("ADMIN TOKEN (save this): {}", token);
            println!(
                "This token is EPHEMERAL — it was generated because no admin_token_hash"
            );
            println!(
                "is set in server-config.toml. It will change on the next restart unless"
            );
            println!(
                "you persist it: add `admin_token_hash = \"{}\"` to server-config.toml.",
                hash
            );
            println!("================================================================");
            hash
        }
    };

    // Create shared state
    let state = Arc::new(RwLock::new(AppState {
        db_pool: pool.clone(),
        docker: docker_conn.clone(),
        admin_token_hash,
        server_config: config.clone(),
    }));

    // Daily backup: check hourly whether today's (UTC) backup has run; retry
    // next hour on failure. Fires immediately on the first tick (on process
    // boot) — that's intentional, it means a fresh boot with no backup yet
    // today catches up right away instead of waiting up to an hour.
    //
    // `run_backup` returns a calm, actionable error (`BackupError::S3ConfigMissing`)
    // when no S3 config is set yet, rather than panicking — this loop just
    // logs it and quietly retries next hour, so an un-configured server
    // doesn't spam anything worse than a once-an-hour log line.
    {
        let pool = pool.clone();
        let docker_conn = docker_conn.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                let last = db::system_config::get_last_backup_date(&pool).await.ok().flatten();
                if last.as_deref() != Some(today.as_str()) {
                    match backup::run_backup(&pool, &docker_conn).await {
                        Ok(report) => {
                            if let Err(e) = db::system_config::set_last_backup_date(&pool, &today).await {
                                tracing::error!("failed to record last_backup_date: {e:#}");
                            }
                            tracing::info!(?report, "daily backup complete");
                        }
                        Err(e) => tracing::error!("daily backup failed, will retry next hour: {e:#}"),
                    }
                }
            }
        });
    }

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
