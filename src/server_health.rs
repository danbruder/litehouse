//! Server health & security digest: the checklist behind the admin UI's
//! `/health` page, `GET /api/server/health`, `lh server-health` and the MCP
//! `server_health` tool.
//!
//! Two halves, because litehouse-server runs in a container and can't see
//! the host's firewall, sshd, apt or reboot state:
//!
//! - **Host facts** ([`HostReport`]) are collected on the host, as root, by
//!   `lh server-health collect --write <path>`, run every 15 minutes by the
//!   `litehouse-host-health.timer` systemd unit (installed by `lh install` and
//!   every `lh upgrade`). The report lands in `/opt/litehouse/health/`, which
//!   is bind-mounted read-only into the server container. Collection only
//!   reads: `ufw status`, `ss`, `iptables -S`, `sshd -T`, `apt list`,
//!   `df`, `/proc`, and the cert files in Caddy's data volume.
//! - **Server facts** ([`ServerFacts`]) — backups, apps, published container
//!   ports, Caddy's admin API — are gathered live by the server per request,
//!   from its DB and the Docker API.
//!
//! [`evaluate`] turns both into a list of [`Check`]s. It is pure, so the
//! thresholds are unit-tested below. Host facts are raw observations, not
//! verdicts, so changing a threshold only needs a server upgrade.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

use crate::commands::os_updates::OsUpdatesStatus;

/// Where the host timer writes the report, and where the server reads it
/// (the same path on both sides of the bind mount).
pub const HOST_REPORT_PATH: &str = "/opt/litehouse/health/host.json";
pub const HOST_REPORT_ENV: &str = "LITEHOUSE_HOST_REPORT";

/// Where Caddy keeps the certificates it manages (the `caddy_data` volume's
/// mountpoint on a stock Docker install).
const CADDY_CERT_DIR: &str = "/var/lib/docker/volumes/caddy_data/_data/caddy/certificates";

/// Ports that are meant to be reachable from the internet.
const EXPECTED_PUBLIC_TCP: [u16; 3] = [22, 80, 443];

/// The host report is refreshed every 15 minutes; give it a few misses
/// before calling it stale.
const HOST_REPORT_STALE_MINUTES: i64 = 45;

pub fn host_report_path() -> String {
    std::env::var(HOST_REPORT_ENV).unwrap_or_else(|_| HOST_REPORT_PATH.to_string())
}

// ---------------------------------------------------------------------
// Host report (collected on the host)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostReport {
    pub generated_at: DateTime<Utc>,
    pub lh_version: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub uptime_secs: Option<u64>,
    /// `None` when ufw isn't installed.
    pub ufw: Option<UfwStatus>,
    /// Sockets listening on any address, from `ss -tulpn`.
    pub listeners: Vec<Listener>,
    /// The DOCKER-USER rule that drops WAN traffic to Caddy's admin port.
    /// `None` when iptables couldn't be read.
    pub caddy_admin_guard: Option<bool>,
    pub sshd: Option<SshdSettings>,
    pub fail2ban_active: Option<bool>,
    pub os_updates: Option<OsUpdatesStatus>,
    pub upgradable: Option<Upgradable>,
    pub disk: Option<DiskUsage>,
    pub memory: Option<MemoryUsage>,
    pub clock_synced: Option<bool>,
    pub certificates: Vec<CertInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UfwStatus {
    pub active: bool,
    /// "deny", "reject" or "allow".
    pub default_incoming: Option<String>,
    /// Allow rules, e.g. `22/tcp`, deduplicated across IPv4/IPv6.
    pub allowed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Listener {
    /// "tcp" or "udp".
    pub proto: String,
    pub addr: String,
    pub port: u16,
    pub process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SshdSettings {
    pub permit_root_login: Option<String>,
    pub password_authentication: Option<String>,
    pub kbd_interactive_authentication: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Upgradable {
    pub total: usize,
    /// Package names whose candidate comes from a `-security` suite.
    pub security: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryUsage {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CertInfo {
    /// The hostname Caddy filed it under (`wildcard_.example.com` for a
    /// wildcard certificate).
    pub host: String,
    pub not_after: DateTime<Utc>,
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Parse `ufw status verbose`.
pub fn parse_ufw_status(s: &str) -> UfwStatus {
    let active = s.lines().any(|l| l.trim() == "Status: active");
    let default_incoming = s
        .lines()
        .find_map(|l| l.strip_prefix("Default:"))
        .and_then(|d| {
            d.split(',')
                .find(|p| p.contains("(incoming)"))
                .map(|p| p.replace("(incoming)", "").trim().to_string())
        });
    let mut allowed = BTreeSet::new();
    let mut in_rules = false;
    for line in s.lines() {
        if line.starts_with("--") {
            in_rules = true;
            continue;
        }
        if !in_rules || !line.contains("ALLOW") {
            continue;
        }
        // "22/tcp (v6)                ALLOW IN    Anywhere (v6)"
        let to = line.split("ALLOW").next().unwrap_or("").replace("(v6)", "");
        let to = to.trim();
        if !to.is_empty() {
            allowed.insert(to.to_string());
        }
    }
    UfwStatus {
        active,
        default_incoming,
        allowed: allowed.into_iter().collect(),
    }
}

/// Parse `ss -H -tulpn`.
pub fn parse_ss(s: &str) -> Vec<Listener> {
    let mut out = Vec::new();
    for line in s.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let proto = fields[0].to_string();
        if proto != "tcp" && proto != "udp" {
            continue;
        }
        let Some((addr, port)) = fields[4].rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        let addr = addr
            .trim_start_matches('[')
            .split(['%', ']'])
            .next()
            .unwrap_or("")
            .to_string();
        let process = line
            .split("((\"")
            .nth(1)
            .and_then(|p| p.split('"').next())
            .map(str::to_string);
        let l = Listener { proto, addr, port, process };
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// Whether a listener's address is reachable from off the box. Loopback is
/// not, and neither are Docker's bridge networks (172.16.0.0/12).
pub fn is_public_addr(addr: &str) -> bool {
    let addr = addr.trim_start_matches("::ffff:");
    if addr.starts_with("127.") || addr == "::1" || addr == "localhost" {
        return false;
    }
    if let Some(rest) = addr.strip_prefix("172.")
        && let Some(second) = rest.split('.').next().and_then(|o| o.parse::<u8>().ok())
        && (16..=31).contains(&second)
    {
        return false;
    }
    true
}

/// `sshd -T` prints the effective config as lowercase `key value` lines.
pub fn parse_sshd_t(s: &str) -> SshdSettings {
    let get = |key: &str| {
        s.lines().find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k.eq_ignore_ascii_case(key)).then(|| v.trim().to_lowercase())
        })
    };
    SshdSettings {
        permit_root_login: get("permitrootlogin"),
        password_authentication: get("passwordauthentication"),
        kbd_interactive_authentication: get("kbdinteractiveauthentication"),
    }
}

/// Fallback when `sshd -T` can't run: read the config files the way sshd
/// does (first value wins, drop-ins are included before the main file on
/// Debian/Ubuntu), ignoring `Match` blocks, then apply OpenSSH's defaults.
fn sshd_from_files() -> Option<SshdSettings> {
    let mut texts = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/etc/ssh/sshd_config.d") {
        let mut paths: Vec<_> = dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "conf"))
            .collect();
        paths.sort();
        for p in paths {
            texts.push(std::fs::read_to_string(p).unwrap_or_default());
        }
    }
    texts.push(std::fs::read_to_string("/etc/ssh/sshd_config").ok()?);
    let mut found: HashMap<String, String> = HashMap::new();
    for text in &texts {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(k), Some(v)) = (parts.next(), parts.next()) else {
                continue;
            };
            let k = k.to_lowercase();
            if k == "match" {
                break;
            }
            found.entry(k).or_insert_with(|| v.to_lowercase());
        }
    }
    Some(SshdSettings {
        permit_root_login: Some(found.get("permitrootlogin").cloned().unwrap_or("prohibit-password".into())),
        password_authentication: Some(found.get("passwordauthentication").cloned().unwrap_or("yes".into())),
        kbd_interactive_authentication: found
            .get("kbdinteractiveauthentication")
            .or(found.get("challengeresponseauthentication"))
            .cloned()
            .or(Some("yes".into())),
    })
}

