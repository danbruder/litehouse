use crate::caddy;
use crate::config::ServerConfig;
use crate::db::system_config as db_system_config;
use crate::litestream;
use bollard::Docker;
use sqlx::{Pool, Sqlite};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Result of a single reconciliation phase
#[derive(Debug, Clone)]
pub enum PhaseResult {
    /// Already correct, no action needed
    Healthy,
    /// Was wrong, now fixed (with description)
    Fixed(String),
    /// Could not fix (with error)
    Failed(String),
    /// Intentionally skipped (e.g., not configured)
    Skipped(String),
}

impl PhaseResult {
    pub fn is_healthy(&self) -> bool {
        matches!(self, PhaseResult::Healthy)
    }

    pub fn is_fixed(&self) -> bool {
        matches!(self, PhaseResult::Fixed(_))
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, PhaseResult::Failed(_))
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, PhaseResult::Skipped(_))
    }
}

/// Report for a single phase
#[derive(Debug, Clone)]
pub struct PhaseReport {
    pub name: String,
    pub result: PhaseResult,
    pub duration: Duration,
}

/// Complete reconciliation report
#[derive(Debug)]
pub struct ReconcileReport {
    pub phases: Vec<PhaseReport>,
    pub total_duration: Duration,
}

impl ReconcileReport {
    /// Returns true if any phase was fixed or failed
    pub fn has_fixes_or_failures(&self) -> bool {
        self.phases
            .iter()
            .any(|p| p.result.is_fixed() || p.result.is_failed())
    }

    /// Returns true if any phase failed
    pub fn has_failures(&self) -> bool {
        self.phases.iter().any(|p| p.result.is_failed())
    }

    /// Count of healthy phases
    pub fn healthy_count(&self) -> usize {
        self.phases.iter().filter(|p| p.result.is_healthy()).count()
    }

    /// Count of fixed phases
    pub fn fixed_count(&self) -> usize {
        self.phases.iter().filter(|p| p.result.is_fixed()).count()
    }

    /// Count of failed phases
    pub fn failed_count(&self) -> usize {
        self.phases.iter().filter(|p| p.result.is_failed()).count()
    }

    /// Count of skipped phases
    pub fn skipped_count(&self) -> usize {
        self.phases.iter().filter(|p| p.result.is_skipped()).count()
    }
}

/// Reconciler that ensures system components are running and properly configured
#[derive(Clone)]
pub struct Reconciler {
    db_pool: Pool<Sqlite>,
    docker: Docker,
}

impl Reconciler {
    pub fn new(db_pool: Pool<Sqlite>, docker: Docker) -> Self {
        Self { db_pool, docker }
    }

    /// Check and reconcile the Caddy container
    async fn check_caddy_container(&self, config: &ServerConfig) -> PhaseResult {
        // Check current state before attempting to start
        let container_name = "caddy-container";
        let was_running = self.is_container_running(container_name).await;

        match caddy::start(&self.docker, config).await {
            Ok(()) => {
                if was_running {
                    PhaseResult::Healthy
                } else {
                    PhaseResult::Fixed("started container".to_string())
                }
            }
            Err(e) => PhaseResult::Failed(e.to_string()),
        }
    }

    /// Check and reconcile Caddy configuration
    async fn check_caddy_config(&self) -> PhaseResult {
        // We always sync config to ensure it matches the database
        // The sync is idempotent - if config already matches, it's a no-op from Caddy's perspective
        match caddy::sync_configuration(&self.docker, &self.db_pool).await {
            Ok(()) => {
                // Get app count for reporting
                match crate::db::app::get_all_with_ports(&self.db_pool).await {
                    Ok(apps) => {
                        if apps.is_empty() {
                            PhaseResult::Healthy
                        } else {
                            PhaseResult::Fixed(format!("synced {} app routes", apps.len()))
                        }
                    }
                    Err(_) => PhaseResult::Healthy,
                }
            }
            Err(e) => PhaseResult::Failed(e.to_string()),
        }
    }

