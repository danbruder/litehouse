use anyhow::{Context, Result};
use tracing::{info, instrument};
use indicatif::ProgressBar;

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
    if !std::process::Command::new("ssh")
        .arg("-V")
        .output()
        .is_ok()
    {
        anyhow::bail!("SSH command not found. Please install OpenSSH client.");
    }

    // Check if SCP is available locally
    if !std::process::Command::new("scp")
        .arg("-h")
        .output()
        .is_ok()
    {
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

/// Phase 5: Podman Configuration
#[instrument(skip(log_window))]
pub fn phase5_podman_configuration(ssh_target: &str, log_window: Option<&ProgressBar>) -> Result<String> {
    info!("Phase 5: Podman Configuration");

    let script = templates::podman_setup_script();

    // Upload and execute the script
    upload_content(ssh_target, script, "/tmp/podman_setup.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/podman_setup.sh")?;
    let output = execute_remote_with_log(ssh_target, "sudo /tmp/podman_setup.sh", log_window)?;
    execute_remote(ssh_target, "rm /tmp/podman_setup.sh")?;

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

/// Phase 6: Systemd Service Setup
#[instrument]
pub fn phase6_systemd_service(ssh_target: &str, litehouse_uid: &str) -> Result<()> {
    info!("Phase 6: Systemd Service Setup");

    let service_content = templates::systemd_service_template(litehouse_uid);

    // Upload service file
    upload_content(ssh_target, &service_content, "/tmp/litehouse.service")?;
    execute_remote(ssh_target, "sudo mv /tmp/litehouse.service /etc/systemd/system/litehouse.service")?;
    execute_remote(ssh_target, "sudo chmod 644 /etc/systemd/system/litehouse.service")?;

    // Reload systemd
    execute_remote(ssh_target, "sudo systemctl daemon-reload")?;

    info!("Phase 6 completed successfully");
    Ok(())
}

/// Phase 7: Binary Build & Deployment
#[instrument(skip(log_window))]
pub fn phase7_binary_deployment(ssh_target: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::collections::VecDeque;

    info!("Phase 7: Binary Build & Deployment");

    info!("Building Linux musl binary...");

    // Build the binary with streaming output
    let mut child = std::process::Command::new("cargo")
        .args(["build", "--release", "--target", "x86_64-unknown-linux-musl"])
        .env("TARGET_CC", "x86_64-linux-musl-gcc")
        .env("SQLX_OFFLINE", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to execute cargo build")?;

    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

    // Read stdout
    let stdout_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        for line in reader.lines() {
            if let Ok(line) = line {
                lines.push(line);
            }
        }
        lines
    });

    // Read stderr with log window updates
    let stderr_handle = std::thread::spawn({
        let log_window = log_window.map(|pb| pb.clone());
        move || {
            let reader = BufReader::new(stderr);
            let mut lines = Vec::new();
            let mut log_buffer: VecDeque<String> = VecDeque::with_capacity(20);
            for line in reader.lines() {
                if let Ok(line) = line {
                    lines.push(line.clone());

                    // Update log buffer
                    if log_buffer.len() >= 20 {
                        log_buffer.pop_front();
                    }
                    log_buffer.push_back(line);

                    // Update log window if provided
                    if let Some(ref pb) = log_window {
                        let log_display: Vec<String> = log_buffer.iter().cloned().collect();
                        pb.set_message(log_display.join("\n"));
                    }
                }
            }
            lines
        }
    });

    let stdout_lines = stdout_handle.join().unwrap();
    let stderr_lines = stderr_handle.join().unwrap();

    let status = child.wait().context("Failed to wait for cargo build")?;

    if !status.success() {
        anyhow::bail!(
            "Failed to build binary:\nSTDOUT: {}\nSTDERR: {}",
            stdout_lines.join("\n"),
            stderr_lines.join("\n")
        );
    }

    info!("Binary built successfully");

    // Upload binary
    let binary_path = "target/x86_64-unknown-linux-musl/release/lh";
    if !std::path::Path::new(binary_path).exists() {
        anyhow::bail!("Binary not found at {}", binary_path);
    }

    info!("Uploading binary to server...");
    super::ssh::upload_file(ssh_target, binary_path, "/tmp/lh")?;

    // Move binary to final location
    execute_remote(ssh_target, "sudo mv /tmp/lh /opt/litehouse/lh")?;
    execute_remote(ssh_target, "sudo chmod +x /opt/litehouse/lh")?;
    execute_remote(ssh_target, "sudo chown litehouse:litehouse /opt/litehouse/lh")?;

    info!("Phase 7 completed successfully");
    Ok(())
}

/// Phase 7.5: Server Configuration
#[instrument]
pub fn phase7_5_server_configuration(ssh_target: &str, domain: &str) -> Result<()> {
    info!("Phase 7.5: Server Configuration");

    let config_content = templates::server_config_template(domain);

    // Upload server config
    upload_content(ssh_target, &config_content, "/tmp/server-config.toml")?;
    execute_remote(ssh_target, "sudo mv /tmp/server-config.toml /opt/litehouse/config/server-config.toml")?;
    execute_remote(ssh_target, "sudo chown litehouse:litehouse /opt/litehouse/config/server-config.toml")?;

    info!("Phase 7.5 completed successfully");
    Ok(())
}

/// Phase 8: Log Rotation
#[instrument]
pub fn phase8_log_rotation(ssh_target: &str) -> Result<()> {
    info!("Phase 8: Log Rotation");

    let logrotate_content = templates::logrotate_template();

    // Create log directory
    execute_remote(ssh_target, "sudo mkdir -p /var/log/litehouse")?;
    execute_remote(ssh_target, "sudo chown litehouse:litehouse /var/log/litehouse")?;

    // Upload logrotate config
    upload_content(ssh_target, logrotate_content, "/tmp/litehouse")?;
    execute_remote(ssh_target, "sudo mv /tmp/litehouse /etc/logrotate.d/litehouse")?;
    execute_remote(ssh_target, "sudo chmod 644 /etc/logrotate.d/litehouse")?;

    info!("Phase 8 completed successfully");
    Ok(())
}

/// Phase 9: Service Start
#[instrument]
pub fn phase9_service_start(ssh_target: &str) -> Result<()> {
    info!("Phase 9: Service Start");

    // Enable service
    execute_remote(ssh_target, "sudo systemctl enable litehouse")?;

    // Start service
    execute_remote(ssh_target, "sudo systemctl start litehouse")?;

    // Wait for service to be active
    let wait_script = templates::wait_for_service_script();
    upload_content(ssh_target, wait_script, "/tmp/wait_for_service.sh")?;
    execute_remote(ssh_target, "chmod +x /tmp/wait_for_service.sh")?;
    execute_remote(ssh_target, "/tmp/wait_for_service.sh litehouse")?;
    execute_remote(ssh_target, "rm /tmp/wait_for_service.sh")?;

    // Check logs for any errors
    let logs = execute_remote(ssh_target, "sudo journalctl -u litehouse -n 20 --no-pager")?;
    info!("Recent service logs:\n{}", logs);

    // Verify service is running
    let status = execute_remote(ssh_target, "systemctl is-active litehouse")?;
    if status.trim() != "active" {
        anyhow::bail!("Service is not active. Status: {}", status.trim());
    }

    info!("Phase 9 completed successfully");
    Ok(())
}

/// Phase 10: Client Configuration
#[instrument]
pub fn phase10_client_configuration(domain: &str) -> Result<()> {
    info!("Phase 10: Client Configuration");

    // Load or create client config
    let base_url = format!("http://admin-api.{}", domain);

    let client_config = crate::config::ClientConfig { base_url };
    client_config.save()?;

    info!("Client config updated to: {}", client_config.base_url);
    info!("Phase 10 completed successfully");
    Ok(())
}

/// Phase 11: Verification
#[instrument]
pub fn phase11_verification(domain: &str) -> Result<()> {
    info!("Phase 11: Verification");

    let api_url = format!("http://admin-api.{}/apps", domain);
    info!("Testing API endpoint: {}", api_url);

    // Retry with backoff
    let max_retries = 30;
    let mut retries = 0;
    let mut last_error = String::new();

    while retries < max_retries {
        match reqwest::blocking::get(&api_url) {
            Ok(response) => {
                if response.status().is_success() {
                    info!("API endpoint responding successfully!");
                    info!("Phase 11 completed successfully");
                    return Ok(());
                } else {
                    last_error = format!("HTTP {}", response.status());
                }
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }

        retries += 1;
        if retries < max_retries {
            info!("API not ready yet, retrying... ({}/{})", retries, max_retries);
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    anyhow::bail!(
        "Failed to verify API endpoint after {} attempts. Last error: {}",
        max_retries,
        last_error
    );
}
