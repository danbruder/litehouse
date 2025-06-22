use anyhow::{anyhow, Result};
use chrono::Utc;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time;
use tracing::{error, info, instrument, warn};

use crate::commands;
use crate::db;
use crate::models::{App, AppState, HealthCheckType};

use once_cell::sync::OnceCell;

// Global supervisor instance
pub static SUPERVISOR: OnceCell<Supervisor> = OnceCell::new();

/// Initialize the process supervisor
#[instrument]
pub async fn init(pool: Pool<Sqlite>) -> Result<()> {
    info!("Initializing process supervisor...");
    // Create supervisor
    let supervisor = Supervisor::new(pool).await?;

    // Store in global state
    if SUPERVISOR.set(supervisor).is_err() {
        return Err(anyhow::anyhow!("Supervisor already initialized"));
    }

    info!("Process supervisor initialized");

    Ok(())
}

// Message types for the supervisor channel
#[derive(Debug)]
pub enum SupervisorMessage {
    Start(String),            // App name
    Stop(String),             // App name
    Restart(String),          // App name
    CheckHealth(String),      // App name
    ProcessExit(String, i64), // App name, exit code
}

pub struct Supervisor {
    tx: mpsc::Sender<SupervisorMessage>,
    db_pool: Pool<Sqlite>,
    running_processes: Arc<Mutex<HashMap<String, RunningProcess>>>,
}

struct RunningProcess {
    child: Child,
    started_at: Instant,
}

impl Supervisor {
    #[instrument]
    pub async fn new(db_pool: Pool<Sqlite>) -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<SupervisorMessage>(100);

        // Create shared state
        let running_processes = Arc::new(Mutex::new(HashMap::new()));
        let processes_clone = running_processes.clone();
        let pool_clone = db_pool.clone();

        // Spawn the supervisor task
        tokio::spawn(async move {
            info!("Process supervisor started");

            // Restore running apps from database
            match Self::restore_running_apps(&pool_clone, &processes_clone).await {
                Ok(count) => info!("Restored {} running apps", count),
                Err(e) => error!("Failed to restore running apps: {}", e),
            }

            // Start health check timer
            let health_check_interval = time::interval(Duration::from_secs(10));
            tokio::pin!(health_check_interval);

            // Process messages
            loop {
                tokio::select! {
                    Some(msg) = rx.recv() => {
                        match msg {
                            SupervisorMessage::Start(app_name) => {
                                if let Err(e) = Self::handle_start(&pool_clone, &processes_clone, &app_name).await {
                                    error!("Failed to start app '{}': {}", app_name, e);
                                }
                            },
                            SupervisorMessage::Stop(app_name) => {
                                if let Err(e) = Self::handle_stop(&pool_clone, &processes_clone, &app_name).await {
                                    error!("Failed to stop app '{}': {}", app_name, e);
                                }
                            },
                            SupervisorMessage::Restart(app_name) => {
                                if let Err(e) = Self::handle_restart(&pool_clone, &processes_clone, &app_name).await {
                                    error!("Failed to restart app '{}': {}", app_name, e);
                                }
                            },
                            SupervisorMessage::CheckHealth(app_name) => {
                                if let Err(e) = Self::handle_health_check(&pool_clone, &processes_clone, &app_name).await {
                                    warn!("Health check failed for app '{}': {}", app_name, e);
                                }
                            },
                            SupervisorMessage::ProcessExit(app_name, exit_code) => {
                                if let Err(e) = Self::handle_process_exit(&pool_clone, &processes_clone, &app_name, exit_code).await {
                                    error!("Failed to handle process exit for app '{}': {}", app_name, e);
                                }
                            }
                        }
                    },
                    _ = health_check_interval.tick() => {
                        Self::run_health_checks(&pool_clone, &processes_clone).await;
                    }
                }
            }
        });

        info!("Supervisor task spawned");