/// Parse `apt list --upgradable`.
pub fn parse_apt_upgradable(s: &str) -> Upgradable {
    let mut total = 0;
    let mut security = Vec::new();
    for line in s.lines() {
        // "openssl/noble-updates,noble-security 3.0.13-0ubuntu3.5 amd64 [upgradable from: ...]"
        if !line.contains("[upgradable from") {
            continue;
        }
        total += 1;
        let first = line.split_whitespace().next().unwrap_or("");
        if let Some((name, suites)) = first.split_once('/')
            && suites.split(',').any(|s| s.ends_with("-security"))
        {
            security.push(name.to_string());
        }
    }
    Upgradable { total, security }
}

/// Parse `df -P -B1 <path>`.
pub fn parse_df(s: &str) -> Option<DiskUsage> {
    let line = s.lines().nth(1)?;
    let f: Vec<&str> = line.split_whitespace().collect();
    Some(DiskUsage {
        total_bytes: f.get(1)?.parse().ok()?,
        available_bytes: f.get(3)?.parse().ok()?,
    })
}

pub fn parse_meminfo(s: &str) -> Option<MemoryUsage> {
    let kb = |key: &str| -> Option<u64> {
        s.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map(|v| v * 1024)
    };
    Some(MemoryUsage {
        total_bytes: kb("MemTotal:")?,
        available_bytes: kb("MemAvailable:")?,
    })
}

/// Parse `openssl x509 -noout -enddate` (`notAfter=Feb  2 12:00:00 2027 GMT`).
pub fn parse_openssl_enddate(s: &str) -> Option<DateTime<Utc>> {
    let v = s.trim().strip_prefix("notAfter=")?;
    let v = v.trim_end_matches("GMT").split_whitespace().collect::<Vec<_>>().join(" ");
    let naive = chrono::NaiveDateTime::parse_from_str(&v, "%b %d %H:%M:%S %Y").ok()?;
    Some(naive.and_utc())
}

fn collect_certificates() -> Vec<CertInfo> {
    let mut best: HashMap<String, DateTime<Utc>> = HashMap::new();
    for entry in walkdir::WalkDir::new(CADDY_CERT_DIR)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "crt") {
            continue;
        }
        let Some(host) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let Some(path_str) = path.to_str() else { continue };
        let Some(not_after) = output("openssl", &["x509", "-noout", "-enddate", "-in", path_str])
            .as_deref()
            .and_then(parse_openssl_enddate)
        else {
            continue;
        };
        // Several issuers can hold a cert for the same host; Caddy serves
        // the freshest.
        let e = best.entry(host).or_insert(not_after);
        if not_after > *e {
            *e = not_after;
        }
    }
    let mut certs: Vec<CertInfo> = best
        .into_iter()
        .map(|(host, not_after)| CertInfo { host, not_after })
        .collect();
    certs.sort_by(|a, b| a.host.cmp(&b.host));
    certs
}

/// Gather every host fact. Runs on the host as root; every probe is
/// read-only and failures just leave that field empty.
pub fn collect_host_report() -> HostReport {
    let first_line = |s: Option<String>| s.and_then(|s| s.lines().next().map(|l| l.trim().to_string()));

    let os = std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string())
    });
    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|f| f as u64);

    let ufw = output("ufw", &["status", "verbose"]).map(|s| parse_ufw_status(&s));
    let listeners = output("ss", &["-H", "-tulpn"]).map(|s| parse_ss(&s)).unwrap_or_default();
    let caddy_admin_guard = output("iptables", &["-S", "DOCKER-USER"]).map(|s| {
        s.lines()
            .any(|l| l.contains("--dport 2019") && l.contains("-j DROP"))
    });
    let sshd = output("sshd", &["-T"])
        .map(|s| parse_sshd_t(&s))
        .filter(|s| s.password_authentication.is_some())
        .or_else(sshd_from_files);
    let fail2ban_active = output("systemctl", &["is-active", "fail2ban"])
        .map(|s| s.trim() == "active")
        .or(Some(false));
    let upgradable = output("apt", &["list", "--upgradable"]).map(|s| parse_apt_upgradable(&s));
    let disk = output("df", &["-P", "-B1", "/"]).as_deref().and_then(parse_df);
    let memory = std::fs::read_to_string("/proc/meminfo").ok().as_deref().and_then(parse_meminfo);
    let clock_synced = output("timedatectl", &["show", "-p", "NTPSynchronized", "--value"])
        .map(|s| s.trim() == "yes");

    HostReport {
        generated_at: Utc::now(),
        lh_version: env!("CARGO_PKG_VERSION").to_string(),
        hostname: first_line(output("hostname", &[])),
        os,
        kernel: first_line(output("uname", &["-r"])),
        uptime_secs,
        ufw,
        listeners,
        caddy_admin_guard,
        sshd,
        fail2ban_active,
        os_updates: Some(crate::commands::os_updates::collect()),
        upgradable,
        disk,
        memory,
        clock_synced,
        certificates: collect_certificates(),
    }
}