    /// Check and reconcile the Litestream container
    async fn check_litestream_container(&self) -> PhaseResult {
        // Check if S3 is configured
        let s3_config = match db_system_config::get_s3_config(&self.db_pool).await {
            Ok(config) => config,
            Err(e) => {
                return PhaseResult::Failed(format!("failed to check S3 config: {}", e));
            }
        };

        if s3_config.is_none() {
            return PhaseResult::Skipped("S3 not configured".to_string());
        }

        // Check current state before attempting to start
        let container_name = "litestream-container";
        let was_running = self.is_container_running(container_name).await;

        match litestream::start_with_pool(&self.docker, &self.db_pool).await {
            Ok(()) => {
                if was_running {
                    PhaseResult::Healthy
                } else {
                    PhaseResult::Fixed("started container".to_string())
                }
            }
            Err(e) => PhaseResult::Failed(e.to_string()),
        }
    }

    /// Check and reconcile Litestream configuration
    async fn check_litestream_config(&self) -> PhaseResult {
        // Check if S3 is configured
        let s3_config = match db_system_config::get_s3_config(&self.db_pool).await {
            Ok(config) => config,
            Err(e) => {
                return PhaseResult::Failed(format!("failed to check S3 config: {}", e));
            }
        };

        if s3_config.is_none() {
            return PhaseResult::Skipped("S3 not configured".to_string());
        }

        match litestream::sync_configuration(&self.docker, &self.db_pool).await {
            Ok(()) => PhaseResult::Fixed("synced configuration".to_string()),
            Err(e) => PhaseResult::Failed(e.to_string()),
        }
    }

    /// Check and restore app databases from S3
    async fn check_app_databases_restored(&self) -> PhaseResult {
        // Check if S3 is configured
        let s3_config = match db_system_config::get_s3_config(&self.db_pool).await {
            Ok(config) => config,
            Err(e) => return PhaseResult::Failed(format!("failed to check S3 config: {}", e)),
        };

        if s3_config.is_none() {
            return PhaseResult::Skipped("S3 not configured".to_string());
        }

        // Get all apps from database
        let apps = match crate::db::app::get_all(&self.db_pool).await {
            Ok(apps) => apps,
            Err(e) => return PhaseResult::Failed(format!("failed to get apps: {}", e)),
        };

        if apps.is_empty() {
            return PhaseResult::Healthy;
        }

        // Restore each app database
        let mut checked_count = 0;
        for app in apps {
            let volume_name = crate::volume::get_app_volume_name(&app.id);

            // Create volume if needed (idempotent)
            if let Err(e) = crate::volume::create_app_volume(&self.docker, &app.id).await {
                warn!("Failed to create volume for app {}: {}", app.name, e);
                continue;
            }

            // Restore if needed (idempotent - checks if DB exists first)
            match crate::litestream::restore_if_needed(
                &self.docker,
                &self.db_pool,
                &app.id,
                &volume_name,
            )
            .await
            {
                Ok(_) => checked_count += 1,
                Err(e) => warn!("Failed to restore database for app {}: {}", app.name, e),
            }
        }

        PhaseResult::Fixed(format!("checked {} app databases", checked_count))
    }

    /// Helper to check if a container is currently running
    async fn is_container_running(&self, container_name: &str) -> bool {
        let list_options = bollard::container::ListContainersOptions::<String> {
            all: false, // Only running containers
            ..Default::default()
        };

        match self.docker.list_containers(Some(list_options)).await {
            Ok(containers) => containers.iter().any(|c| {
                c.names.as_ref().map_or(false, |names| {
                    names
                        .iter()
                        .any(|n| n == container_name || n == &format!("/{}", container_name))
                })
            }),
            Err(_) => false,
        }
    }

    /// Run a single phase and record timing
    async fn run_phase<F, Fut>(&self, name: &str, check_fn: F) -> PhaseReport
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = PhaseResult>,
    {
        let start = Instant::now();
        let result = check_fn().await;
        let duration = start.elapsed();

        PhaseReport {
            name: name.to_string(),
            result,
            duration,
        }
    }

