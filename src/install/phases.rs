use anyhow::{Context, Result};
use indicatif::ProgressBar;
use tracing::{info, instrument};

use super::executor::{run_command, run_command_with_log, sudo_write_file};
use super::templates;

/// Phase 1: Validation
#[instrument]
pub fn phase1_validation(domain: &str) -> Result<()> {
    info!("Phase 1: Validation");

    // Validate domain format (basic check)
    if domain.is_empty() || !domain.contains('.') {
        anyhow::bail!("Invalid domain format: {}", domain);
    }

    // Check if running as root
    if !super::executor::is_root() {
        anyhow::bail!("This command must be run as root");
    }

    // Detect OS
    let os_info = run_command("cat /etc/os-release | grep ^ID= | cut -d= -f2")?;
    let os_id = os_info.trim().trim_matches('"');

    if os_id != "ubuntu" && os_id != "debian" {
        anyhow::bail!(
            "Unsupported OS: {}. Only Ubuntu and Debian are supported.",
            os_id
        );
    }

    info!("Detected OS: {}", os_id);
    info!("Phase 1 completed successfully");
    Ok(())
}

/// Phase 2: System Preparation
#[instrument(skip(log_window))]
pub fn phase2_system_preparation(log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 2: System Preparation");

    let script = templates::system_preparation_script();

    // Write and execute the script
    std::fs::write("/tmp/system_prep.sh", script)?;
    run_command("chmod +x /tmp/system_prep.sh")?;
    run_command_with_log("/tmp/system_prep.sh", log_window)?;
    run_command("rm /tmp/system_prep.sh")?;

    info!("Phase 2 completed successfully");
    Ok(())
}

/// Phase 3: Security Hardening
#[instrument(skip(log_window))]
pub fn phase3_security_hardening(log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 3: Security Hardening");

    let script = templates::security_hardening_script();

    // Write and execute the script
    std::fs::write("/tmp/security_setup.sh", script)?;
    run_command("chmod +x /tmp/security_setup.sh")?;
    run_command_with_log("/tmp/security_setup.sh", log_window)?;
    run_command("rm /tmp/security_setup.sh")?;

    info!("Phase 3 completed successfully");
    Ok(())
}

/// Phase 4: User & Directory Setup
#[instrument(skip(log_window))]
pub fn phase4_user_setup(log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 4: User & Directory Setup");

    let script = templates::user_setup_script();

    // Write and execute the script
    std::fs::write("/tmp/user_setup.sh", script)?;
    run_command("chmod +x /tmp/user_setup.sh")?;
    run_command_with_log("/tmp/user_setup.sh", log_window)?;
    run_command("rm /tmp/user_setup.sh")?;

    info!("Phase 4 completed successfully");
    Ok(())
}

/// Phase 5: Podman Configuration
#[instrument(skip(log_window))]
pub fn phase5_podman_configuration(log_window: Option<&ProgressBar>) -> Result<String> {
    info!("Phase 5: Podman Configuration");

    let script = templates::podman_setup_script();

    // Write and execute the script
    std::fs::write("/tmp/podman_setup.sh", script)?;
    run_command("chmod +x /tmp/podman_setup.sh")?;
    let output = run_command_with_log("/tmp/podman_setup.sh", log_window)?;
    run_command("rm /tmp/podman_setup.sh")?;

    // Extract UID from output
    let uid_line = output
        .lines()
        .find(|line| line.starts_with("UID:"))
        .context("Failed to get litehouse user UID from podman setup output")?;

    let uid = uid_line
        .strip_prefix("UID:")
        .context("Failed to parse UID")?
        .trim()
        .to_string();

    info!("Litehouse user UID: {}", uid);
    info!("Phase 5 completed successfully");

    Ok(uid)
}

/// Get litehouse user UID
pub fn get_litehouse_uid() -> Result<String> {
    let output = run_command("id -u litehouse")?;
    Ok(output.trim().to_string())
}

/// Phase 6a: Build litehouse image locally
#[instrument(skip(log_window))]
pub fn phase6a_build_litehouse_image(litehouse_uid: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 6a: Build litehouse image locally");

    let script = templates::build_litehouse_image_script(litehouse_uid);
    std::fs::write("/tmp/build_litehouse.sh", &script)?;
    run_command("chmod +x /tmp/build_litehouse.sh")?;
    run_command_with_log("/tmp/build_litehouse.sh", log_window)?;
    run_command("rm /tmp/build_litehouse.sh")?;

    info!("Phase 6a completed successfully");
    Ok(())
}

/// Phase 6b: Pull caddy image
#[instrument(skip(log_window))]
pub fn phase6b_pull_caddy_image(litehouse_uid: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 6b: Pull caddy image");

    let script = templates::pull_caddy_image_script(litehouse_uid);
    std::fs::write("/tmp/pull_caddy.sh", &script)?;
    run_command("chmod +x /tmp/pull_caddy.sh")?;
    run_command_with_log("/tmp/pull_caddy.sh", log_window)?;
    run_command("rm /tmp/pull_caddy.sh")?;

    info!("Phase 6b completed successfully");
    Ok(())
}

