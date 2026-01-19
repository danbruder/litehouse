use anyhow::{Context, Result};
use indicatif::ProgressBar;
use tracing::{info, instrument};

use super::ssh::{
    execute_remote, execute_remote_with_log, has_sudo_access, test_ssh_connection, upload_content,
};
use super::templates;

/// Phase 1: Validation & Connectivity
#[instrument]
pub fn phase1_validation(ssh_target: &str, domain: &str) -> Result<()> {
    info!("Phase 1: Validation & Connectivity");

    // Validate domain format (basic check)
    if domain.is_empty() || !domain.contains('.') {
        anyhow::bail!("Invalid domain format: {}", domain);
    }

    // Test SSH connectivity
    test_ssh_connection(ssh_target)
        .context("Failed to establish SSH connection. Please check your SSH keys and network.")?;

    // Check if SSH is available locally
    if !std::process::Command::new("ssh").arg("-V").output().is_ok() {
        anyhow::bail!("SSH command not found. Please install OpenSSH client.");
    }

    // Check if SCP is available locally
    if !std::process::Command::new("scp").arg("-h").output().is_ok() {
        anyhow::bail!("SCP command not found. Please install OpenSSH client.");
    }

    // Detect OS
    let os_info = execute_remote(ssh_target, "cat /etc/os-release | grep ^ID= | cut -d= -f2")?;
    let os_id = os_info.trim().trim_matches('"');

    if os_id != "ubuntu" && os_id != "debian" {
        anyhow::bail!(
            "Unsupported OS: {}. Only Ubuntu and Debian are supported.",
            os_id
        );
    }

    info!("Detected OS: {}", os_id);

    // Check sudo/root access
    let has_sudo = has_sudo_access(ssh_target)?;
    if !has_sudo {
        anyhow::bail!("User does not have sudo access on the remote server");
    }

    info!("Phase 1 completed successfully");
    Ok(())
}

/// Phase 2: System Preparation
#[instrument(skip(log_window))]
pub fn phase2_system_preparation(ssh_target: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 2: System Preparation");

    let script = templates::system_preparation_script();

    // Upload and execute the script
    upload_content(ssh_target, script, "/tmp/system_prep.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/system_prep.sh")?;
    execute_remote_with_log(ssh_target, "sudo /tmp/system_prep.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/system_prep.sh")?;

    info!("Phase 2 completed successfully");
    Ok(())
}

/// Phase 3: Security Hardening
#[instrument(skip(log_window))]
pub fn phase3_security_hardening(ssh_target: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 3: Security Hardening");

    let script = templates::security_hardening_script();

    // Upload and execute the script
    upload_content(ssh_target, script, "/tmp/security_setup.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/security_setup.sh")?;
    execute_remote_with_log(ssh_target, "sudo /tmp/security_setup.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/security_setup.sh")?;

    info!("Phase 3 completed successfully");
    Ok(())
}

/// Phase 4: User & Directory Setup
#[instrument(skip(log_window))]
pub fn phase4_user_setup(ssh_target: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 4: User & Directory Setup");

    let script = templates::user_setup_script();

    // Upload and execute the script
    upload_content(ssh_target, script, "/tmp/user_setup.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/user_setup.sh")?;
    execute_remote_with_log(ssh_target, "sudo /tmp/user_setup.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/user_setup.sh")?;

    info!("Phase 4 completed successfully");
    Ok(())
}

/// Phase 5: Docker Configuration
#[instrument(skip(log_window))]
pub fn phase5_docker_configuration(
    ssh_target: &str,
    log_window: Option<&ProgressBar>,
) -> Result<String> {
    info!("Phase 5: Docker Configuration");

    let script = templates::docker_setup_script();

    // Upload and execute the script
    upload_content(ssh_target, script, "/tmp/docker_setup.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/docker_setup.sh")?;
    let output = execute_remote_with_log(ssh_target, "sudo /tmp/docker_setup.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/docker_setup.sh")?;

    // Extract UID from output
    let uid_line = output
        .lines()
        .find(|line| line.starts_with("UID:"))
        .context("Failed to get litehouse user UID from docker setup output")?;

    let uid = uid_line
        .strip_prefix("UID:")
        .context("Failed to parse UID")?
        .trim()
        .to_string();

    info!("Litehouse user UID: {}", uid);
    info!("Phase 5 completed successfully");

    Ok(uid)
}