    /// Run all reconciliation phases
    pub async fn reconcile_all(&self, config: &ServerConfig) -> ReconcileReport {
        let start = Instant::now();
        let mut phases = Vec::new();

        // Phase 1: Caddy container
        let report = self
            .run_phase("Caddy container", || self.check_caddy_container(config))
            .await;
        phases.push(report);

        // Phase 2: Caddy config (only if container is healthy/fixed)
        let caddy_container_ok = phases
            .last()
            .map(|p| !p.result.is_failed())
            .unwrap_or(false);

        if caddy_container_ok {
            let report = self
                .run_phase("Caddy config", || self.check_caddy_config())
                .await;
            phases.push(report);
        } else {
            phases.push(PhaseReport {
                name: "Caddy config".to_string(),
                result: PhaseResult::Skipped("container not running".to_string()),
                duration: Duration::ZERO,
            });
        }

        // Phase 3: Litestream container
        let report = self
            .run_phase("Litestream container", || {
                self.check_litestream_container()
            })
            .await;
        phases.push(report);

        // Phase 4: Litestream config (only if container is healthy/fixed)
        let litestream_container_ok = phases
            .last()
            .map(|p| !p.result.is_failed() && !p.result.is_skipped())
            .unwrap_or(false);

        if litestream_container_ok {
            let report = self
                .run_phase("Litestream config", || self.check_litestream_config())
                .await;
            phases.push(report);
        } else {
            // Get the skip reason from the container phase
            let skip_reason = phases
                .iter()
                .rev()
                .find(|p| p.name == "Litestream container")
                .and_then(|p| match &p.result {
                    PhaseResult::Skipped(reason) => Some(reason.clone()),
                    PhaseResult::Failed(_) => Some("container not running".to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "container not running".to_string());

            phases.push(PhaseReport {
                name: "Litestream config".to_string(),
                result: PhaseResult::Skipped(skip_reason),
                duration: Duration::ZERO,
            });
        }

        // Phase 5: App database restore (only if Litestream is configured)
        let litestream_configured = phases
            .iter()
            .find(|p| p.name == "Litestream config")
            .map(|p| !p.result.is_skipped())
            .unwrap_or(false);

        if litestream_configured {
            let report = self
                .run_phase("App database restore", || {
                    self.check_app_databases_restored()
                })
                .await;
            phases.push(report);
        } else {
            phases.push(PhaseReport {
                name: "App database restore".to_string(),
                result: PhaseResult::Skipped("S3 not configured".to_string()),
                duration: Duration::ZERO,
            });
        }

        ReconcileReport {
            phases,
            total_duration: start.elapsed(),
        }
    }

    /// Log the reconciliation report in human-readable format
    pub fn log_report(report: &ReconcileReport) {
        let total_ms = report.total_duration.as_millis();

        if report.has_failures() {
            warn!(
                "Reconciliation completed with errors (took {}ms)",
                total_ms
            );
        } else if report.has_fixes_or_failures() {
            info!(
                "Reconciliation completed with fixes (took {}ms)",
                total_ms
            );
        } else {
            info!("Reconciliation complete (took {}ms)", total_ms);
        }

        for phase in &report.phases {
            let phase_ms = phase.duration.as_millis();
            match &phase.result {
                PhaseResult::Healthy => {
                    info!("  {}: Healthy ({}ms)", phase.name, phase_ms);
                }
                PhaseResult::Fixed(desc) => {
                    info!("  {}: Fixed - {} ({}ms)", phase.name, desc, phase_ms);
                }
                PhaseResult::Failed(err) => {
                    warn!("  {}: Failed - {} ({}ms)", phase.name, err, phase_ms);
                }
                PhaseResult::Skipped(reason) => {
                    info!("  {}: Skipped - {} ({}ms)", phase.name, reason, phase_ms);
                }
            }
        }
    }
}