/// `lh server-health collect [--write PATH]`.
pub fn collect_command(write: Option<String>) -> anyhow::Result<()> {
    let report = collect_host_report();
    let json = serde_json::to_string_pretty(&report)?;
    match write {
        None => println!("{json}"),
        Some(path) => {
            let path = Path::new(&path);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            // Write-then-rename so the server never reads a half-written file.
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json)?;
            std::fs::rename(&tmp, path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// The checklist
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    /// Couldn't be checked (no host report yet, a probe failed).
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Check {
    pub id: String,
    pub group: String,
    pub title: String,
    pub status: CheckStatus,
    /// One line on what was found.
    pub summary: String,
    /// Optional longer explanation (lists of ports, packages, ...).
    pub detail: Option<String>,
    /// How to fix it, when it isn't ok.
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HealthSummary {
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub checked_at: DateTime<Utc>,
    pub host_checked_at: Option<DateTime<Utc>>,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub kernel: Option<String>,
    pub uptime_secs: Option<u64>,
    pub summary: HealthSummary,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppFacts {
    pub name: String,
    /// The state litehouse last put it in (`running`, `stopped`, ...).
    pub desired: String,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedPort {
    pub container: String,
    pub host_ip: String,
    pub host_port: u16,
    pub proto: String,
}

/// What the server knows itself, gathered per request.
#[derive(Debug, Clone, Default)]
pub struct ServerFacts {
    pub now: DateTime<Utc>,
    pub host: Option<HostReport>,
    /// Why the host report couldn't be read, when it couldn't.
    pub host_error: Option<String>,
    /// Hostnames litehouse serves over HTTPS (admin host, app subdomains,
    /// custom domains). Empty in local dev, where there's no TLS.
    pub served_hosts: Vec<String>,
    pub s3_configured: bool,
    pub last_backup_date: Option<String>,
    pub last_backup_failures: Vec<String>,
    pub apps: Vec<AppFacts>,
    /// `None` when Docker couldn't be listed.
    pub published_ports: Option<Vec<PublishedPort>>,
    /// Caddy's `CADDY_ADMIN` listen address, when the container exists.
    pub caddy_admin_listen: Option<String>,
    /// App containers attached to the network Caddy's admin API listens on.
    pub apps_on_caddy_network: Vec<String>,
}

struct Builder {
    checks: Vec<Check>,
    group: &'static str,
}

impl Builder {
    fn group(&mut self, g: &'static str) {
        self.group = g;
    }
    fn push(&mut self, id: &str, title: &str, status: CheckStatus, summary: impl Into<String>) -> &mut Check {
        self.checks.push(Check {
            id: id.into(),
            group: self.group.into(),
            title: title.into(),
            status,
            summary: summary.into(),
            detail: None,
            fix: None,
        });
        self.checks.last_mut().unwrap()
    }
}

trait CheckExt {
    fn detail(&mut self, d: impl Into<String>) -> &mut Self;
    fn fix(&mut self, f: impl Into<String>) -> &mut Self;
}

impl CheckExt for Check {
    fn detail(&mut self, d: impl Into<String>) -> &mut Self {
        self.detail = Some(d.into());
        self
    }
    fn fix(&mut self, f: impl Into<String>) -> &mut Self {
        self.fix = Some(f.into());
        self
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

const NO_HOST_REPORT: &str = "Not checked yet: the host report is missing.";

/// Build the checklist. Pure: everything it looks at is in `f`.
pub fn evaluate(f: &ServerFacts) -> Vec<Check> {
    use CheckStatus::*;
    let mut b = Builder { checks: Vec::new(), group: "" };
    let host = f.host.as_ref();
    let unknown = |b: &mut Builder, id: &str, title: &str| {
        b.push(id, title, Unknown, NO_HOST_REPORT);
    };

    // ---- Network ----------------------------------------------------
    b.group("Network");

    match host.map(|h| h.ufw.as_ref()) {
        None => unknown(&mut b, "firewall", "Firewall is on"),
        Some(None) => {
            b.push("firewall", "Firewall is on", Fail, "ufw isn't installed.")
                .fix("apt install ufw, then re-run lh install's hardening (ufw allow 22,80,443/tcp; ufw enable).");
        }
        Some(Some(ufw)) if !ufw.active => {
            b.push("firewall", "Firewall is on", Fail, "ufw is installed but inactive.")
                .fix("ufw allow 22/tcp && ufw allow 80/tcp && ufw allow 443/tcp && ufw enable");
        }
        Some(Some(ufw)) => {
            let incoming = ufw.default_incoming.as_deref().unwrap_or("unknown");
            if incoming == "deny" || incoming == "reject" {
                b.push("firewall", "Firewall is on", Ok, "ufw is active and blocks incoming traffic by default.")
                    .detail(format!("Allowed: {}", ufw.allowed.join(", ")));
            } else {
                b.push("firewall", "Firewall is on", Warn, format!("ufw is active but its default for incoming traffic is {incoming}."))
                    .fix("ufw default deny incoming");
            }
        }
    }

    match host {
        None => unknown(&mut b, "open_ports", "Only SSH and web ports are open"),
        Some(h) => {
            let ufw_on = h.ufw.as_ref().is_some_and(|u| u.active);
            let allowed_ports: BTreeSet<(u16, String)> = h
                .ufw
                .iter()
                .flat_map(|u| u.allowed.iter())
                .filter_map(|r| {
                    let (port, proto) = r.split_once('/').unwrap_or((r, "tcp"));
                    Some((port.parse::<u16>().ok()?, proto.to_string()))
                })
                .collect();
            // Container ports are covered by the next check (Docker's
            // forwarding bypasses ufw, so they're judged on their own).
            let mut exposed: Vec<String> = h
                .listeners
                .iter()
                .filter(|l| is_public_addr(&l.addr))
                .filter(|l| l.process.as_deref() != Some("docker-proxy"))
                .filter(|l| !(l.proto == "tcp" && EXPECTED_PUBLIC_TCP.contains(&l.port)))
                .filter(|l| !ufw_on || allowed_ports.contains(&(l.port, l.proto.clone())))
                .map(|l| {
                    format!(
                        "{}/{}{}",
                        l.port,
                        l.proto,
                        l.process.as_deref().map(|p| format!(" ({p})")).unwrap_or_default()
                    )
                })
                .collect();
            exposed.sort();
            exposed.dedup();
            let extra_rules: Vec<String> = allowed_ports
                .iter()
                .filter(|(p, proto)| !(proto == "tcp" && EXPECTED_PUBLIC_TCP.contains(p)))
                .map(|(p, proto)| format!("{p}/{proto}"))
                .collect();
            let listening_publicly = h.listeners.iter().filter(|l| is_public_addr(&l.addr)).count();
            if exposed.is_empty() {
                let c = b.push(
                    "open_ports",
                    "Only SSH and web ports are open",
                    Ok,
                    "Nothing but 22, 80 and 443 is reachable from the internet.",
                );
                if !extra_rules.is_empty() {
                    c.detail(format!(
                        "The firewall also allows {}, but nothing listens there.",
                        extra_rules.join(", ")
                    ));
                } else {
                    c.detail(format!(
                        "{} on public addresses; the firewall blocks everything else.",
                        plural(listening_publicly, "socket listens", "sockets listen")
                    ));
                }
            } else {
                b.push(
                    "open_ports",
                    "Only SSH and web ports are open",
                    Warn,
                    format!("Also reachable: {}.", exposed.join(", ")),
                )
                .fix(if ufw_on {
                    "Remove the ufw allow rule (ufw status numbered; ufw delete N) or stop the service."
                } else {
                    "Turn the firewall on, or stop the service, or bind it to 127.0.0.1."
                });
            }
        }
    }

    match &f.published_ports {
        None => {
            b.push("container_ports", "Containers publish only 80 and 443", Unknown, "Couldn't list containers from Docker.");
        }
        Some(ports) => {
            let mut public: Vec<String> = ports
                .iter()
                .filter(|p| p.host_ip.is_empty() || is_public_addr(&p.host_ip))
                .filter(|p| !(p.container == "caddy-container" && (p.host_port == 80 || p.host_port == 443)))
                .map(|p| format!("{} → {}/{}", p.container, p.host_port, p.proto))
                .collect();
            public.sort();
            public.dedup();
            if public.is_empty() {
                b.push(
                    "container_ports",
                    "Containers publish only 80 and 443",
                    Ok,
                    "Only Caddy publishes ports; apps are reached through it.",
                )
                .detail("Docker's port forwarding skips the firewall, so this is checked separately.");
            } else {
                b.push(
                    "container_ports",
                    "Containers publish only 80 and 443",
                    Warn,
                    format!("Published to the internet, past the firewall: {}.", public.join(", ")),
                )
                .detail("Docker's port forwarding is evaluated before ufw, so these are open even though ufw doesn't allow them.")
                .fix("Publish to 127.0.0.1 only, or drop the port; apps should be reached through Caddy.");
            }
        }
    }

    {
        let wan_published = f.published_ports.as_ref().is_some_and(|ps| {
            ps.iter()
                .any(|p| p.host_port == 2019 && (p.host_ip.is_empty() || is_public_addr(&p.host_ip)))
        });
        let guard = host.and_then(|h| h.caddy_admin_guard);
        let open_to_network = f
            .caddy_admin_listen
            .as_deref()
            .is_some_and(|l| l.starts_with("0.0.0.0") || l.starts_with(':') || l.starts_with("[::]"));
        let guard_note = match guard {
            Some(true) => "The DOCKER-USER firewall guard for port 2019 is in place.",
            Some(false) => "The DOCKER-USER firewall guard for port 2019 is missing (lh upgrade re-adds it).",
            None => "Couldn't read the DOCKER-USER firewall guard.",
        };
        let title = "Caddy's admin API is private";
        if wan_published {
            b.push("caddy_admin", title, Fail, "Port 2019 is published to the internet: anyone can rewrite routing.")
                .detail(guard_note)
                .fix("Recreate caddy-container without -p 2019 (lh upgrade does).");
        } else if open_to_network && !f.apps_on_caddy_network.is_empty() {
            b.push(
                "caddy_admin",
                title,
                Warn,
                format!(
                    "Not on the internet, but {} share Caddy's network and can reach its unauthenticated admin API.",
                    plural(f.apps_on_caddy_network.len(), "app", "apps")
                ),
            )
            .detail(format!(
                "An app could reroute or take over other apps' domains through caddy-container:2019. {guard_note} Apps on the network: {}.",
                f.apps_on_caddy_network.join(", ")
            ))
            .fix("Known gap, not fixed yet: Caddy's admin API needs to move off the network apps share.");
        } else {
            b.push("caddy_admin", title, Ok, "Not published, and no apps share its network.")
                .detail(guard_note);
        }
    }

    // ---- Access -----------------------------------------------------
    b.group("Access");

    match host.map(|h| h.sshd.as_ref()) {
        None => {
            unknown(&mut b, "ssh_passwords", "SSH password login is off");
            unknown(&mut b, "ssh_root", "Root can't log in with a password");
        }
        Some(None) => {
            b.push("ssh_passwords", "SSH password login is off", Unknown, "Couldn't read the sshd config.");
            b.push("ssh_root", "Root can't log in with a password", Unknown, "Couldn't read the sshd config.");
        }
        Some(Some(s)) => {
            let pw = s.password_authentication.as_deref() == Some("yes");
            let kbd = s.kbd_interactive_authentication.as_deref() == Some("yes");
            if pw || kbd {
                b.push(
                    "ssh_passwords",
                    "SSH password login is off",
                    Fail,
                    if pw { "PasswordAuthentication is yes." } else { "KbdInteractiveAuthentication is yes (prompts for passwords)." },
                )
                .detail("Passwords can be brute-forced; keys can't.")
                .fix("Set PasswordAuthentication no and KbdInteractiveAuthentication no in /etc/ssh/sshd_config.d/, then systemctl reload ssh.");
            } else {
                b.push("ssh_passwords", "SSH password login is off", Ok, "Only SSH keys are accepted.");
            }
            let root = s.permit_root_login.as_deref().unwrap_or("unknown");
            match root {
                "no" => {
                    b.push("ssh_root", "Root can't log in with a password", Ok, "Root login is disabled.");
                }
                "prohibit-password" | "without-password" | "forced-commands-only" => {
                    b.push("ssh_root", "Root can't log in with a password", Ok, "Root can log in with a key only.");
                }
                _ if !pw && !kbd => {
                    b.push(
                        "ssh_root",
                        "Root can't log in with a password",
                        Ok,
                        format!("PermitRootLogin is {root}, but password login is off everywhere."),
                    )
                    .detail("Setting PermitRootLogin prohibit-password keeps this safe if password login is ever turned back on.");
                }
                _ => {
                    b.push("ssh_root", "Root can't log in with a password", Fail, format!("PermitRootLogin is {root}."))
                        .fix("Set PermitRootLogin prohibit-password in /etc/ssh/sshd_config.d/, then systemctl reload ssh.");
                }
            }
        }
    }

    match host.and_then(|h| h.fail2ban_active) {
        None => unknown(&mut b, "fail2ban", "fail2ban is running"),
        Some(true) => {
            b.push("fail2ban", "fail2ban is running", Ok, "Repeated failed SSH logins get banned.");
        }
        Some(false) => {
            b.push("fail2ban", "fail2ban is running", Warn, "fail2ban isn't running.")
                .fix("apt install fail2ban && systemctl enable --now fail2ban");
        }
    }

    // ---- Updates ----------------------------------------------------
    b.group("Updates");

    match host.map(|h| h.os_updates.as_ref()) {
        None | Some(None) => unknown(&mut b, "auto_updates", "Automatic security updates are on"),
        Some(Some(u)) if u.enabled => {
            b.push("auto_updates", "Automatic security updates are on", Ok, "unattended-upgrades installs security patches nightly at 2am Eastern.")
                .detail(match &u.last_run_log {
                    Some(l) => format!("Last log line: {l}"),
                    None => "No run logged yet.".to_string(),
                });
        }
        Some(Some(_)) => {
            b.push("auto_updates", "Automatic security updates are on", Fail, "unattended-upgrades isn't fully enabled.")
                .fix("lh os-updates enable (as root on the server)");
        }
    }

    match host.map(|h| h.upgradable.as_ref()) {
        None => unknown(&mut b, "security_updates", "Packages are up to date"),
        Some(None) => {
            b.push("security_updates", "Packages are up to date", Unknown, "Couldn't list upgradable packages.");
        }
        Some(Some(u)) if u.security.is_empty() => {
            let c = b.push(
                "security_updates",
                "Packages are up to date",
                Ok,
                "No security updates are waiting.",
            );
            if u.total > 0 {
                c.detail(format!(
                    "{} non-security {} available; those aren't installed automatically.",
                    u.total,
                    if u.total == 1 { "update is" } else { "updates are" }
                ));
            }
        }
        Some(Some(u)) => {
            let auto = host.and_then(|h| h.os_updates.as_ref()).is_some_and(|o| o.enabled);
            b.push(
                "security_updates",
                "Packages are up to date",
                Warn,
                format!("{} waiting.", plural(u.security.len(), "security update", "security updates")),
            )
            .detail(format!("{}{}", u.security.join(", "), if auto { ". The next nightly run should install them." } else { "" }))
            .fix("apt-get update && apt-get upgrade (or wait for the 2am run)");
        }
    }

    match host.map(|h| h.os_updates.as_ref()) {
        None | Some(None) => unknown(&mut b, "reboot", "No reboot pending"),
        Some(Some(u)) if !u.reboot_required => {
            b.push("reboot", "No reboot pending", Ok, "The running kernel and libraries are current.");
        }
        Some(Some(u)) => {
            let next = u
                .weekly_reboot_next_run
                .as_deref()
                .map(|n| format!(" The weekly reboot is scheduled for {n}."))
                .unwrap_or_else(|| " No weekly reboot is scheduled.".to_string());
            let c = b.push(
                "reboot",
                "No reboot pending",
                Warn,
                format!("A reboot is needed to finish installing updates.{next}"),
            );
            if !u.reboot_required_packages.is_empty() {
                c.detail(format!("Waiting on: {}", u.reboot_required_packages.join(", ")));
            }
            c.fix("Reboot the server (apps come back on their own), or wait for the Sunday 3am Eastern reboot.");
        }
    }

    // ---- System -----------------------------------------------------
    b.group("System");

    match host.map(|h| h.disk.as_ref()) {
        None | Some(None) => unknown(&mut b, "disk", "Disk has room"),
        Some(Some(d)) => {
            let used_pct = if d.total_bytes == 0 {
                0.0
            } else {
                100.0 * (1.0 - d.available_bytes as f64 / d.total_bytes as f64)
            };
            let summary = format!("{:.0}% used, {} free of {}.", used_pct, gib(d.available_bytes), gib(d.total_bytes));
            let status = if used_pct >= 90.0 { Fail } else if used_pct >= 80.0 { Warn } else { Ok };
            let c = b.push("disk", "Disk has room", status, summary);
            if status != Ok {
                c.fix("docker image prune -a frees old app images; check /var/log and app /data volumes.");
            }
        }
    }

    match host.map(|h| h.memory.as_ref()) {
        None | Some(None) => unknown(&mut b, "memory", "Memory is available"),
        Some(Some(m)) => {
            let avail_pct = if m.total_bytes == 0 { 100.0 } else { 100.0 * m.available_bytes as f64 / m.total_bytes as f64 };
            let summary = format!("{} available of {} ({:.0}%).", gib(m.available_bytes), gib(m.total_bytes), avail_pct);
            let status = if avail_pct < 5.0 { Fail } else if avail_pct < 10.0 { Warn } else { Ok };
            let c = b.push("memory", "Memory is available", status, summary);
            if status != Ok {
                c.fix("Check per-app memory on each app's Overview tab; stop or resize the heaviest.");
            }
        }
    }

    match host.and_then(|h| h.clock_synced) {
        None => unknown(&mut b, "clock", "Clock is synced"),
        Some(true) => {
            b.push("clock", "Clock is synced", Ok, "NTP is keeping time; TLS and S3 signing depend on it.");
        }
        Some(false) => {
            b.push("clock", "Clock is synced", Warn, "The system clock isn't NTP-synchronized.")
                .detail("A drifting clock breaks TLS validation and S3 request signing.")
                .fix("timedatectl set-ntp true");
        }
    }

    if f.served_hosts.is_empty() {
        b.push("tls", "TLS certificates are valid", Ok, "No domain configured, so there's nothing to certify.");
    } else if let Some(h) = host {
        let mut expired = Vec::new();
        let mut expiring = Vec::new();
        let mut missing = Vec::new();
        let mut soonest: Option<(String, i64)> = None;
        for served in &f.served_hosts {
            let wildcard = served.split_once('.').map(|(_, rest)| format!("wildcard_.{rest}"));
            let cert = h
                .certificates
                .iter()
                .filter(|c| c.host == *served || Some(&c.host) == wildcard.as_ref())
                .max_by_key(|c| c.not_after);
            match cert {
                None => missing.push(served.clone()),
                Some(c) => {
                    let days = (c.not_after - f.now).num_days();
                    if c.not_after <= f.now {
                        expired.push(served.clone());
                    } else if days < 14 {
                        expiring.push(format!("{served} ({days}d)"));
                    }
                    if soonest.as_ref().is_none_or(|(_, d)| days < *d) {
                        soonest = Some((served.clone(), days));
                    }
                }
            }
        }
        let title = "TLS certificates are valid";
        let n = f.served_hosts.len();
        if !expired.is_empty() {
            b.push("tls", title, Fail, format!("Expired: {}.", expired.join(", ")))
                .fix("docker logs caddy-container | grep -i acme — renewal is failing (DNS, rate limits, or port 80 blocked).");
        } else if !expiring.is_empty() || !missing.is_empty() {
            let mut parts = Vec::new();
            if !expiring.is_empty() {
                parts.push(format!("expiring soon: {}", expiring.join(", ")));
            }
            if !missing.is_empty() {
                parts.push(format!("no certificate yet: {}", missing.join(", ")));
            }
            let mut s = parts.join("; ");
            s.replace_range(..1, &s[..1].to_uppercase());
            b.push("tls", title, Warn, format!("{s}."))
                .detail("Caddy renews 30 days before expiry, so under 14 days means renewal is failing. A missing certificate is normal for a minute after a new app or domain is added.")
                .fix("docker logs caddy-container | grep -i acme");
        } else {
            let c = b.push("tls", title, Ok, format!("All {} valid.", plural(n, "hostname is", "hostnames are")).replace("All 1 hostname is valid", "The hostname is valid"));
            if let Some((host, days)) = soonest {
                c.detail(format!("Next to renew: {host}, {days} days left. Caddy renews automatically."));
            }
        }
    } else {
        unknown(&mut b, "tls", "TLS certificates are valid");
    }

    // ---- Data & apps --------------------------------------------------
    b.group("Data & apps");

    {
        let title = "Backups are recent";
        if !f.s3_configured {
            b.push("backups", title, Fail, "S3 isn't configured, so nothing is backed up.")
                .fix("lh config s3 set (or the Settings tab).");
        } else {
            let age = f
                .last_backup_date
                .as_deref()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .map(|d| (f.now.date_naive() - d).num_days());
            match age {
                None => {
                    b.push("backups", title, Warn, "No clean backup day yet.")
                        .fix("lh backup run (or Run backup now on the Backups tab).");
                }
                Some(days) if days <= 1 => {
                    let c = b.push(
                        "backups",
                        title,
                        if f.last_backup_failures.is_empty() { Ok } else { Warn },
                        format!(
                            "Last clean day: {}.",
                            if days == 0 { "today".to_string() } else { "yesterday".to_string() }
                        ),
                    );
                    if !f.last_backup_failures.is_empty() {
                        c.detail(format!("The latest run failed for: {}.", f.last_backup_failures.join(", ")))
                            .fix("See the Backups tab for the errors, then lh backup run.");
                    }
                }
                Some(days) => {
                    let c = b.push("backups", title, Fail, format!("Last clean day was {days} days ago."));
                    if !f.last_backup_failures.is_empty() {
                        c.detail(format!("The latest run failed for: {}.", f.last_backup_failures.join(", ")));
                    }
                    c.fix("See the Backups tab for the errors, then lh backup run.");
                }
            }
        }
    }

    {
        let title = "Apps are running";
        let down: Vec<&str> = f
            .apps
            .iter()
            .filter(|a| matches!(a.desired.as_str(), "running" | "starting" | "crashed") && !a.running)
            .map(|a| a.name.as_str())
            .collect();
        let running = f.apps.iter().filter(|a| a.running).count();
        if f.apps.is_empty() {
            b.push("apps", title, Ok, "No apps yet.");
        } else if down.is_empty() {
            let stopped = f.apps.len() - running;
            let c = b.push("apps", title, Ok, format!("{running} of {} running.", f.apps.len()));
            if stopped > 0 {
                c.detail(format!("{} stopped on purpose.", plural(stopped, "app is", "apps are")));
            }
        } else {
            b.push("apps", title, Warn, format!("Should be running but aren't: {}.", down.join(", ")))
                .fix("Open the app and check its Logs tab, then restart it.");
        }
    }

    match (&f.host, &f.host_error) {
        (Some(h), _) => {
            let age = (f.now - h.generated_at).num_minutes();
            if age <= HOST_REPORT_STALE_MINUTES {
                b.push("host_report", "Host checks are current", Ok, format!("Host checked {} ago.", if age < 1 { "under a minute".to_string() } else { plural(age as usize, "minute", "minutes") }))
                    .detail("A systemd timer on the host refreshes these every 15 minutes.");
            } else {
                b.push("host_report", "Host checks are current", Warn, format!("The host report is {} old.", plural((age / 60).max(1) as usize, "hour", "hours")))
                    .fix("systemctl status litehouse-host-health.timer on the server; lh upgrade reinstalls it.");
            }
        }
        (None, err) => {
            let c = b.push("host_report", "Host checks are current", Unknown, "The host hasn't reported yet.");
            c.detail(
                err.clone()
                    .unwrap_or_else(|| "Host checks run from a systemd timer that lh install and lh upgrade set up.".into()),
            )
            .fix("Run lh upgrade on the server (installs the 15-minute host check timer).");
        }
    }

    b.checks
}

pub fn summarize(checks: &[Check]) -> HealthSummary {
    let mut s = HealthSummary::default();
    for c in checks {
        match c.status {
            CheckStatus::Ok => s.ok += 1,
            CheckStatus::Warn => s.warn += 1,
            CheckStatus::Fail => s.fail += 1,
            CheckStatus::Unknown => s.unknown += 1,
        }
    }
    s
}

pub fn build_report(f: &ServerFacts) -> HealthReport {
    let checks = evaluate(f);
    let host = f.host.as_ref();
    HealthReport {
        checked_at: f.now,
        host_checked_at: host.map(|h| h.generated_at),
        hostname: host.and_then(|h| h.hostname.clone()),
        os: host.and_then(|h| h.os.clone()),
        kernel: host.and_then(|h| h.kernel.clone()),
        uptime_secs: host.and_then(|h| h.uptime_secs),
        summary: summarize(&checks),
        checks,
    }
}

/// Read the host report the timer wrote.
pub fn read_host_report() -> (Option<HostReport>, Option<String>) {
    let path = host_report_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => match serde_json::from_str::<HostReport>(&s) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(format!("{path} couldn't be parsed: {e}"))),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(e) => (None, Some(format!("{path} couldn't be read: {e}"))),
    }
}

/// Docker-side facts: published ports, Caddy's admin listen address, and
/// which app containers share Caddy's network.
pub async fn docker_facts(
    docker: &bollard::Docker,
    app_names: &[String],
) -> (Option<Vec<PublishedPort>>, Option<String>, Vec<String>) {
    let published = docker
        .list_containers::<String>(Some(bollard::container::ListContainersOptions {
            all: false,
            ..Default::default()
        }))
        .await
        .ok()
        .map(|cs| {
            cs.into_iter()
                .flat_map(|c| {
                    let name = c
                        .names
                        .and_then(|n| n.into_iter().next())
                        .unwrap_or_default()
                        .trim_start_matches('/')
                        .to_string();
                    c.ports.unwrap_or_default().into_iter().filter_map(move |p| {
                        Some(PublishedPort {
                            container: name.clone(),
                            host_ip: p.ip.unwrap_or_default(),
                            host_port: p.public_port?,
                            proto: p.typ.map(|t| t.to_string()).unwrap_or_else(|| "tcp".into()),
                        })
                    })
                })
                .collect()
        });

    let mut caddy_admin_listen = None;
    let mut caddy_networks = Vec::new();
    if let Ok(c) = docker.inspect_container("caddy-container", None).await {
        caddy_admin_listen = c
            .config
            .and_then(|cfg| cfg.env)
            .unwrap_or_default()
            .into_iter()
            .find_map(|e| e.strip_prefix("CADDY_ADMIN=").map(str::to_string))
            // Caddy's own default is localhost:2019, reachable only inside
            // its container.
            .or(Some("localhost:2019".into()));
        caddy_networks = c
            .network_settings
            .and_then(|n| n.networks)
            .map(|n| n.into_keys().collect())
            .unwrap_or_default();
    }

    let mut on_network = BTreeSet::new();
    for net in &caddy_networks {
        if let Ok(n) = docker
            .inspect_network::<String>(net, None)
            .await
        {
            for (_, c) in n.containers.unwrap_or_default() {
                let Some(name) = c.name else { continue };
                if let Some(app) = name.strip_suffix("-container")
                    && app_names.iter().any(|a| a == app)
                {
                    on_network.insert(app.to_string());
                }
            }
        }
    }

    (published, caddy_admin_listen, on_network.into_iter().collect())
}

/// Render the checklist for `lh server-health`.
pub fn print_report(r: &HealthReport) {
    let mark = |s: CheckStatus| match s {
        CheckStatus::Ok => "✓",
        CheckStatus::Warn => "!",
        CheckStatus::Fail => "✗",
        CheckStatus::Unknown => "?",
    };
    println!(
        "{} ok, {} warning{}, {} failing, {} unknown",
        r.summary.ok,
        r.summary.warn,
        if r.summary.warn == 1 { "" } else { "s" },
        r.summary.fail,
        r.summary.unknown
    );
    let mut group = "";
    for c in &r.checks {
        if c.group != group {
            group = &c.group;
            println!("\n{group}");
        }
        println!("  {} {}: {}", mark(c.status), c.title, c.summary);
        if c.status != CheckStatus::Ok {
            if let Some(d) = &c.detail {
                println!("      {d}");
            }
            if let Some(fix) = &c.fix {
                println!("      fix: {fix}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UFW: &str = "Status: active
Logging: on (low)
Default: deny (incoming), allow (outgoing), deny (routed)
New profiles: skip

To                         Action      From
--                         ------      ----
22/tcp                     ALLOW IN    Anywhere
80/tcp                     ALLOW IN    Anywhere
443/tcp                    ALLOW IN    Anywhere
22/tcp (v6)                ALLOW IN    Anywhere (v6)
80/tcp (v6)                ALLOW IN    Anywhere (v6)
443/tcp (v6)               ALLOW IN    Anywhere (v6)
";

    const SS: &str = r#"udp   UNCONN 0      0         127.0.0.53%lo:53         0.0.0.0:*    users:(("systemd-resolve",pid=500,fd=13))
udp   UNCONN 0      0      10.17.0.5%eth0:68          0.0.0.0:*    users:(("systemd-network",pid=480,fd=19))
tcp   LISTEN 0      4096         0.0.0.0:443          0.0.0.0:*    users:(("docker-proxy",pid=1201,fd=4))
tcp   LISTEN 0      4096       127.0.0.1:3030         0.0.0.0:*    users:(("docker-proxy",pid=1300,fd=4))
tcp   LISTEN 0      4096               *:22                 *:*    users:(("sshd",pid=1,fd=3),("systemd",pid=1,fd=50))
tcp   LISTEN 0      4096            [::]:443             [::]:*    users:(("docker-proxy",pid=1210,fd=4))
tcp   LISTEN 0      511          0.0.0.0:6379         0.0.0.0:*    users:(("redis-server",pid=99,fd=6))
"#;

    fn os_updates(enabled: bool, reboot: bool) -> OsUpdatesStatus {
        OsUpdatesStatus {
            enabled,
            package_installed: enabled,
            periodic_enabled: true,
            update_lists: enabled,
            unattended_upgrade: enabled,
            automatic_reboot: Some("false".into()),
            timer_enabled: enabled,
            timer_next_run: None,
            last_run_log: None,
            reboot_required: reboot,
            reboot_required_packages: if reboot { vec!["linux-image-6.8.0-50-generic".into()] } else { vec![] },
            weekly_reboot_next_run: Some("Sun 2026-11-29 03:00:00 EST".into()),
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-11-22T12:00:00Z".parse().unwrap()
    }

    fn healthy_host() -> HostReport {
        HostReport {
            generated_at: now() - chrono::Duration::minutes(5),
            lh_version: "x".into(),
            hostname: Some("litehouse-1".into()),
            ufw: Some(parse_ufw_status(UFW)),
            listeners: parse_ss(SS).into_iter().filter(|l| l.port != 6379).collect(),
            caddy_admin_guard: Some(true),
            sshd: Some(SshdSettings {
                permit_root_login: Some("prohibit-password".into()),
                password_authentication: Some("no".into()),
                kbd_interactive_authentication: Some("no".into()),
            }),
            fail2ban_active: Some(true),
            os_updates: Some(os_updates(true, false)),
            upgradable: Some(Upgradable { total: 2, security: vec![] }),
            disk: Some(DiskUsage { total_bytes: 50_000_000_000, available_bytes: 30_000_000_000 }),
            memory: Some(MemoryUsage { total_bytes: 2_000_000_000, available_bytes: 1_000_000_000 }),
            clock_synced: Some(true),
            certificates: vec![
                CertInfo { host: "admin.lh.example.com".into(), not_after: now() + chrono::Duration::days(60) },
                CertInfo { host: "hello.lh.example.com".into(), not_after: now() + chrono::Duration::days(45) },
            ],
            ..Default::default()
        }
    }

    fn healthy() -> ServerFacts {
        ServerFacts {
            now: now(),
            host: Some(healthy_host()),
            host_error: None,
            served_hosts: vec!["admin.lh.example.com".into(), "hello.lh.example.com".into()],
            s3_configured: true,
            last_backup_date: Some("2026-11-22".into()),
            last_backup_failures: vec![],
            apps: vec![AppFacts { name: "hello".into(), desired: "running".into(), running: true }],
            published_ports: Some(vec![
                PublishedPort { container: "caddy-container".into(), host_ip: "0.0.0.0".into(), host_port: 443, proto: "tcp".into() },
                PublishedPort { container: "caddy-container".into(), host_ip: "0.0.0.0".into(), host_port: 80, proto: "tcp".into() },
                PublishedPort { container: "litehouse-server".into(), host_ip: "127.0.0.1".into(), host_port: 3030, proto: "tcp".into() },
            ]),
            caddy_admin_listen: Some("0.0.0.0:2019".into()),
            apps_on_caddy_network: vec![],
        }
    }

    fn status(checks: &[Check], id: &str) -> CheckStatus {
        checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check {id}")).status
    }

    #[test]
    fn parses_ufw() {
        let u = parse_ufw_status(UFW);
        assert!(u.active);
        assert_eq!(u.default_incoming.as_deref(), Some("deny"));
        assert_eq!(u.allowed, vec!["22/tcp", "443/tcp", "80/tcp"]);
        assert!(!parse_ufw_status("Status: inactive\n").active);
    }

    #[test]
    fn parses_ss() {
        let l = parse_ss(SS);
        assert_eq!(l.len(), 7);
        assert_eq!(l[0].addr, "127.0.0.53");
        assert_eq!(l[1].addr, "10.17.0.5");
        assert_eq!(l[4].addr, "*");
        assert_eq!(l[4].process.as_deref(), Some("sshd"));
        assert_eq!(l[5].addr, "::");
        assert!(!is_public_addr("127.0.0.53"));
        assert!(!is_public_addr("::1"));
        assert!(!is_public_addr("172.18.0.1"));
        assert!(is_public_addr("172.32.0.1"));
        assert!(is_public_addr("*"));
        assert!(is_public_addr("::"));
    }

    #[test]
    fn parses_sshd_apt_df_meminfo_openssl() {
        let s = parse_sshd_t("port 22\npermitrootlogin without-password\npasswordauthentication no\nkbdinteractiveauthentication no\n");
        assert_eq!(s.permit_root_login.as_deref(), Some("without-password"));
        assert_eq!(s.password_authentication.as_deref(), Some("no"));

        let apt = "Listing...\nopenssl/noble-updates,noble-security 3.0.13-0ubuntu3.5 amd64 [upgradable from: 3.0.13-0ubuntu3.4]\nvim/noble-updates 2:9.1 amd64 [upgradable from: 2:9.0]\n";
        assert_eq!(parse_apt_upgradable(apt), Upgradable { total: 2, security: vec!["openssl".into()] });

        let df = "Filesystem     1-blocks       Used   Available Capacity Mounted on\n/dev/vda1   51835101184 9000000000 42835101184      18% /\n";
        assert_eq!(parse_df(df), Some(DiskUsage { total_bytes: 51835101184, available_bytes: 42835101184 }));

        let mem = "MemTotal:        2014000 kB\nMemFree:          100000 kB\nMemAvailable:    1000000 kB\n";
        assert_eq!(parse_meminfo(mem), Some(MemoryUsage { total_bytes: 2014000 * 1024, available_bytes: 1000000 * 1024 }));

        assert_eq!(
            parse_openssl_enddate("notAfter=Feb  2 12:00:00 2027 GMT\n"),
            Some("2027-02-02T12:00:00Z".parse().unwrap())
        );
    }

    #[test]
    fn healthy_server_is_all_green() {
        let checks = evaluate(&healthy());
        let bad: Vec<_> = checks.iter().filter(|c| c.status != CheckStatus::Ok).collect();
        assert!(bad.is_empty(), "{bad:#?}");
        // Every check has a distinct id.
        let ids: BTreeSet<_> = checks.iter().map(|c| &c.id).collect();
        assert_eq!(ids.len(), checks.len());
    }

    #[test]
    fn no_host_report_marks_host_checks_unknown_but_keeps_server_checks() {
        let mut f = healthy();
        f.host = None;
        let checks = evaluate(&f);
        assert_eq!(status(&checks, "firewall"), CheckStatus::Unknown);
        assert_eq!(status(&checks, "ssh_passwords"), CheckStatus::Unknown);
        assert_eq!(status(&checks, "host_report"), CheckStatus::Unknown);
        assert_eq!(status(&checks, "backups"), CheckStatus::Ok);
        assert_eq!(status(&checks, "container_ports"), CheckStatus::Ok);
    }

    #[test]
    fn exposed_service_and_published_port_warn() {
        let mut f = healthy();
        // redis on 0.0.0.0 with ufw off: reachable.
        let host = f.host.as_mut().unwrap();
        host.listeners = parse_ss(SS);
        host.ufw.as_mut().unwrap().active = false;
        f.published_ports.as_mut().unwrap().push(PublishedPort {
            container: "hello-container".into(),
            host_ip: "0.0.0.0".into(),
            host_port: 8080,
            proto: "tcp".into(),
        });
        let checks = evaluate(&f);
        assert_eq!(status(&checks, "firewall"), CheckStatus::Fail);
        let open = checks.iter().find(|c| c.id == "open_ports").unwrap();
        assert_eq!(open.status, CheckStatus::Warn);
        assert!(open.summary.contains("6379/tcp (redis-server)"), "{}", open.summary);
        // UDP 68 on the private interface also counts with the firewall off.
        assert!(open.summary.contains("68/udp"));
        assert_eq!(status(&checks, "container_ports"), CheckStatus::Warn);
    }

    #[test]
    fn firewall_hides_unlisted_listeners() {
        let mut f = healthy();
        f.host.as_mut().unwrap().listeners = parse_ss(SS);
        assert_eq!(status(&evaluate(&f), "open_ports"), CheckStatus::Ok);
    }

    #[test]
    fn caddy_admin_shared_with_apps_warns_and_published_fails() {
        let mut f = healthy();
        f.apps_on_caddy_network = vec!["hello".into()];
        assert_eq!(status(&evaluate(&f), "caddy_admin"), CheckStatus::Warn);
        f.caddy_admin_listen = Some("localhost:2019".into());
        assert_eq!(status(&evaluate(&f), "caddy_admin"), CheckStatus::Ok);
        f.published_ports.as_mut().unwrap().push(PublishedPort {
            container: "caddy-container".into(),
            host_ip: "0.0.0.0".into(),
            host_port: 2019,
            proto: "tcp".into(),
        });
        assert_eq!(status(&evaluate(&f), "caddy_admin"), CheckStatus::Fail);
    }

    #[test]
    fn ssh_password_and_root_login() {
        let mut f = healthy();
        let s = f.host.as_mut().unwrap().sshd.as_mut().unwrap();
        s.permit_root_login = Some("yes".into());
        // Root "yes" is fine while passwords are off everywhere.
        assert_eq!(status(&evaluate(&f), "ssh_root"), CheckStatus::Ok);
        f.host.as_mut().unwrap().sshd.as_mut().unwrap().password_authentication = Some("yes".into());
        let checks = evaluate(&f);
        assert_eq!(status(&checks, "ssh_passwords"), CheckStatus::Fail);
        assert_eq!(status(&checks, "ssh_root"), CheckStatus::Fail);
    }

    #[test]
    fn updates_and_reboot() {
        let mut f = healthy();
        let h = f.host.as_mut().unwrap();
        h.os_updates = Some(os_updates(false, true));
        h.upgradable = Some(Upgradable { total: 3, security: vec!["openssl".into()] });
        let checks = evaluate(&f);
        assert_eq!(status(&checks, "auto_updates"), CheckStatus::Fail);
        assert_eq!(status(&checks, "security_updates"), CheckStatus::Warn);
        assert_eq!(status(&checks, "reboot"), CheckStatus::Warn);
    }

    #[test]
    fn disk_memory_thresholds() {
        let mut f = healthy();
        f.host.as_mut().unwrap().disk = Some(DiskUsage { total_bytes: 100, available_bytes: 15 });
        assert_eq!(status(&evaluate(&f), "disk"), CheckStatus::Warn);
        f.host.as_mut().unwrap().disk = Some(DiskUsage { total_bytes: 100, available_bytes: 5 });
        assert_eq!(status(&evaluate(&f), "disk"), CheckStatus::Fail);
        f.host.as_mut().unwrap().memory = Some(MemoryUsage { total_bytes: 100, available_bytes: 8 });
        assert_eq!(status(&evaluate(&f), "memory"), CheckStatus::Warn);
    }

    #[test]
    fn tls_expiring_missing_and_wildcard() {
        let mut f = healthy();
        f.served_hosts.push("new.lh.example.com".into());
        assert_eq!(status(&evaluate(&f), "tls"), CheckStatus::Warn);
        f.host.as_mut().unwrap().certificates.push(CertInfo {
            host: "wildcard_.lh.example.com".into(),
            not_after: now() + chrono::Duration::days(80),
        });
        assert_eq!(status(&evaluate(&f), "tls"), CheckStatus::Ok);
        f.host.as_mut().unwrap().certificates[0].not_after = now() - chrono::Duration::days(1);
        // admin has its own (expired) cert, but the wildcard covers it too.
        assert_eq!(status(&evaluate(&f), "tls"), CheckStatus::Ok);
        f.host.as_mut().unwrap().certificates.pop();
        assert_eq!(status(&evaluate(&f), "tls"), CheckStatus::Fail);
    }

    #[test]
    fn backups_age() {
        let mut f = healthy();
        f.last_backup_date = Some("2026-11-21".into());
        assert_eq!(status(&evaluate(&f), "backups"), CheckStatus::Ok);
        f.last_backup_date = Some("2026-11-19".into());
        assert_eq!(status(&evaluate(&f), "backups"), CheckStatus::Fail);
        f.s3_configured = false;
        assert_eq!(status(&evaluate(&f), "backups"), CheckStatus::Fail);
    }

    #[test]
    fn crashed_app_and_stale_host_report_warn() {
        let mut f = healthy();
        f.apps.push(AppFacts { name: "web".into(), desired: "running".into(), running: false });
        f.apps.push(AppFacts { name: "old".into(), desired: "stopped".into(), running: false });
        f.host.as_mut().unwrap().generated_at = now() - chrono::Duration::hours(3);
        let checks = evaluate(&f);
        let apps = checks.iter().find(|c| c.id == "apps").unwrap();
        assert_eq!(apps.status, CheckStatus::Warn);
        assert!(apps.summary.contains("web") && !apps.summary.contains("old"));
        assert_eq!(status(&checks, "host_report"), CheckStatus::Warn);
    }

    #[test]
    fn host_report_roundtrips_json() {
        let h = healthy_host();
        let back: HostReport = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back.listeners, h.listeners);
    }
}
