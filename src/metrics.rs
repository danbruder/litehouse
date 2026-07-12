//! Resource metrics: pure parsing/calculation helpers (this file) plus the
//! async sampler driver (Task 7) wired into `lh serve` (Task 8).

use bollard::container::CPUStats;

/// Parsed fields from the aggregate `cpu` line of `/proc/stat` (USER_HZ
/// jiffies since boot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcStatCpu {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl ProcStatCpu {
    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq + self.steal
    }
    fn idle_total(&self) -> u64 {
        self.idle + self.iowait
    }
}

/// Parse the first line of `/proc/stat` (starts with `cpu `). `None` if the
/// line is missing or has fewer fields than expected.
pub fn parse_proc_stat_cpu_line(contents: &str) -> Option<ProcStatCpu> {
    let line = contents.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|f| f.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;
    if fields.len() < 7 {
        return None;
    }
    Some(ProcStatCpu {
        user: fields[0],
        nice: fields[1],
        system: fields[2],
        idle: fields[3],
        iowait: fields[4],
        irq: fields[5],
        softirq: fields[6],
        steal: fields.get(7).copied().unwrap_or(0),
    })
}

/// CPU percent over the interval between two `/proc/stat` readings
/// ("1 - idle_delta/total_delta"). `None` if no time elapsed or the
/// readings moved backwards (e.g. counter reset).
pub fn cpu_pct_from_proc_stat(prev: &ProcStatCpu, curr: &ProcStatCpu) -> Option<f64> {
    let total_delta = curr.total().checked_sub(prev.total())?;
    let idle_delta = curr.idle_total().checked_sub(prev.idle_total())?;
    if total_delta == 0 {
        return None;
    }
    let busy_delta = total_delta.saturating_sub(idle_delta);
    Some((busy_delta as f64 / total_delta as f64) * 100.0)
}

/// Parse `MemTotal`/`MemAvailable` (in kB) out of `/proc/meminfo` into
/// `(used_bytes, total_bytes)`.
pub fn mem_usage_from_meminfo(contents: &str) -> Option<(i64, i64)> {
    let mut total_kb = None;
    let mut available_kb = None;
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest.split_whitespace().next().and_then(|s| s.parse::<i64>().ok());
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available_kb = rest.split_whitespace().next().and_then(|s| s.parse::<i64>().ok());
        }
    }
    let total_kb = total_kb?;
    let available_kb = available_kb?;
    let used_kb = (total_kb - available_kb).max(0);
    Some((used_kb * 1024, total_kb * 1024))
}

/// Parse the second line of `df -B1 <path>` output into `(used_bytes,
/// total_bytes)`. Columns: Filesystem, 1B-blocks, Used, Available, Use%, Mounted.
pub fn parse_df_output(stdout: &str) -> Option<(i64, i64)> {
    let data_line = stdout.lines().nth(1)?;
    let fields: Vec<&str> = data_line.split_whitespace().collect();
    if fields.len() < 3 {
        return None;
    }
    let total = fields[1].parse::<i64>().ok()?;
    let used = fields[2].parse::<i64>().ok()?;
    Some((used, total))
}

/// Docker's own CPU% formula: `cpu_delta / system_delta * online_cpus * 100`.
/// `None` when either delta is non-positive (e.g. a container's very first
/// sample).
pub fn cpu_pct_from_docker_stats(curr: &CPUStats, prev: &CPUStats) -> Option<f64> {
    let cpu_delta = curr.cpu_usage.total_usage.checked_sub(prev.cpu_usage.total_usage)?;
    let system_delta = curr.system_cpu_usage?.checked_sub(prev.system_cpu_usage?)?;
    if cpu_delta == 0 || system_delta == 0 {
        return None;
    }
    let online_cpus = curr.online_cpus.unwrap_or(1).max(1) as f64;
    Some((cpu_delta as f64 / system_delta as f64) * online_cpus * 100.0)
}

use anyhow::{anyhow, Result};
use bollard::container::{Stats, StatsOptions};
use bollard::Docker;
use chrono::Timelike;
use futures_util::StreamExt;
use std::collections::HashMap;
use tracing::warn;

use crate::{config, db};

/// Live host memory usage: `(used_bytes, total_bytes)`.
pub async fn mem_usage() -> Result<(i64, i64)> {
    let contents = tokio::fs::read_to_string("/proc/meminfo").await?;
    mem_usage_from_meminfo(&contents).ok_or_else(|| anyhow!("failed to parse /proc/meminfo"))
}

