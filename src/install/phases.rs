use anyhow::{Context, Result};
use indicatif::ProgressBar;
use tracing::{info, instrument, warn};

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

/// Phase 5: Docker Configuration
#[instrument(skip(log_window))]
pub fn phase5_docker_configuration(log_window: Option<&ProgressBar>) -> Result<String> {
    info!("Phase 5: Docker Configuration");

    let script = templates::docker_setup_script();

    // Write and execute the script
    std::fs::write("/tmp/docker_setup.sh", script)?;
    run_command("chmod +x /tmp/docker_setup.sh")?;
    let output = run_command_with_log("/tmp/docker_setup.sh", log_window)?;
    run_command("rm /tmp/docker_setup.sh")?;

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

/// Get litehouse user UID
pub fn get_litehouse_uid() -> Result<String> {
    let output = run_command("id -u litehouse")?;
    Ok(output.trim().to_string())
}

/// Phase 6a: Pull the litehouse-server image from GHCR
///
/// Returns the tag that was actually pulled (the requested `version`, or
/// `"latest"` if that exact version isn't published yet) so the caller can
/// use the same image reference when starting the container.
#[instrument(skip(log_window))]
pub fn phase6a_pull_litehouse_image(version: &str, log_window: Option<&ProgressBar>) -> Result<String> {
    info!("Phase 6a: Pull litehouse image from GHCR");

    let script = templates::pull_litehouse_image_script(version);
    std::fs::write("/tmp/pull_litehouse.sh", &script)?;
    run_command("chmod +x /tmp/pull_litehouse.sh")?;
    let output = run_command_with_log("/tmp/pull_litehouse.sh", log_window)?;
    run_command("rm /tmp/pull_litehouse.sh")?;

    let tag_line = output
        .lines()
        .find(|line| line.starts_with("PULLED_TAG:"))
        .context("Failed to determine which litehouse image tag was pulled")?;
    let tag = tag_line
        .strip_prefix("PULLED_TAG:")
        .context("Failed to parse pulled image tag")?
        .trim()
        .to_string();

    if tag != version {
        warn!(
            "Requested litehouse image version {} was not available; fell back to {}",
            version, tag
        );
    }

    info!("Phase 6a completed successfully (pulled tag: {})", tag);
    Ok(tag)
}

/// Phase 6b: Pull Caddy and backup/restore helper images (sqlite3, alpine)
#[instrument(skip(log_window))]
pub fn phase6b_pull_images(litehouse_uid: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    info!("Phase 6b: Pull Caddy and helper images");

    let script = templates::pull_images_script(litehouse_uid);
    std::fs::write("/tmp/pull_images.sh", &script)?;
    run_command("chmod +x /tmp/pull_images.sh")?;
    run_command_with_log("/tmp/pull_images.sh", log_window)?;
    run_command("rm /tmp/pull_images.sh")?;

    info!("Phase 6b completed successfully");
    Ok(())
}

/// Phase 7: Server Configuration
///
/// Generates a fresh admin token, persists only its hash in
/// server-config.toml (never the plaintext), and returns the plaintext
/// token so the caller can print it exactly once at the end of install.
#[instrument(skip(s3_config))]
pub fn phase7_server_configuration(
    domain: &str,
    s3_config: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
) -> Result<String> {
    info!("Phase 7: Server Configuration");

    // Generate the single admin token for this install. Only its hash is
    // ever written to disk; the plaintext is handed back to the caller.
    let admin_token = crate::auth::generate_token();
    let admin_token_hash = crate::auth::hash_token(&admin_token);

    // Write server config
    let config_content = templates::server_config_template(domain, &admin_token_hash);
    sudo_write_file("/opt/litehouse/config/server-config.toml", &config_content)?;
    run_command("chown litehouse:litehouse /opt/litehouse/config/server-config.toml")?;

    // Write S3 credentials if provided
    if let (Some(access_key), Some(secret_key), Some(bucket), Some(region)) =
        (&s3_config.0, &s3_config.1, &s3_config.2, &s3_config.3)
    {
        info!("Writing S3 credentials");

        let mut s3_env = format!(
            "S3_ACCESS_KEY_ID={}\nS3_SECRET_ACCESS_KEY={}\nS3_BUCKET={}\nS3_REGION={}\n",
            access_key, secret_key, bucket, region
        );

        if let Some(endpoint) = &s3_config.4 {
            s3_env.push_str(&format!("S3_ENDPOINT={}\n", endpoint));
        }

        if let Some(path_prefix) = &s3_config.5 {
            s3_env.push_str(&format!("S3_PATH_PREFIX={}\n", path_prefix));
        }

        sudo_write_file("/opt/litehouse/config/s3-credentials.env", &s3_env)?;
        run_command("chown root:root /opt/litehouse/config/s3-credentials.env")?;
        run_command("chmod 600 /opt/litehouse/config/s3-credentials.env")?;
        info!("S3 credentials file created with 600 permissions");
    }

    info!("Phase 7 completed successfully");
    Ok(admin_token)
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

/// Phase 9a: Start Caddy Container
#[instrument]
pub fn phase9a_start_caddy_container(litehouse_uid: &str, domain: &str) -> Result<()> {
    info!("Phase 9a: Start Caddy Container");

    let script = templates::start_caddy_container_script(litehouse_uid);
    std::fs::write("/tmp/start_caddy.sh", &script)?;
    run_command("chmod +x /tmp/start_caddy.sh")?;
    run_command("/tmp/start_caddy.sh")?;
    run_command("rm /tmp/start_caddy.sh")?;

    // Wait a moment for container to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify container is running
    let ps_output = run_command("docker ps --filter name=caddy-container --format '{{.Status}}'")?;

    if !ps_output.contains("Up") {
        // Container is not running - get the logs to understand why
        let logs = run_command("docker logs caddy-container 2>&1 | tail -50")
            .unwrap_or_else(|_| "Could not retrieve container logs".to_string());

        anyhow::bail!(
            "caddy-container is not running. Container logs:\n{}",
            logs
        );
    }

    // Wait for Caddy admin API to be ready (with retries)
    info!("Waiting for Caddy admin API to be ready...");
    let max_retries = 10;
    let mut retries = 0;
    let mut api_ready = false;

    while retries < max_retries {
        // Check if admin API is responding
        let check_cmd = "curl -s -o /dev/null -w '%{http_code}' http://localhost:2019/config/ || echo '000'";
        match run_command(check_cmd) {
            Ok(output) => {
                let status = output.trim();
                if status == "200" || status == "404" {
                    // 200 = API is responding, 404 = API is up but endpoint doesn't exist (which is fine)
                    api_ready = true;
                    break;
                }
            }
            Err(_) => {
                // curl failed, API not ready yet
            }
        }

        retries += 1;
        if retries < max_retries {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    if !api_ready {
        warn!("Caddy admin API may not be ready, but continuing anyway");
    } else {
        info!("Caddy admin API is ready");
    }

    // Load initial Caddy configuration with admin route
    info!("Loading initial Caddy configuration...");
    let initial_config = templates::initial_caddy_config(domain);
    let config_file = format!("/tmp/caddy_initial_config.json");
    std::fs::write(&config_file, &initial_config)?;

    // Send config to Caddy API
    let load_cmd = format!(
        "curl -s -X POST -H 'Content-Type: application/json' -d @{} http://localhost:2019/load",
        config_file
    );
    
    match run_command(&load_cmd) {
        Ok(output) => {
            info!("Initial Caddy configuration loaded successfully");
            if !output.trim().is_empty() {
                info!("Caddy response: {}", output.trim());
            }
        }
        Err(e) => {
            warn!("Failed to load initial Caddy configuration: {}. This may be okay if Caddy is not fully ready yet.", e);
        }
    }

    // Clean up temp file
    std::fs::remove_file(&config_file).ok();

    info!("caddy-container started successfully");
    info!("Phase 9a completed successfully");
    Ok(())
}

/// Phase 9b: Start litehouse-server Container
#[instrument]
pub fn phase9b_start_litehouse_container(litehouse_uid: &str, image_tag: &str) -> Result<()> {
    info!("Phase 9b: Start litehouse-server Container");

    let script = templates::start_litehouse_container_script(litehouse_uid, image_tag);
    std::fs::write("/tmp/start_litehouse.sh", &script)?;
    run_command("chmod +x /tmp/start_litehouse.sh")?;
    run_command("/tmp/start_litehouse.sh")?;
    run_command("rm /tmp/start_litehouse.sh")?;

    // Wait a moment for container to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify container is running
    let ps_output = run_command("docker ps --filter name=litehouse-server --format '{{.Status}}'")?;

    if !ps_output.contains("Up") {
        // Container is not running - get the logs to understand why
        let logs = run_command("docker logs litehouse-server 2>&1 | tail -50")
            .unwrap_or_else(|_| "Could not retrieve container logs".to_string());

        anyhow::bail!(
            "litehouse-server container is not running. Container logs:\n{}",
            logs
        );
    }

    info!("litehouse-server container started successfully");
    info!("Phase 9b completed successfully");
    Ok(())
}

/// Phase 10: Docker restart configuration (containers use --restart=unless-stopped)
#[instrument]
pub fn phase10_enable_docker_restart(_litehouse_uid: &str) -> Result<()> {
    info!("Phase 10: Docker restart configuration");

    // Docker containers with --restart=unless-stopped will automatically restart on boot
    // No additional systemd service needed like with Podman

    info!("Docker restart policy configured - containers will restart on boot");
    info!("Phase 10 completed successfully");
    Ok(())
}

/// Phase 11: Verification
#[instrument]
pub fn phase11_verification(domain: &str) -> Result<()> {
    info!("Phase 11: Verification");

    // Step 1: Verify DNS configuration
    info!("Step 1: Verifying DNS configuration...");

    // Get this server's public IP
    let server_ip = match run_command("curl -4 -s ifconfig.me --max-time 10") {
        Ok(ip) => ip.trim().to_string(),
        Err(_) => {
            // Try alternative service
            match run_command("curl -4 -s icanhazip.com --max-time 10") {
                Ok(ip) => ip.trim().to_string(),
                Err(e) => {
                    anyhow::bail!("Failed to determine server's public IP address: {}", e);
                }
            }
        }
    };

    info!("Server public IP: {}", server_ip);

    // Check if dig is available, otherwise use host
    let dns_command = if run_command("which dig").is_ok() {
        format!("dig +short admin.{} A | head -1", domain)
    } else {
        format!("host -t A admin.{} | grep 'has address' | awk '{{print $NF}}' | head -1", domain)
    };

    // Resolve DNS for admin subdomain
    match run_command(&dns_command) {
        Ok(resolved_ip) => {
            let resolved_ip = resolved_ip.trim();
            if resolved_ip.is_empty() {
                anyhow::bail!(
                    "DNS verification failed: admin.{} does not resolve to any IP address.\n\
                    Please configure your DNS provider to point:\n\
                    - admin.{} (A record) -> {}\n\
                    - *.{} (A record) -> {}",
                    domain, domain, server_ip, domain, server_ip
                );
            } else if resolved_ip != server_ip {
                anyhow::bail!(
                    "DNS verification failed: admin.{} resolves to {} but this server's IP is {}.\n\
                    Please update your DNS provider to point:\n\
                    - admin.{} (A record) -> {}\n\
                    - *.{} (A record) -> {}",
                    domain, resolved_ip, server_ip,
                    domain, server_ip, domain, server_ip
                );
            } else {
                info!("✓ DNS configured correctly: admin.{} -> {}", domain, resolved_ip);
            }
        }
        Err(e) => {
            anyhow::bail!(
                "DNS verification failed: Unable to resolve admin.{}: {}\n\
                Please configure your DNS provider to point:\n\
                - admin.{} (A record) -> {}\n\
                - *.{} (A record) -> {}",
                domain, e, domain, server_ip, domain, server_ip
            );
        }
    }

    // Step 2: Verify API endpoint is responding
    info!("Step 2: Verifying API endpoint is responding...");
    // Verify via loopback: the server publishes 3030 on 127.0.0.1. /login is
    // public (the UI login page) and returns 200 when the server is healthy.
    // DNS/Caddy/TLS are exercised separately (e2e acceptance final curl) —
    // install verification must not depend on ACME issuance timing.
    let _ = domain;
    let api_url = "http://127.0.0.1:3030/login".to_string();
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
                    info!("✓ API endpoint responding successfully!");
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
