use crate::caddy;
use crate::docker;
use anyhow::{Context, Result};
use axum::Router;
use bollard::Docker;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, RwLock};
use tracing::{info, instrument};

use crate::api;
use crate::backup;
use crate::metrics;
use crate::ui;
use crate::config::ServerConfig;
use crate::db;

/// Registry of per-app locks, keyed by app name.
pub type AppLocks = Arc<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>>;

/// Shared state for the server
pub struct AppState {
    pub db_pool: sqlx::Pool<sqlx::Sqlite>,
    pub docker: Docker,
    /// sha256 hex hash of the single admin token
    pub admin_token_hash: String,
    pub server_config: ServerConfig,
    /// Serializes container-lifecycle operations per app — see [`lock_app`].
    pub app_locks: AppLocks,
}

/// Acquire the lock for a single app's container-lifecycle operations
/// (start/stop/restart/redeploy/the GitHub deploy hook).
///
/// Without this, a manual action (e.g. clicking "restart" in the admin UI)
/// and an automated deploy (the GitHub Actions hook firing off a `git push`)
/// can run fully concurrently against the same app. Both end up calling
/// `volume::init_app_volume`, which creates an ephemeral helper container
/// with a name derived only from the app id (`litehouse-init-{app_id}`) —
/// whichever one's `create_container` call lands first wins, and the other
/// gets a Docker 409 "name already in use" and its whole deploy/restart
/// fails. Holding this lock for the duration of any such operation makes
/// them run one at a time per app instead of racing.
pub async fn lock_app(locks: &AppLocks, name: &str) -> OwnedMutexGuard<()> {
    let entry = locks
        .lock()
        .unwrap()
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone();
    entry.lock_owned().await
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
        app_locks: Arc::new(StdMutex::new(HashMap::new())),
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
                        // Only mark the day done when *everything* succeeded:
                        // run_backup returns Ok even when individual apps (or
                        // the state DB) failed, and a partial backup should be
                        // retried next hour, not counted as today's backup.
                        Ok(report) if report.failed.is_empty() => {
                            if let Err(e) = db::system_config::set_last_backup_date(&pool, &today).await {
                                tracing::error!("failed to record last_backup_date: {e:#}");
                            }
                            tracing::info!(?report, "daily backup complete");
                        }
                        Ok(report) => tracing::error!(
                            ?report,
                            "daily backup completed with failures, will retry next hour"
                        ),
                        Err(e) => tracing::error!("daily backup failed, will retry next hour: {e:#}"),
                    }
                }
            }
        });
    }

    // Resource-usage sampler: every 60s, snapshot host + per-running-app
    // CPU/mem/disk into `metric_sample`; once an hour, roll the completed
    // hour up into `metric_hourly` and prune old rows. See src/metrics.rs.
    {
        let pool = pool.clone();
        let docker_conn = docker_conn.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            let mut prev_host_cpu = None;
            let mut prev_app_disk = std::collections::HashMap::new();
            let mut tick_count: u64 = 0;
            loop {
                interval.tick().await;
                metrics::run_tick(&pool, &docker_conn, &mut prev_host_cpu, tick_count, &mut prev_app_disk).await;
                if tick_count > 0 && tick_count % 60 == 0 {
                    metrics::rollup_and_prune(&pool).await;
                }
                tick_count += 1;
            }
        });
    }

    // Build combined router: API routes under /api, admin UI for everything else
    let app = Router::new()
        .nest("/api", api::create_api_router(state.clone()))
        .fallback_service(ui::create_ui_router(state.clone()));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lock_app_serializes_operations_on_the_same_app() {
        let locks: AppLocks = Arc::new(StdMutex::new(HashMap::new()));
        let events = Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let mut handles = Vec::new();
        for i in 0..5 {
            let locks = locks.clone();
            let events = events.clone();
            handles.push(tokio::spawn(async move {
                let _guard = lock_app(&locks, "same-app").await;
                events.lock().await.push((i, "enter"));
                // Yield so a racing task would interleave here if the lock
                // weren't actually held for the critical section.
                tokio::task::yield_now().await;
                events.lock().await.push((i, "exit"));
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let events = events.lock().await;
        // Every enter must be immediately followed by that same task's exit —
        // no two tasks' critical sections interleaved.
        for pair in events.chunks(2) {
            assert_eq!(pair[0].0, pair[1].0, "events interleaved: {:?}", *events);
            assert_eq!(pair[0].1, "enter");
            assert_eq!(pair[1].1, "exit");
        }
    }

    #[tokio::test]
    async fn lock_app_does_not_block_different_apps() {
        let locks: AppLocks = Arc::new(StdMutex::new(HashMap::new()));

        // Hold app "a"'s lock, then confirm app "b"'s lock is acquirable
        // without waiting on "a".
        let guard_a = lock_app(&locks, "a").await;
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            lock_app(&locks, "b"),
        )
        .await;
        assert!(result.is_ok(), "locking a different app should not block");
        drop(guard_a);
    }
}
