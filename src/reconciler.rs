use crate::caddy;
use crate::config::ServerConfig;
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