/// Live disk usage of the filesystem backing the backups/data directory, via
/// `df -B1`: `(used_bytes, total_bytes)`. Shelling out (rather than adding a
/// `statvfs`-wrapping crate) matches how this codebase already invokes
/// system CLIs for one-off queries (e.g. `docker build`).
pub async fn disk_usage() -> Result<(i64, i64)> {
    let dir = config::get_backups_dir().map_err(|e| anyhow!("{e}"))?;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("df").arg("-B1").arg(&dir).output(),
    )
    .await
    .map_err(|_| anyhow!("df timed out after 5s"))??;
    if !output.status.success() {
        return Err(anyhow!("df exited with {}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_df_output(&stdout).ok_or_else(|| anyhow!("unparseable df output: {stdout}"))
}

async fn docker_stats_once(docker: &Docker, container_name: &str) -> Result<Stats> {
    let mut stream = docker.stats(container_name, Some(StatsOptions { stream: false, one_shot: false }));
    match stream.next().await {
        Some(Ok(stats)) => Ok(stats),
        Some(Err(e)) => Err(e.into()),
        None => Err(anyhow!("no stats returned for container '{container_name}'")),
    }
}

async fn app_volume_size(docker: &Docker, app_id: &str) -> Result<i64> {
    let volume_name = crate::volume::get_app_volume_name(app_id);
    let usage = docker.df().await?;
    let size = usage
        .volumes
        .unwrap_or_default()
        .into_iter()
        .find(|v| v.name == volume_name)
        .and_then(|v| v.usage_data)
        .map(|u| u.size)
        .unwrap_or(0);
    Ok(size)
}

/// One 60-second sampling tick: reads server-wide CPU/mem/disk and, for
/// every currently-running app, its container CPU/mem plus (every 10th tick
/// only — data-volume size changes slowly and `docker df` walks the whole
/// volume) its data size, persisting all of it to `metric_sample`. Never
/// panics — a sampling failure for one scope just logs a warning and skips
/// that scope's row for this tick.
pub async fn run_tick(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    docker: &Docker,
    prev_host_cpu: &mut Option<ProcStatCpu>,
    tick_count: u64,
    prev_app_disk: &mut HashMap<String, i64>,
) {
    let ts = chrono::Utc::now().to_rfc3339();
    sample_server(pool, &ts, prev_host_cpu).await;

    let apps = match db::app::get_all(pool).await {
        Ok(apps) => apps,
        Err(e) => {
            warn!("metrics: failed to list apps for sampling: {e:#}");
            return;
        }
    };

    prev_app_disk.retain(|id, _| apps.iter().any(|a| &a.id == id));

    for app in apps {
        sample_app(pool, docker, &app, &ts, tick_count, prev_app_disk).await;
    }
}

async fn sample_server(pool: &sqlx::Pool<sqlx::Sqlite>, ts: &str, prev_host_cpu: &mut Option<ProcStatCpu>) {
    let cpu_pct = match tokio::fs::read_to_string("/proc/stat").await {
        Ok(contents) => match parse_proc_stat_cpu_line(&contents) {
            Some(curr) => {
                let pct = prev_host_cpu.as_ref().and_then(|prev| cpu_pct_from_proc_stat(prev, &curr));
                *prev_host_cpu = Some(curr);
                pct
            }
            None => {
                warn!("metrics: failed to parse /proc/stat");
                None
            }
        },
        Err(e) => {
            warn!("metrics: failed to read /proc/stat: {e}");
            None
        }
    };

    let mem_bytes = mem_usage().await.map(|(used, _)| used).ok();
    let disk_bytes = disk_usage().await.map(|(used, _)| used).ok();

    if let Err(e) = db::metrics::insert_sample(pool, ts, "server", cpu_pct, mem_bytes, disk_bytes).await {
        warn!("metrics: failed to save server sample: {e:#}");
    }
}

async fn sample_app(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    docker: &Docker,
    app: &crate::models::App,
    ts: &str,
    tick_count: u64,
    prev_app_disk: &mut HashMap<String, i64>,
) {
    let live = crate::docker::live_state(&app.name).await.unwrap_or(app.state);
    if live != crate::models::AppState::Running {
        return;
    }

    let container_name = format!("{}-container", app.name);
    let (cpu_pct, mem_bytes) = match docker_stats_once(docker, &container_name).await {
        Ok(stats) => (
            cpu_pct_from_docker_stats(&stats.cpu_stats, &stats.precpu_stats),
            stats.memory_stats.usage.map(|u| u as i64),
        ),
        Err(e) => {
            warn!("metrics: failed to sample container stats for '{}': {e:#}", app.name);
            (None, None)
        }
    };

    let disk_bytes = if tick_count % 10 == 0 {
        match app_volume_size(docker, &app.id).await {
            Ok(size) => {
                prev_app_disk.insert(app.id.clone(), size);
                Some(size)
            }
            Err(e) => {
                warn!("metrics: failed to measure data volume for '{}': {e:#}", app.name);
                prev_app_disk.get(&app.id).copied()
            }
        }
    } else {
        prev_app_disk.get(&app.id).copied()
    };

    if let Err(e) = db::metrics::insert_sample(pool, ts, &app.id, cpu_pct, mem_bytes, disk_bytes).await {
        warn!("metrics: failed to save sample for app '{}': {e:#}", app.name);
    }
}

/// Roll the most recently completed UTC hour's samples into `metric_hourly`,
/// then prune samples older than 24h and hourly rows older than 30 days.
/// Called once per hour from the sampler loop in `commands::server::execute`.
pub async fn rollup_and_prune(pool: &sqlx::Pool<sqlx::Sqlite>) {
    let now = chrono::Utc::now();
    let hour_end = now
        .date_naive()
        .and_hms_opt(now.hour(), 0, 0)
        .expect("hour/0/0 is always a valid time")
        .and_utc();
    let hour_start = hour_end - chrono::Duration::hours(1);
    let hour_start_s = hour_start.to_rfc3339();
    let hour_end_s = hour_end.to_rfc3339();

    if let Err(e) = db::metrics::rollup_hour(pool, &hour_start_s, &hour_end_s).await {
        warn!("metrics: hourly rollup failed: {e:#}");
    }

    let sample_cutoff = (now - chrono::Duration::hours(24)).to_rfc3339();
    if let Err(e) = db::metrics::prune_samples_older_than(pool, &sample_cutoff).await {
        warn!("metrics: sample prune failed: {e:#}");
    }

    let hourly_cutoff = (now - chrono::Duration::days(30)).to_rfc3339();
    if let Err(e) = db::metrics::prune_hourly_older_than(pool, &hourly_cutoff).await {
        warn!("metrics: hourly prune failed: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu(user: u64, idle: u64) -> ProcStatCpu {
        ProcStatCpu { user, nice: 0, system: 0, idle, iowait: 0, irq: 0, softirq: 0, steal: 0 }
    }

    #[test]
    fn parse_proc_stat_cpu_line_reads_first_seven_fields() {
        let contents = "cpu  100 0 50 850 0 0 0 0\ncpu0 100 0 50 850 0 0 0 0\n";
        let parsed = parse_proc_stat_cpu_line(contents).unwrap();
        assert_eq!(parsed.user, 100);
        assert_eq!(parsed.system, 50);
        assert_eq!(parsed.idle, 850);
    }

    #[test]
    fn parse_proc_stat_cpu_line_missing_returns_none() {
        assert!(parse_proc_stat_cpu_line("not stat data").is_none());
    }

    #[test]
    fn cpu_pct_from_proc_stat_computes_busy_fraction() {
        let prev = cpu(0, 0);
        let curr = cpu(100, 900);
        assert_eq!(cpu_pct_from_proc_stat(&prev, &curr), Some(10.0));
    }

    #[test]
    fn cpu_pct_from_proc_stat_zero_elapsed_returns_none() {
        let snapshot = cpu(100, 900);
        assert_eq!(cpu_pct_from_proc_stat(&snapshot, &snapshot), None);
    }

    #[test]
    fn mem_usage_from_meminfo_computes_used_as_total_minus_available() {
        let contents = "MemTotal:       16384000 kB\nMemAvailable:    8192000 kB\n";
        let (used, total) = mem_usage_from_meminfo(contents).unwrap();
        assert_eq!(total, 16384000 * 1024);
        assert_eq!(used, 8192000 * 1024);
    }

    #[test]
    fn mem_usage_from_meminfo_missing_field_returns_none() {
        assert!(mem_usage_from_meminfo("MemTotal: 1000 kB\n").is_none());
    }

    #[test]
    fn parse_df_output_reads_used_and_total() {
        let stdout = "Filesystem      1B-blocks       Used   Available Use% Mounted on\n/dev/sda1  25000000000 4100000000 20000000000  18% /\n";
        let (used, total) = parse_df_output(stdout).unwrap();
        assert_eq!(total, 25_000_000_000);
        assert_eq!(used, 4_100_000_000);
    }

    #[test]
    fn parse_df_output_missing_data_line_returns_none() {
        assert!(parse_df_output("Filesystem 1B-blocks Used\n").is_none());
    }

    fn docker_cpu(total_usage: u64, system_cpu_usage: u64, online_cpus: u64) -> CPUStats {
        CPUStats {
            cpu_usage: bollard::container::CPUUsage {
                percpu_usage: None,
                usage_in_usermode: 0,
                total_usage,
                usage_in_kernelmode: 0,
            },
            system_cpu_usage: Some(system_cpu_usage),
            online_cpus: Some(online_cpus),
            throttling_data: bollard::container::ThrottlingData { periods: 0, throttled_periods: 0, throttled_time: 0 },
        }
    }

    #[test]
    fn cpu_pct_from_docker_stats_computes_percentage() {
        let prev = docker_cpu(1_000_000_000, 10_000_000_000, 2);
        let curr = docker_cpu(1_200_000_000, 11_000_000_000, 2);
        // cpu_delta=200_000_000, system_delta=1_000_000_000 -> 0.2 * 2 * 100 = 40%
        assert_eq!(cpu_pct_from_docker_stats(&curr, &prev), Some(40.0));
    }

    #[test]
    fn cpu_pct_from_docker_stats_no_elapsed_time_returns_none() {
        let snapshot = docker_cpu(1_000_000_000, 10_000_000_000, 2);
        assert_eq!(cpu_pct_from_docker_stats(&snapshot, &snapshot), None);
    }
}
