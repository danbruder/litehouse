//! `lh os-updates status|enable`: check or turn on automatic OS security
//! updates (unattended-upgrades) on the litehouse host. Runs on the server
//! itself, like `lh install`/`lh upgrade`; it never talks to the API.
//! See docs/superpowers/specs/2026-11-21-os-security-updates-design.md.

use anyhow::Result;
use serde::Serialize;
use std::process::Command;

use crate::install::executor::is_root;
use crate::install::phases::phase14_os_security_updates;

#[derive(Debug, Serialize)]
pub struct OsUpdatesStatus {
    /// Automatic security updates are installed, switched on, and scheduled.
    pub enabled: bool,
    pub package_installed: bool,
    /// `APT::Periodic::Enable` isn't "0" (unset means on).
    pub periodic_enabled: bool,
    /// `APT::Periodic::Update-Package-Lists` is non-zero.
    pub update_lists: bool,
    /// `APT::Periodic::Unattended-Upgrade` is non-zero.
    pub unattended_upgrade: bool,
    pub automatic_reboot: Option<String>,
    pub timer_enabled: bool,
    pub timer_next_run: Option<String>,
    /// Last line of /var/log/unattended-upgrades/unattended-upgrades.log.
    pub last_run_log: Option<String>,
    pub reboot_required: bool,
    pub reboot_required_packages: Vec<String>,
    pub weekly_reboot_next_run: Option<String>,
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Value of `key` in `apt-config dump` output (`Key "value";`).
fn apt_config_value(dump: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} ");
    dump.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim().trim_end_matches(';').trim_matches('"').to_string())
}

fn periodic_on(dump: &str, key: &str) -> bool {
    apt_config_value(dump, key)
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false)
}

fn timer_next_run(unit: &str) -> Option<String> {
    output(
        "systemctl",
        &["show", unit, "-p", "NextElapseUSecRealtime", "--value"],
    )
    .filter(|v| v != "n/a" && v != "0")
}

pub fn collect() -> OsUpdatesStatus {
    let package_installed = output("dpkg-query", &["-W", "-f=${Status}", "unattended-upgrades"])
        .map(|s| s == "install ok installed")
        .unwrap_or(false);
    let dump = output("apt-config", &["dump"]).unwrap_or_default();
    let periodic_enabled = apt_config_value(&dump, "APT::Periodic::Enable")
        .map(|v| v != "0")
        .unwrap_or(true);
    let update_lists = periodic_on(&dump, "APT::Periodic::Update-Package-Lists");
    let unattended_upgrade = periodic_on(&dump, "APT::Periodic::Unattended-Upgrade");
    let automatic_reboot = apt_config_value(&dump, "Unattended-Upgrade::Automatic-Reboot");
    let timer_enabled = output("systemctl", &["is-enabled", "apt-daily-upgrade.timer"])
        .map(|s| s == "enabled")
        .unwrap_or(false);
    let last_run_log = std::fs::read_to_string("/var/log/unattended-upgrades/unattended-upgrades.log")
        .ok()
        .and_then(|s| s.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string));
    let reboot_required = std::path::Path::new("/var/run/reboot-required").exists();
    let reboot_required_packages = std::fs::read_to_string("/var/run/reboot-required.pkgs")
        .map(|s| {
            let mut pkgs: Vec<String> = s.lines().map(str::to_string).collect();
            pkgs.sort();
            pkgs.dedup();
            pkgs
        })
        .unwrap_or_default();

    OsUpdatesStatus {
        enabled: package_installed && periodic_enabled && update_lists && unattended_upgrade && timer_enabled,
        package_installed,
        periodic_enabled,
        update_lists,
        unattended_upgrade,
        automatic_reboot,
        timer_enabled,
        timer_next_run: timer_next_run("apt-daily-upgrade.timer"),
        last_run_log,
        reboot_required,
        reboot_required_packages,
        weekly_reboot_next_run: timer_next_run("litehouse-weekly-reboot.timer"),
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn print_status(s: &OsUpdatesStatus) {
    println!(
        "Automatic OS security updates: {}",
        if s.enabled { "ENABLED" } else { "NOT ENABLED" }
    );
    println!("  unattended-upgrades installed: {}", yes_no(s.package_installed));
    if !s.periodic_enabled {
        println!("  APT::Periodic::Enable:         0 (all periodic apt jobs disabled)");
    }
    println!("  daily package list refresh:    {}", yes_no(s.update_lists));
    println!("  daily unattended upgrade:      {}", yes_no(s.unattended_upgrade));
    println!("  apt-daily-upgrade.timer:       {}", if s.timer_enabled { "enabled" } else { "not enabled" });
    println!("  next upgrade run:              {}", s.timer_next_run.as_deref().unwrap_or("-"));
    println!(
        "  automatic reboot:              {} (weekly reboot timer handles kernel updates)",
        s.automatic_reboot.as_deref().unwrap_or("unset")
    );
    println!("  next weekly reboot:            {}", s.weekly_reboot_next_run.as_deref().unwrap_or("-"));
    if s.reboot_required {
        let pkgs = if s.reboot_required_packages.is_empty() {
            String::new()
        } else {
            format!(" ({})", s.reboot_required_packages.join(", "))
        };
        println!("  reboot pending:                yes{pkgs}");
    } else {
        println!("  reboot pending:                no");
    }
    println!("  last log line:                 {}", s.last_run_log.as_deref().unwrap_or("-"));
    if !s.enabled {
        println!("\nRun `lh os-updates enable` as root to turn them on.");
    }
}

/// Print status; exits 1 (via error) when updates aren't enabled so it can
/// be used as a check in scripts.
pub fn status(json: bool) -> Result<()> {
    let s = collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
    } else {
        print_status(&s);
    }
    if !s.enabled {
        std::process::exit(1);
    }
    Ok(())
}

pub fn enable() -> Result<()> {
    if !is_root() {
        anyhow::bail!("This command must be run as root on the litehouse server");
    }
    phase14_os_security_updates(None)?;
    let s = collect();
    print_status(&s);
    if !s.enabled {
        anyhow::bail!("unattended-upgrades was configured but still isn't fully enabled (see above)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"APT "";
APT::Periodic "";
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "0";
Unattended-Upgrade::Automatic-Reboot "false";
"#;

    #[test]
    fn reads_apt_config_dump_values() {
        assert!(periodic_on(DUMP, "APT::Periodic::Update-Package-Lists"));
        assert!(!periodic_on(DUMP, "APT::Periodic::Unattended-Upgrade"));
        assert!(!periodic_on(DUMP, "APT::Periodic::Missing"));
        assert_eq!(
            apt_config_value(DUMP, "Unattended-Upgrade::Automatic-Reboot").as_deref(),
            Some("false")
        );
        // A key that's a prefix of another must not match it.
        assert_eq!(apt_config_value(DUMP, "APT::Periodic::Update"), None);
    }
}
