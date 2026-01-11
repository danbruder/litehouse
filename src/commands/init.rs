use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracing::{error, info, instrument};

use crate::init::{phases::*, ssh::parse_ssh_target};

#[instrument]
pub async fn execute(ssh_target: &str, domain: &str) -> Result<()> {
    info!("Starting litehouse server initialization");
    info!("SSH Target: {}", ssh_target);
    info!("Domain: {}", domain);

    // Parse SSH target
    let (user, host) = parse_ssh_target(ssh_target)?;
    info!("Connecting as user '{}' to host '{}'", user, host);

    // Create multi-progress container
    let multi = MultiProgress::new();

    // Create progress bar for phases (pinned at top)
    let pb = multi.add(ProgressBar::new(11));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len}: {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    // Create log window (scrolling output below progress bar)
    let log_window = multi.add(ProgressBar::new_spinner());
    log_window.set_style(
        ProgressStyle::default_spinner()
            .template("{msg}")
            .unwrap(),
    );
    log_window.set_message("Waiting for output...");

    // Phase 1: Validation
    pb.set_message("Validating prerequisites and connectivity...");
    if let Err(e) = phase1_validation(ssh_target, domain) {
        pb.finish_with_message("❌ Validation failed");
        error!("Phase 1 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 2: System Preparation
    pb.set_message("Preparing system (updating packages, installing dependencies)...");
    log_window.set_message("Starting system preparation...");
    if let Err(e) = phase2_system_preparation(ssh_target, Some(&log_window)) {
        pb.finish_with_message("❌ System preparation failed");
        error!("Phase 2 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 3: Security Hardening
    pb.set_message("Configuring security (firewall, fail2ban)...");
    log_window.set_message("Starting security hardening...");
    if let Err(e) = phase3_security_hardening(ssh_target, Some(&log_window)) {
        pb.finish_with_message("❌ Security hardening failed");
        error!("Phase 3 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 4: User Setup
    pb.set_message("Creating litehouse user and directories...");
    log_window.set_message("Starting user setup...");
    if let Err(e) = phase4_user_setup(ssh_target, Some(&log_window)) {
        pb.finish_with_message("❌ User setup failed");
        error!("Phase 4 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 5: Podman Configuration
    pb.set_message("Configuring Podman (rootless mode, socket)...");
    log_window.set_message("Starting Podman configuration...");
    let litehouse_uid = match phase5_podman_configuration(ssh_target, Some(&log_window)) {
        Ok(uid) => uid,
        Err(e) => {
            pb.finish_with_message("❌ Podman configuration failed");
            error!("Phase 5 failed: {}", e);
            return Err(e);
        }
    };
    pb.inc(1);

    // Phase 6: Systemd Service
    pb.set_message("Setting up systemd service...");
    if let Err(e) = phase6_systemd_service(ssh_target, &litehouse_uid) {
        pb.finish_with_message("❌ Systemd service setup failed");
        error!("Phase 6 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 7: Container Image Pull
    pb.set_message("Pulling litehouse server container image...");
    log_window.set_message("Starting container image pull...");
    if let Err(e) = phase7_binary_deployment(ssh_target, Some(&log_window)) {
        pb.finish_with_message("❌ Container image pull failed");
        error!("Phase 7 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 7.5: Server Configuration
    pb.set_message("Creating server configuration...");
    if let Err(e) = phase7_5_server_configuration(ssh_target, domain) {
        pb.finish_with_message("❌ Server configuration failed");
        error!("Phase 7.5 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 8: Log Rotation
    pb.set_message("Configuring log rotation...");
    if let Err(e) = phase8_log_rotation(ssh_target) {
        pb.finish_with_message("❌ Log rotation setup failed");
        error!("Phase 8 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 9: Service Start
    pb.set_message("Starting litehouse service...");
    if let Err(e) = phase9_service_start(ssh_target) {
        pb.finish_with_message("❌ Service start failed");
        error!("Phase 9 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 10: Client Configuration
    pb.set_message("Updating local client configuration...");
    if let Err(e) = phase10_client_configuration(domain) {
        pb.finish_with_message("❌ Client configuration failed");
        error!("Phase 10 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 11: Verification
    pb.set_message("Verifying server is responding...");
    if let Err(e) = phase11_verification(domain) {
        pb.finish_with_message("❌ Verification failed");
        error!("Phase 11 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    pb.finish_with_message("✅ Initialization completed successfully!");
    log_window.finish_and_clear();

    // Print success message with next steps
    println!("\n{}", "=".repeat(60));
    println!("✓ Server initialized successfully at {}", domain);
    println!("✓ Client configured to use http://admin-api.{}", domain);
    println!("✓ Server is responding to API requests");
    println!("{}", "=".repeat(60));
    println!("\nNext steps:");
    println!("  1. Create a test app:");
    println!("     lh create myapp");
    println!("\n  2. Add a git remote:");
    println!("     lh remote myapp add git@github.com:danbruder/bindrop-example-go.git");
    println!("\n  3. Build and start your app:");
    println!("     lh build myapp && lh start myapp");
    println!("\n  4. View your app at:");
    println!("     http://myapp.{}", domain);
    println!("\nTroubleshooting:");
    println!("  - Check service status:");
    println!("    ssh litehouse@{} 'systemctl status litehouse'", host);
    println!("  - View container logs:");
    println!("    ssh litehouse@{} 'podman logs -f litehouse-server'", host);
    println!("  - Check container status:");
    println!("    ssh litehouse@{} 'podman ps -a | grep litehouse-server'", host);
    println!("{}", "=".repeat(60));

    Ok(())
}