/// Phase 6: Container Image Pull (litehouse, caddy, litestream)
#[instrument(skip(log_window))]
pub fn phase6_container_image_pull(
    ssh_target: &str,
    log_window: Option<&ProgressBar>,
) -> Result<()> {
    info!("Phase 6: Container Image Pull");

    // Get litehouse UID from remote system
    let uid_output = execute_remote(ssh_target, "id -u litehouse")?;
    let litehouse_uid = uid_output.trim();

    // Pull litehouse server image
    let script = templates::pull_container_script(litehouse_uid);
    upload_content(ssh_target, &script, "/tmp/pull_container.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/pull_container.sh")?;
    execute_remote_with_log(ssh_target, "sudo /tmp/pull_container.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/pull_container.sh")?;

    // Pull Caddy image
    let caddy_script = templates::pull_caddy_image_script(litehouse_uid);
    upload_content(ssh_target, &caddy_script, "/tmp/pull_caddy.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/pull_caddy.sh")?;
    execute_remote_with_log(ssh_target, "sudo /tmp/pull_caddy.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/pull_caddy.sh")?;

    // Pull Litestream image
    let litestream_script = templates::pull_litestream_image_script(litehouse_uid);
    upload_content(ssh_target, &litestream_script, "/tmp/pull_litestream.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/pull_litestream.sh")?;
    execute_remote_with_log(ssh_target, "sudo /tmp/pull_litestream.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/pull_litestream.sh")?;

    info!("Phase 6 completed successfully");
    Ok(())
}

/// Phase 7: Server Configuration (server config, initial Caddy config, initial Litestream config)
#[instrument]
pub fn phase7_server_configuration(ssh_target: &str, domain: &str) -> Result<()> {
    info!("Phase 7: Server Configuration");

    // Upload server config
    let config_content = templates::server_config_template(domain);
    upload_content(ssh_target, &config_content, "/tmp/server-config.toml")?;
    execute_remote(
        ssh_target,
        "sudo mv /tmp/server-config.toml /opt/litehouse/config/server-config.toml",
    )?;
    execute_remote(
        ssh_target,
        "sudo chown litehouse:litehouse /opt/litehouse/config/server-config.toml",
    )?;

    // Create initial Litestream config
    info!("Creating initial Litestream configuration");
    let litestream_config = templates::initial_litestream_config();
    upload_content(ssh_target, litestream_config, "/tmp/litestream.yml")?;
    execute_remote(
        ssh_target,
        "sudo mv /tmp/litestream.yml /opt/litehouse/data/litestream.yml",
    )?;
    execute_remote(
        ssh_target,
        "sudo chown litehouse:litehouse /opt/litehouse/data/litestream.yml",
    )?;

    // Create litestream replicas directory
    execute_remote(
        ssh_target,
        "sudo mkdir -p /opt/litehouse/data/litestream-replicas && sudo chown litehouse:litehouse /opt/litehouse/data/litestream-replicas",
    )?;

    info!("Phase 7 completed successfully");
    Ok(())
}

/// Phase 8: Log Rotation
#[instrument]
pub fn phase8_log_rotation(ssh_target: &str) -> Result<()> {
    info!("Phase 8: Log Rotation");

    let logrotate_content = templates::logrotate_template();

    // Create log directory
    execute_remote(ssh_target, "sudo mkdir -p /var/log/litehouse")?;
    execute_remote(
        ssh_target,
        "sudo chown litehouse:litehouse /var/log/litehouse",
    )?;

    // Upload logrotate config
    upload_content(ssh_target, logrotate_content, "/tmp/litehouse")?;
    execute_remote(
        ssh_target,
        "sudo mv /tmp/litehouse /etc/logrotate.d/litehouse",
    )?;
    execute_remote(ssh_target, "sudo chmod 644 /etc/logrotate.d/litehouse")?;

    info!("Phase 8 completed successfully");
    Ok(())
}