/// Phase 6c: Pull litestream image
#[instrument(skip(log_window))]
pub fn phase6c_pull_litestream_image(litehouse_uid: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 6c: Pull litestream image");

    let script = templates::pull_litestream_image_script(litehouse_uid);
    std::fs::write("/tmp/pull_litestream.sh", &script)?;
    run_command("chmod +x /tmp/pull_litestream.sh")?;
    run_command_with_log("/tmp/pull_litestream.sh", log_window)?;
    run_command("rm /tmp/pull_litestream.sh")?;

    info!("Phase 6c completed successfully");
    Ok(())
}

/// Phase 7: Server Configuration
#[instrument]
pub fn phase7_server_configuration(domain: &str) -> Result<()> {
    info!("Phase 7: Server Configuration");

    // Write server config
    let config_content = templates::server_config_template(domain);
    sudo_write_file("/opt/litehouse/config/server-config.toml", &config_content)?;
    run_command("chown litehouse:litehouse /opt/litehouse/config/server-config.toml")?;

    // Create initial Litestream config
    info!("Creating initial Litestream configuration");
    let litestream_config = templates::initial_litestream_config();
    sudo_write_file("/opt/litehouse/data/litestream.yml", litestream_config)?;
    run_command("chown litehouse:litehouse /opt/litehouse/data/litestream.yml")?;

    // Create litestream replicas directory
    run_command("mkdir -p /opt/litehouse/data/litestream-replicas")?;
    run_command("chown litehouse:litehouse /opt/litehouse/data/litestream-replicas")?;

    info!("Phase 7 completed successfully");
    Ok(())
}

/// Phase 8: Log Rotation
#[instrument]
pub fn phase8_log_rotation() -> Result<()> {
    info!("Phase 8: Log Rotation");

    let logrotate_content = templates::logrotate_template();

    // Create log directory
    run_command("mkdir -p /var/log/litehouse")?;
    run_command("chown litehouse:litehouse /var/log/litehouse")?;

    // Write logrotate config
    sudo_write_file("/etc/logrotate.d/litehouse", logrotate_content)?;
    run_command("chmod 644 /etc/logrotate.d/litehouse")?;

    info!("Phase 8 completed successfully");
    Ok(())
}

/// Phase 9: Start litehouse-server Container
#[instrument]
pub fn phase9_start_litehouse_container(litehouse_uid: &str) -> Result<()> {
    info!("Phase 9: Start litehouse-server Container");

    let script = templates::start_litehouse_container_script(litehouse_uid);
    std::fs::write("/tmp/start_litehouse.sh", &script)?;
    run_command("chmod +x /tmp/start_litehouse.sh")?;
    run_command("/tmp/start_litehouse.sh")?;
    run_command("rm /tmp/start_litehouse.sh")?;

    // Wait a moment for container to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify container is running
    let ps_output = run_command(&format!(
        "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman ps --filter name=litehouse-server --format {{{{.Status}}}}'",
        litehouse_uid
    ))?;

    if !ps_output.contains("Up") {
        // Container is not running - get the logs to understand why
        let logs = run_command(&format!(
            "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman logs litehouse-server 2>&1 | tail -50'",
            litehouse_uid
        )).unwrap_or_else(|_| "Could not retrieve container logs".to_string());

        anyhow::bail!(
            "litehouse-server container is not running. Container logs:\n{}",
            logs
        );
    }

    info!("litehouse-server container started successfully");
    info!("Phase 9 completed successfully");
    Ok(())
}

/// Phase 10: Enable podman-restart Service
#[instrument]
pub fn phase10_enable_podman_restart(litehouse_uid: &str) -> Result<()> {
    info!("Phase 10: Enable podman-restart Service");

    let script = templates::enable_podman_restart_script(litehouse_uid);
    std::fs::write("/tmp/enable_restart.sh", &script)?;
    run_command("chmod +x /tmp/enable_restart.sh")?;
    run_command("/tmp/enable_restart.sh")?;
    run_command("rm /tmp/enable_restart.sh")?;

    info!("podman-restart.service enabled - containers will restart on boot");
    info!("Phase 10 completed successfully");
    Ok(())
}

/// Phase 11: Verification
#[instrument]
pub fn phase11_verification(domain: &str) -> Result<()> {
    info!("Phase 11: Verification");

    let api_url = format!("http://admin.{}/apps", domain);
    info!("Testing API endpoint: {}", api_url);

    // Retry with backoff
    let max_retries = 30;
    let mut retries = 0;
    let mut last_error = String::new();

    while retries < max_retries {
        let curl_command = format!("curl -s -o /dev/null -w '%{{http_code}}' {}", api_url);
        match run_command(&curl_command) {
            Ok(output) => {
                let status_code = output.trim();
                if status_code == "200" {
                    info!("API endpoint responding successfully!");
                    info!("Phase 11 completed successfully");
                    return Ok(());
                } else {
                    last_error = format!("HTTP {}", status_code);
                }
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }

        retries += 1;
        if retries < max_retries {
            info!(
                "API not ready yet, retrying... ({}/{})",
                retries, max_retries
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    anyhow::bail!(
        "Failed to verify API endpoint after {} attempts. Last error: {}",
        max_retries,
        last_error
    );
}