        Ok(Self {
            tx,
            db_pool,
            running_processes,
        })
    }

    #[instrument(skip(db_pool, processes))]
    async fn restore_running_apps(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
    ) -> Result<usize> {
        let running_apps = db::apps::get_by_state(db_pool, AppState::Running).await?;

        let mut count = 0;
        for app in running_apps {
            match Self::start_process(db_pool, processes, &app).await {
                Ok(_) => {
                    info!("Restored app '{}' (PID: {:?})", app.name, app.process_id);
                    count += 1;
                }
                Err(e) => {
                    error!("Failed to restore app '{}': {}", app.name, e);
                    // Update app state to failed
                    let mut app = app.clone();
                    app.state = AppState::Failed;
                    app.updated_at = Utc::now();
                    if let Err(e) = db::apps::save(db_pool, &app).await {
                        error!("Failed to update app state: {}", e);
                    }
                }
            }
        }

        Ok(count)
    }

    #[instrument(skip(db_pool, processes))]
    async fn handle_start(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
        app_name: &str,
    ) -> Result<()> {
        // Get app from database
        let app = db::apps::get_by_name(db_pool, app_name)
            .await?
            .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

        // Check if app is already running
        {
            let process_map = processes.lock().unwrap();
            if process_map.contains_key(&app.name) {
                return Err(anyhow!("App '{}' is already running", app_name));
            }
        }

        // Start the process
        Self::start_process(db_pool, processes, &app).await
    }

    #[instrument(skip(db_pool, processes))]
    async fn start_process(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
        app: &App,
    ) -> Result<()> {
        use crate::providers::cmd::CmdProvider;

        match commands::start::execute(db_pool, &app.name, CmdProvider {}).await {
            Ok(handle) => {
                let mut process_map = processes.lock().unwrap();
                process_map.insert(
                    app.name.clone(),
                    RunningProcess {
                        child: handle,
                        started_at: Instant::now(),
                    },
                );
                Ok(())
            }
            Err(e) => {
                error!("Failed to start process '{}': {}", app.name, e);
                Err(anyhow!("Failed to start process: {}", e))
            }
        }
    }

    #[instrument(skip(db_pool, processes))]
    async fn handle_stop(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
        app_name: &str,
    ) -> Result<()> {
        // Get app
        let app = db::apps::get_by_name(db_pool, app_name)
            .await?
            .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

        // Get child process
        let child_opt = {
            let mut process_map = processes.lock().unwrap();
            process_map.remove(app_name)
        };

        // If we found a child process, try to stop it gracefully
        if let Some(mut running) = child_opt {
            info!("Stopping app '{}' (PID: {})", app_name, running.child.id());

            // Update app state
            let mut app = app.clone();
            app.state = AppState::Stopping;
            app.updated_at = Utc::now();
            db::apps::save(db_pool, &app).await?;

            // Try to terminate gracefully first
            match running.child.kill() {
                Ok(_) => {
                    info!(
                        "Sent kill signal to app '{}' (PID: {})",
                        app_name,
                        running.child.id()
                    );

                    // Try to wait for process to exit
                    match running.child.wait() {
                        Ok(status) => {
                            info!("App '{}' exited with status: {:?}", app_name, status);

                            // Update process history
                            let exit_code = status.code().map(|c| c as i64);
                            Self::update_process_history(
                                db_pool,
                                &app,
                                exit_code,
                                "Stopped by user",
                            )
                            .await?;

                            // Update app state
                            app.state = AppState::Stopped;
                            app.process_id = None;
                            app.last_exit_code = exit_code;
                            app.last_exit_time = Some(Utc::now());
                            app.updated_at = Utc::now();
                            db::apps::save(db_pool, &app).await?;
                        }
                        Err(e) => {
                            error!("Failed to wait for app '{}' to exit: {}", app_name, e);
                            return Err(anyhow!("Failed to wait for app to exit: {}", e));
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to kill app '{}': {}", app_name, e);
                    return Err(anyhow!("Failed to kill app: {}", e));
                }
            }
        } else {
            // App was not in the running processes map, but marked as running in DB
            warn!(
                "App '{}' was marked as running but not found in process map",
                app_name
            );

            // Update app state
            let mut app = app.clone();
            app.state = AppState::Stopped;
            app.process_id = None;
            app.updated_at = Utc::now();
            db::apps::save(db_pool, &app).await?;
        }

        info!("Successfully stopped app '{}'", app_name);

        Ok(())
    }

    #[instrument(skip(db_pool, processes))]
    async fn handle_restart(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
        app_name: &str,
    ) -> Result<()> {
        // Stop app if running
        let app = db::apps::get_by_name(db_pool, app_name)
            .await?
            .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

        if app.state == AppState::Running {
            Self::handle_stop(db_pool, processes, app_name).await?;
        }

        // Wait a moment before starting
        time::sleep(Duration::from_secs(1)).await;

        // Start app
        Self::handle_start(db_pool, processes, app_name).await
    }

    #[instrument(skip(db_pool, processes))]
    async fn handle_process_exit(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
        app_name: &str,
        exit_code: i64,
    ) -> Result<()> {
        // Get app
        let app = db::apps::get_by_name(db_pool, app_name)
            .await?
            .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

        // Remove from running processes
        {
            let mut process_map = processes.lock().unwrap();
            process_map.remove(app_name);
        }

        // Update app in database
        let mut app = app.clone();
        app.process_id = None;
        app.last_exit_code = Some(exit_code);
        app.last_exit_time = Some(Utc::now());
        app.updated_at = Utc::now();

        let exit_reason = if exit_code == 0 {
            "Clean exit"
        } else {
            "Crashed"
        };

        // Update process history
        Self::update_process_history(db_pool, &app, Some(exit_code), exit_reason).await?;

        // Handle restart logic
        if app.should_restart() {
            if app.reached_max_restarts() {
                error!(
                    "App '{}' reached maximum restart count ({})",
                    app_name, app.restart_count
                );
                app.state = AppState::Crashed;
                db::apps::save(db_pool, &app).await?;
                return Err(anyhow!("App reached maximum restart count"));
            }

            info!(
                "Restarting app '{}' after exit (code: {})",
                app_name, exit_code
            );
            app.state = AppState::Restarting;
            app.restart_count += 1;
            db::apps::save(db_pool, &app).await?;

            // Wait before restart
            let backoff = std::cmp::min(app.restart_count, 5) as u64;
            time::sleep(Duration::from_secs(backoff)).await;

            // Start the process again
            return Self::start_process(db_pool, processes, &app).await;
        } else {
            // App should not be restarted
            if exit_code == 0 {
                app.state = AppState::Stopped;
            } else {
                app.state = AppState::Failed;
            }
            db::apps::save(db_pool, &app).await?;
        }

        Ok(())
    }

    async fn update_process_history(
        db_pool: &Pool<Sqlite>,
        app: &App,
        exit_code: Option<i64>,
        exit_reason: &str,
    ) -> Result<()> {
        // Find the latest history entry for this app
        let entries = db::process_history::get_by_app_id(db_pool, &app.id).await?;

        if let Some(mut latest) = entries.into_iter().next() {
            // Update the entry
            latest.ended_at = Some(Utc::now());
            latest.exit_code = exit_code;
            latest.exit_reason = Some(exit_reason.to_string());
            db::process_history::save(db_pool, &latest).await?;
        }

        Ok(())
    }

    #[instrument(skip(db_pool, processes))]
    async fn handle_health_check(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
        app_name: &str,
    ) -> Result<()> {
        // Get app
        let app = db::apps::get_by_name(db_pool, app_name)
            .await?
            .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

        // Skip if app is not running
        if app.state != AppState::Running {
            return Ok(());
        }

        // Skip if no health check is configured
        let health_check = match &app.health_check {
            Some(hc) => hc,
            None => return Ok(()),
        };

        // Check if process exists
        {
            let process_map = processes.lock().unwrap();
            if !process_map.contains_key(app_name) {
                return Err(anyhow!("App '{}' not found in process map", app_name));
            }
        }

        // Perform health check
        match &health_check.check_type {
            HealthCheckType::HttpGet {
                path,
                expected_status,
            } => {
                let url = format!("{}{}", app.url(), path);

                // Create a client with timeout
                let client = reqwest::ClientBuilder::new()
                    .timeout(Duration::from_secs(health_check.timeout as u64))
                    .build()?;

                // Make the request
                match client.get(&url).send().await {
                    Ok(response) => {
                        if response.status().as_u16() == *expected_status {
                            info!("Health check passed for app '{}'", app_name);
                            return Ok(());
                        } else {
                            return Err(anyhow!(
                                "Health check failed for app '{}': expected status {}, got {}",
                                app_name,
                                expected_status,
                                response.status()
                            ));
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!("Health check failed for app '{}': {}", app_name, e));
                    }
                }
            }
        }
    }

    async fn run_health_checks(
        db_pool: &Pool<Sqlite>,
        processes: &Arc<Mutex<HashMap<String, RunningProcess>>>,
    ) {
        // Get all running apps
        match db::apps::get_by_state(db_pool, AppState::Running).await {
            Ok(apps) => {
                for app in apps {
                    // Skip apps without health checks
                    if app.health_check.is_none() {
                        continue;
                    }

                    // Run health check
                    match Self::handle_health_check(db_pool, processes, &app.name).await {
                        Ok(_) => {
                            // Health check passed
                        }
                        Err(e) => {
                            error!("Health check failed for app '{}': {}", app.name, e);

                            // Try to restart the app
                            match Self::handle_restart(db_pool, processes, &app.name).await {
                                Ok(_) => {
                                    info!("Restarted app '{}' after failed health check", app.name)
                                }
                                Err(e) => error!("Failed to restart app '{}': {}", app.name, e),
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to get running apps: {}", e);
            }
        }
    }

    // External API

    pub async fn start_app(&self, app_name: &str) -> Result<()> {
        self.tx
            .send(SupervisorMessage::Start(app_name.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send start message: {}", e))
    }

    pub async fn stop_app(&self, app_name: &str) -> Result<()> {
        self.tx
            .send(SupervisorMessage::Stop(app_name.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send stop message: {}", e))
    }

    pub async fn restart_app(&self, app_name: &str) -> Result<()> {
        self.tx
            .send(SupervisorMessage::Restart(app_name.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send restart message: {}", e))
    }

    pub async fn check_app_health(&self, app_name: &str) -> Result<()> {
        self.tx
            .send(SupervisorMessage::CheckHealth(app_name.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send health check message: {}", e))
    }

    pub async fn notify_process_exit(&self, app_name: &str, exit_code: i64) -> Result<()> {
        self.tx
            .send(SupervisorMessage::ProcessExit(
                app_name.to_string(),
                exit_code,
            ))
            .await
            .map_err(|e| anyhow!("Failed to send process exit message: {}", e))
    }

    pub fn is_app_running(&self, app_name: &str) -> bool {
        let processes = self.running_processes.lock().unwrap();
        processes.contains_key(app_name)
    }

    pub async fn get_app_stats(&self, app_name: &str) -> Result<Option<AppStats>> {
        // Get app from database
        let app = match db::apps::get_by_name(&self.db_pool, app_name).await? {
            Some(app) => app,
            None => return Ok(None),
        };

        // Check if app is running
        let uptime = {
            let processes = self.running_processes.lock().unwrap();
            processes.get(app_name).map(|p| p.started_at.elapsed())
        };

        // Get process history
        let history = db::process_history::get_by_app_id(&self.db_pool, &app.id).await?;

        // Calculate stats
        let stats = AppStats {
            app_name: app.name,
            state: app.state,
            uptime,
            pid: app.process_id,
            restart_count: app.restart_count,
            last_exit_code: app.last_exit_code,
            last_exit_time: app.last_exit_time,
            total_runs: history.len() as u32,
        };

        Ok(Some(stats))
    }
}

pub struct AppStats {
    pub app_name: String,
    pub state: AppState,
    pub uptime: Option<Duration>,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_exit_code: Option<i64>,
    pub last_exit_time: Option<chrono::DateTime<Utc>>,
    pub total_runs: u32,
}