/// Phase 9: Start Caddy Container
#[instrument]
pub fn phase9_start_caddy_container(ssh_target: &str) -> Result<()> {
    info!("Phase 9: Start Caddy Container");

    // Get litehouse UID from remote system
    let uid_output = execute_remote(ssh_target, "id -u litehouse")?;
    let litehouse_uid = uid_output.trim();

    let script = templates::start_caddy_container_script(litehouse_uid);

    // Upload and execute the script
    upload_content(ssh_target, &script, "/tmp/start_caddy.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/start_caddy.sh")?;
    execute_remote(ssh_target, "sudo /tmp/start_caddy.sh")?;
    execute_remote(ssh_target, "rm /tmp/start_caddy.sh")?;

    // Wait a moment for container to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify container is running
    let ps_output = execute_remote(
        ssh_target,
        &format!(
            "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman ps --filter name=caddy-container --format {{{{.Status}}}}'",
            litehouse_uid
        ),
    )?;

    if !ps_output.contains("Up") {
        anyhow::bail!("Caddy container is not running after start. Status: {}", ps_output.trim());
    }

    info!("Phase 9 completed successfully");
    Ok(())
}

/// Phase 10: Start Litestream Container
#[instrument]
pub fn phase10_start_litestream_container(ssh_target: &str) -> Result<()> {
    info!("Phase 10: Start Litestream Container");

    // Get litehouse UID from remote system
    let uid_output = execute_remote(ssh_target, "id -u litehouse")?;
    let litehouse_uid = uid_output.trim();

    let script = templates::start_litestream_container_script(litehouse_uid);

    // Upload and execute the script
    upload_content(ssh_target, &script, "/tmp/start_litestream.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/start_litestream.sh")?;
    execute_remote(ssh_target, "sudo /tmp/start_litestream.sh")?;
    execute_remote(ssh_target, "rm /tmp/start_litestream.sh")?;

    // Wait a moment for container to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify container is running
    let ps_output = execute_remote(
        ssh_target,
        &format!(
            "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman ps --filter name=litestream-container --format {{{{.Status}}}}'",
            litehouse_uid
        ),
    )?;

    if !ps_output.contains("Up") {
        // Litestream might fail if the database doesn't exist yet - that's okay, just warn
        info!("Litestream container may not be running yet (database might not exist). Status: {}", ps_output.trim());
    }

    info!("Phase 10 completed successfully");
    Ok(())
}

/// Phase 11: Start litehouse-server Container
#[instrument]
pub fn phase11_start_litehouse_container(ssh_target: &str) -> Result<()> {
    info!("Phase 11: Start litehouse-server Container");

    // Get litehouse UID from remote system
    let uid_output = execute_remote(ssh_target, "id -u litehouse")?;
    let litehouse_uid = uid_output.trim();

    // Stop and remove any existing litehouse-server container (--time 0 for immediate stop)
    let _ = execute_remote(
        ssh_target,
        &format!(
            "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman stop --time 0 -i litehouse-server'",
            litehouse_uid
        ),
    );
    let _ = execute_remote(
        ssh_target,
        &format!(
            "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman rm -f litehouse-server'",
            litehouse_uid
        ),
    );

    // Start litehouse-server container with --restart=unless-stopped
    info!("Starting litehouse-server container with restart policy");
    let start_command = format!(
        r#"cd /tmp && sudo -u litehouse bash -c '
export XDG_RUNTIME_DIR=/run/user/{}
export PODMAN_SOCK=/run/user/{}/podman/podman.sock

podman run -d \
  --name litehouse-server \
  --restart=unless-stopped \
  --replace \
  --userns=keep-id \
  -p 3030:3030 \
  -v /opt/litehouse/config:/opt/litehouse/config \
  -v /opt/litehouse/data:/opt/litehouse/data \
  -v /run/user/{}/podman/podman.sock:/run/podman/podman.sock \
  -e DATABASE_URL=/opt/litehouse/config/litehouse.db \
  -e LITEHOUSE_DIR=/opt/litehouse \
  -e PODMAN_SOCK=/run/podman/podman.sock \
  -e RUST_LOG=info \
  ghcr.io/danbruder/litehouse:latest
'"#,
        litehouse_uid, litehouse_uid, litehouse_uid
    );

    let run_result = execute_remote(ssh_target, &start_command);

    // Check if the run command itself failed
    if let Err(e) = run_result {
        anyhow::bail!("Failed to start container: {}", e);
    }

    // Wait a moment for container to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify container is running
    let ps_output = execute_remote(
        ssh_target,
        &format!(
            "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman ps --filter name=litehouse-server --format {{{{.Status}}}}'",
            litehouse_uid
        ),
    )?;

    if !ps_output.contains("Up") {
        // Container is not running - get the logs to understand why
        let logs_result = execute_remote(
            ssh_target,
            &format!(
                "cd /tmp && sudo -u litehouse bash -c 'export XDG_RUNTIME_DIR=/run/user/{}; podman logs litehouse-server 2>&1 | tail -50'",
                litehouse_uid
            ),
        );

        let logs = logs_result.unwrap_or_else(|_| "Could not retrieve container logs".to_string());

        anyhow::bail!(
            "litehouse-server container is not running. Container logs:\n{}",
            logs
        );
    }

    info!("litehouse-server container started successfully");
    info!("Phase 11 completed successfully");
    Ok(())
}

