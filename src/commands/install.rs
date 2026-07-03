use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracing::{error, info, instrument};

use crate::install::phases::*;

/// Parallelization Strategy:
///
/// Phase 1: Validation (must be first)
///     │
///     v
/// Phase 2: System Preparation (apt-get install)
///     │
///     ├──────────────────┐
///     v                  v
/// Phase 3: Security    Phase 4: User Setup
/// (firewall/fail2ban)  (litehouse user, dirs)
///     │                  │
///     └────────┬─────────┘
///              v
/// Phase 5: Podman Configuration (needs user)
///              │
///     ┌────────┼────────┬─────────┐
///     v        v        v         v
/// Build     Pull      Phase 7   Phase 8
/// litehouse caddy     Config   LogRotate
/// locally     │        │         │
///     └────────┴────────┴─────────┘
///              │
///              v
/// Phase 9a: Start Caddy container
///              │
///              v
/// Phase 9b: Start litehouse-server container
///              │
///              v
/// Phase 10: Enable podman-restart service
///              │
///              v
/// Phase 11: Verification (optional)

#[instrument(skip_all)]
pub async fn execute(
    domain: &str,
    skip_verify: bool,
    s3_access_key: Option<&str>,
    s3_secret_key: Option<&str>,
    s3_bucket: Option<&str>,
    s3_region: Option<&str>,
    s3_endpoint: Option<&str>,
    s3_path_prefix: Option<&str>,
    ghcr_token: Option<&str>,
) -> Result<()> {
    info!("Starting litehouse installation");
    info!("Domain: {}", domain);
    if s3_bucket.is_some() {
        info!("S3 backup configured");
    }

    // Tag we pull/run the litehouse-server image from GHCR: the exact
    // version of the binary running this install (i.e. the release tag),
    // with a `latest` fallback handled inside phase 6a if that tag isn't
    // published yet.
    let requested_version = env!("CARGO_PKG_VERSION");

    // Create multi-progress container
    let multi = MultiProgress::new();

    // Create progress bar for phases
    let total_phases = if skip_verify { 11 } else { 12 };
    let pb = multi.add(ProgressBar::new(total_phases));
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
    log_window.set_message("Starting installation...");

    // Phase 1: Validation (must be first)
    pb.set_message("Validating prerequisites...");
    if let Err(e) = phase1_validation(domain) {
        pb.finish_with_message("❌ Validation failed");
        error!("Phase 1 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 2: System Preparation
    pb.set_message("Preparing system (updating packages, installing dependencies)...");
    log_window.set_message("Starting system preparation...");
    if let Err(e) = phase2_system_preparation(Some(&log_window)) {
        pb.finish_with_message("❌ System preparation failed");
        error!("Phase 2 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phases 3 & 4: Security hardening and User setup (can run in parallel)
    pb.set_message("Configuring security and creating user (parallel)...");
    log_window.set_message("Starting parallel configuration...");

    let (security_result, user_result) = {
        let log_window_clone = log_window.clone();
        let log_window_clone2 = log_window.clone();

        let security_handle = tokio::task::spawn_blocking(move || {
            phase3_security_hardening(Some(&log_window_clone))
        });

        let user_handle = tokio::task::spawn_blocking(move || {
            phase4_user_setup(Some(&log_window_clone2))
        });

        let security_result = security_handle.await.map_err(|e| anyhow::anyhow!("Security task panicked: {}", e))?;
        let user_result = user_handle.await.map_err(|e| anyhow::anyhow!("User setup task panicked: {}", e))?;

        (security_result, user_result)
    };

    if let Err(e) = security_result {
        pb.finish_with_message("❌ Security hardening failed");
        error!("Phase 3 failed: {}", e);
        return Err(e);
    }

    if let Err(e) = user_result {
        pb.finish_with_message("❌ User setup failed");
        error!("Phase 4 failed: {}", e);
        return Err(e);
    }
    pb.inc(1); // Count the parallel phases as one step

    // Phase 5: Docker Configuration (needs user to be set up)
    pb.set_message("Configuring Docker...");
    log_window.set_message("Starting Docker configuration...");
    let litehouse_uid = match phase5_docker_configuration(Some(&log_window)) {
        Ok(uid) => uid,
        Err(e) => {
            pb.finish_with_message("❌ Docker configuration failed");
            error!("Phase 5 failed: {}", e);
            return Err(e);
        }
    };
    pb.inc(1);

    // Phases 6 (images), 7 (config), 8 (logrotate) - can all run in parallel
    pb.set_message("Building/pulling images and configuring (parallel)...");
    log_window.set_message("Starting parallel image build/pulls and configuration...");

    let uid_for_caddy = litehouse_uid.clone();
    let domain_for_config = domain.to_string();
    let version_for_pull = requested_version.to_string();

    // Prepare S3 config for phase7
    let s3_config = (
        s3_access_key.map(|s| s.to_string()),
        s3_secret_key.map(|s| s.to_string()),
        s3_bucket.map(|s| s.to_string()),
        s3_region.map(|s| s.to_string()),
        s3_endpoint.map(|s| s.to_string()),
        s3_path_prefix.map(|s| s.to_string()),
    );

    let (litehouse_result, caddy_result, config_result, logrotate_result) = {
        let log_window_litehouse = log_window.clone();
        let log_window_caddy = log_window.clone();

        let litehouse_handle = tokio::task::spawn_blocking(move || {
            phase6a_pull_litehouse_image(&version_for_pull, Some(&log_window_litehouse))
        });

        let caddy_handle = tokio::task::spawn_blocking(move || {
            phase6b_pull_images(&uid_for_caddy, Some(&log_window_caddy))
        });

        let config_handle = tokio::task::spawn_blocking(move || {
            phase7_server_configuration(&domain_for_config, s3_config)
        });

        let logrotate_handle = tokio::task::spawn_blocking(move || {
            phase8_log_rotation()
        });

        let litehouse_result = litehouse_handle.await.map_err(|e| anyhow::anyhow!("Litehouse image pull panicked: {}", e))?;
        let caddy_result = caddy_handle.await.map_err(|e| anyhow::anyhow!("Caddy image pull panicked: {}", e))?;
        let config_result = config_handle.await.map_err(|e| anyhow::anyhow!("Config task panicked: {}", e))?;
        let logrotate_result = logrotate_handle.await.map_err(|e| anyhow::anyhow!("Logrotate task panicked: {}", e))?;

        (litehouse_result, caddy_result, config_result, logrotate_result)
    };

    let image_tag = match litehouse_result {
        Ok(tag) => tag,
        Err(e) => {
            pb.finish_with_message("❌ Litehouse image pull failed");
            error!("Phase 6a failed: {}", e);
            return Err(e);
        }
    };

    if let Err(e) = caddy_result {
        pb.finish_with_message("❌ Image pull failed");
        error!("Phase 6b failed: {}", e);
        return Err(e);
    }

    let admin_token = match config_result {
        Ok(token) => token,
        Err(e) => {
            pb.finish_with_message("❌ Server configuration failed");
            error!("Phase 7 failed: {}", e);
            return Err(e);
        }
    };

    if let Err(e) = logrotate_result {
        pb.finish_with_message("❌ Log rotation setup failed");
        error!("Phase 8 failed: {}", e);
        return Err(e);
    }
    pb.inc(1); // Count the parallel phases as one step

    // Phase 9a: Start Caddy Container (must be before litehouse-server)
    pb.set_message("Starting Caddy container...");
    if let Err(e) = phase9a_start_caddy_container(&litehouse_uid, domain) {
        pb.finish_with_message("❌ Caddy container start failed");
        error!("Phase 9a failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 9b: Start litehouse-server Container
    pb.set_message("Starting litehouse-server container...");
    if let Err(e) = phase9b_start_litehouse_container(&litehouse_uid, &image_tag) {
        pb.finish_with_message("❌ litehouse-server container start failed");
        error!("Phase 9b failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // If a GHCR token was provided, push it to the freshly started server
    // via its own local admin API — the host `lh` binary has no direct DB
    // access (the database lives inside the litehouse_config Docker
    // volume), so this is the simplest reliable way to seed it.
    if let Some(token) = ghcr_token {
        pb.set_message("Configuring GHCR token...");
        if let Err(e) = configure_ghcr_token(token, &admin_token).await {
            // Non-fatal: the operator can always run `lh config ghcr set`
            // later once connected.
            tracing::warn!("Failed to configure GHCR token during install: {:#}", e);
        }
    }

    // Phase 10: Docker restart configuration
    pb.set_message("Configuring Docker restart policy...");
    if let Err(e) = phase10_enable_docker_restart(&litehouse_uid) {
        pb.finish_with_message("❌ Docker restart configuration failed");
        error!("Phase 10 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 11: Verification (optional)
    if !skip_verify {
        pb.set_message("Verifying server is responding...");
        if let Err(e) = phase11_verification(domain) {
            pb.finish_with_message("❌ Verification failed");
            error!("Phase 11 failed: {}", e);
            return Err(e);
        }
        pb.inc(1);
    }

    pb.finish_with_message("✅ Installation completed successfully!");
    log_window.finish_and_clear();

    // Print success message with next steps. The plaintext admin token is
    // printed here and ONLY here — server-config.toml only ever stores its
    // hash.
    println!("\n{}", "=".repeat(60));
    println!("✓ litehouse installed");
    println!("  admin UI:  https://admin.{}", domain);
    println!(
        "  connect:   lh connect https://admin.{} --token {}",
        domain, admin_token
    );
    println!("{}", "=".repeat(60));
    println!("\nNext steps:");
    println!("  1. Connect the CLI (see command above)");
    println!("  2. Create an app from a GitHub repo:");
    println!("     lh create myapp --repo you/repo");
    println!("\n  3. Push to deploy:");
    println!("     git push");
    println!("\n  4. View your app at:");
    println!("     https://myapp.{}", domain);
    println!("\nTroubleshooting:");
    println!("  - View container logs:");
    println!("    docker logs -f litehouse-server");
    println!("  - Check container status:");
    println!("    docker ps -a");
    println!("  - Restart a container:");
    println!("    docker restart litehouse-server");
    println!("{}", "=".repeat(60));

    Ok(())
}

/// POST the given GHCR PAT to the local admin API using the freshly
/// generated admin token. Runs against `localhost` (not the public domain)
/// so it works before DNS/Caddy verification even completes.
async fn configure_ghcr_token(ghcr_token: &str, admin_token: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let url = "http://localhost:3030/api/config/ghcr";

    // The container may have just started; retry briefly.
    let max_retries = 10;
    let mut last_err = None;
    for attempt in 0..max_retries {
        match client
            .post(url)
            .bearer_auth(admin_token)
            .json(&serde_json::json!({ "token": ghcr_token }))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                info!("GHCR token configured");
                return Ok(());
            }
            Ok(resp) => {
                last_err = Some(anyhow::anyhow!(
                    "server responded with {}",
                    resp.status()
                ));
            }
            Err(e) => {
                last_err = Some(anyhow::anyhow!(e));
            }
        }
        if attempt + 1 < max_retries {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("unknown error configuring GHCR token")))
}
