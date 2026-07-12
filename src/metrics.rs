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