/// Phase 12: Configure Caddy with initial routing
#[instrument]
pub fn phase12_configure_caddy(ssh_target: &str, domain: &str) -> Result<()> {
    info!("Phase 12: Configure Caddy with initial routing");

    let caddy_config = templates::initial_caddy_config(domain);

    // Send configuration to Caddy's admin API via curl on the remote server
    // We need to do this from the server since Caddy is only accessible there
    let config_escaped = caddy_config.replace("'", "'\\''");
    let curl_command = format!(
        r#"curl -s -X POST -H "Content-Type: application/json" -d '{}' http://localhost:2019/load"#,
        config_escaped
    );

    // Retry a few times in case Caddy is still starting up
    let max_retries = 10;
    let mut retries = 0;
    let mut last_error = String::new();

    while retries < max_retries {
        match execute_remote(ssh_target, &curl_command) {
            Ok(output) => {
                if output.trim().is_empty() || output.contains("success") || !output.contains("error") {
                    info!("Caddy configuration applied successfully");
                    info!("Phase 12 completed successfully");
                    return Ok(());
                } else {
                    last_error = output;
                }
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }

        retries += 1;
        if retries < max_retries {
            info!("Caddy API not ready yet, retrying... ({}/{})", retries, max_retries);
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    anyhow::bail!(
        "Failed to configure Caddy after {} attempts. Last error: {}",
        max_retries,
        last_error
    );
}

/// Phase 13: Verify Docker Auto-Restart
#[instrument]
pub fn phase13_verify_docker_restart(ssh_target: &str) -> Result<()> {
    info!("Phase 13: Verify Docker Auto-Restart");

    // Verify Docker service is enabled to start on boot
    let status = execute_remote(ssh_target, "systemctl is-enabled docker")?;

    if status.trim() != "enabled" {
        anyhow::bail!("Docker service is not enabled to start on boot");
    }

    info!("Docker service is enabled - containers with restart policy will auto-restart on boot");
    info!("Phase 13 completed successfully");
    Ok(())
}

/// Phase 14: Client Configuration
#[instrument]
pub fn phase14_client_configuration(domain: &str) -> Result<()> {
    info!("Phase 14: Client Configuration");

    // Load or create client config
    let base_url = format!("http://admin.{}", domain);

    let client_config = crate::config::ClientConfig { base_url };
    client_config.save()?;

    info!("Client config updated to: {}", client_config.base_url);
    info!("Phase 14 completed successfully");
    Ok(())
}

/// Phase 15: Verification
#[instrument]
pub fn phase15_verification(ssh_target: &str, domain: &str) -> Result<()> {
    info!("Phase 15: Verification");

    let api_url = format!("http://admin.{}/apps", domain);
    info!("Testing API endpoint: {}", api_url);

    // Test from server via curl to avoid tokio runtime nesting issues
    let curl_command = format!("curl -s -o /dev/null -w '%{{http_code}}' {}", api_url);

    // Retry with backoff
    let max_retries = 30;
    let mut retries = 0;
    let mut last_error = String::new();

    while retries < max_retries {
        match execute_remote(ssh_target, &curl_command) {
            Ok(output) => {
                let status_code = output.trim();
                if status_code == "200" {
                    info!("API endpoint responding successfully!");
                    info!("Phase 15 completed successfully");
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
