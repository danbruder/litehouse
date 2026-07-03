use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracing::{info, instrument};

use crate::install::executor::run_command;
use crate::install::phases::{
    get_litehouse_uid, phase6a_pull_litehouse_image, phase9b_start_litehouse_container,
};

const GITHUB_REPO: &str = "danbruder/litehouse";

/// Downloads the latest litehouse binary from GitHub releases
fn download_binary(version: Option<&str>, log_window: Option<&ProgressBar>) -> Result<String> {
    let version_str = version.unwrap_or("latest");

    if let Some(pb) = log_window {
        pb.set_message(format!(
            "Downloading litehouse binary (version: {})...",
            version_str
        ));
    }
    info!("Downloading litehouse binary (version: {})", version_str);

    // Determine architecture
    let arch_output = run_command("uname -m")?;
    let arch = match arch_output.trim() {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        other => anyhow::bail!("Unsupported architecture: {}", other),
    };

    // Determine download URL
    let download_url = if version_str == "latest" {
        format!(
            "https://github.com/{}/releases/latest/download/litehouse-linux-{}.tar.gz",
            GITHUB_REPO, arch
        )
    } else {
        format!(
            "https://github.com/{}/releases/download/{}/litehouse-linux-{}.tar.gz",
            GITHUB_REPO, version_str, arch
        )
    };

    info!("Download URL: {}", download_url);

    // Create temp directory and download
    let temp_dir = run_command("mktemp -d")?.trim().to_string();

    let download_cmd = format!(
        "curl -fsSL '{}' -o '{}/litehouse.tar.gz'",
        download_url, temp_dir
    );

    run_command(&download_cmd).context("Failed to download litehouse binary")?;

    // Extract
    if let Some(pb) = log_window {
        pb.set_message("Extracting binary...");
    }
    let extract_cmd = format!("tar -xzf '{}/litehouse.tar.gz' -C '{}'", temp_dir, temp_dir);
    run_command(&extract_cmd).context("Failed to extract archive")?;

    // Find the binary
    let find_cmd = format!(
        "find '{}' -name 'lh' -o -name 'litehouse' | head -1",
        temp_dir
    );
    let binary_path = run_command(&find_cmd)?.trim().to_string();

    if binary_path.is_empty() {
        // Try finding any executable
        let find_exec_cmd = format!("find '{}' -type f -executable | head -1", temp_dir);
        let exec_path = run_command(&find_exec_cmd)?.trim().to_string();
        if exec_path.is_empty() {
            anyhow::bail!("Could not find litehouse binary in downloaded archive");
        }
        return Ok(exec_path);
    }

    Ok(binary_path)
}

/// Installs the new binary to /usr/local/bin/lh
fn install_binary(binary_path: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    if let Some(pb) = log_window {
        pb.set_message("Installing new binary...");
    }
    info!(
        "Installing binary from {} to /usr/local/bin/lh",
        binary_path
    );

    // Backup current binary
    run_command("cp /usr/local/bin/lh /usr/local/bin/lh.backup 2>/dev/null || true")?;

    // Install new binary
    run_command(&format!("cp '{}' /usr/local/bin/lh", binary_path))?;
    run_command("chmod +x /usr/local/bin/lh")?;

    // Verify it works
    match run_command("/usr/local/bin/lh --version") {
        Ok(version) => {
            info!("Installed version: {}", version.trim());
            // Remove backup
            run_command("rm -f /usr/local/bin/lh.backup")?;
            Ok(())
        }
        Err(e) => {
            // Restore backup
            run_command("mv /usr/local/bin/lh.backup /usr/local/bin/lh 2>/dev/null || true")?;
            anyhow::bail!("New binary failed verification, restored backup: {}", e)
        }
    }
}

/// Parse a semver-ish version out of `lh --version` output (e.g. "lh 0.1.34"
/// -> "0.1.34"). Falls back to "latest" if nothing usable is found, which
/// lets the GHCR pull's own fallback logic take over.
fn parse_version(version_output: &str) -> String {
    version_output
        .split_whitespace()
        .last()
        .filter(|s| s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .unwrap_or("latest")
        .to_string()
}

/// Cleanup temp directory
fn cleanup_temp_dir(binary_path: &str) -> Result<()> {
    // Extract the temp directory from the binary path
    if let Some(parent) = std::path::Path::new(binary_path).parent() {
        let parent_str = parent.to_string_lossy();
        if parent_str.contains("/tmp") {
            run_command(&format!("rm -rf '{}'", parent_str))?;
        }
    }
    Ok(())
}

#[instrument]
pub async fn execute(version: Option<&str>, from_path: Option<&str>) -> Result<()> {
    info!("Starting litehouse upgrade");

    // Check if running as root
    if !crate::install::executor::is_root() {
        anyhow::bail!("This command must be run as root. Try: sudo lh upgrade");
    }

    // Create multi-progress container
    let multi = MultiProgress::new();

    // Create progress bar for phases
    let total_phases = if from_path.is_some() { 3 } else { 4 };
    let pb = multi.add(ProgressBar::new(total_phases));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len}: {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    // Create log window
    let log_window = multi.add(ProgressBar::new_spinner());
    log_window.set_style(ProgressStyle::default_spinner().template("{msg}").unwrap());
    log_window.set_message("Starting upgrade...");

    // Get current version for comparison
    let current_version = run_command("/usr/local/bin/lh --version 2>&1 || echo 'unknown'")
        .unwrap_or_else(|_| "unknown".to_string());
    info!("Current version: {}", current_version.trim());
    println!("Current version: {}", current_version.trim());

    // Phase 1: Get binary (either from local path or download)
    let binary_path = if let Some(path) = from_path {
        pb.set_message("Using local binary...");
        log_window.set_message(format!("Using binary from: {}", path));

        // Verify the file exists
        if !std::path::Path::new(path).exists() {
            pb.finish_with_message("❌ Binary not found");
            anyhow::bail!("Binary not found at path: {}", path);
        }

        path.to_string()
    } else {
        pb.set_message("Downloading new binary...");
        match download_binary(version, Some(&log_window)) {
            Ok(path) => path,
            Err(e) => {
                pb.finish_with_message("❌ Download failed");
                return Err(e);
            }
        }
    };
    pb.inc(1);

    // Phase 2: Install new binary
    pb.set_message("Installing new binary...");
    if let Err(e) = install_binary(&binary_path, Some(&log_window)) {
        pb.finish_with_message("❌ Installation failed");
        if from_path.is_none() {
            cleanup_temp_dir(&binary_path).ok();
        }
        return Err(e);
    }
    pb.inc(1);

    // Get new version
    let new_version = run_command("/usr/local/bin/lh --version 2>&1 || echo 'unknown'")
        .unwrap_or_else(|_| "unknown".to_string());
    info!("New version: {}", new_version.trim());

    // Cleanup temp directory (only if we downloaded)
    if from_path.is_none() {
        cleanup_temp_dir(&binary_path).ok();
    }

    // Phase 3: Pull the matching litehouse-server image from GHCR
    pb.set_message("Pulling litehouse container image...");
    log_window.set_message("Pulling new container image...");

    let litehouse_uid = get_litehouse_uid().context("Failed to get litehouse user UID")?;
    let pull_version = parse_version(&new_version);

    let log_window_clone = log_window.clone();
    let pull_result = tokio::task::spawn_blocking(move || {
        phase6a_pull_litehouse_image(&pull_version, Some(&log_window_clone))
    })
    .await
    .map_err(|e| anyhow::anyhow!("Image pull task panicked: {}", e))?;

    let image_tag = match pull_result {
        Ok(tag) => tag,
        Err(e) => {
            pb.finish_with_message("❌ Image pull failed");
            return Err(e);
        }
    };
    pb.inc(1);

    // Phase 4: Restart litehouse-server container
    pb.set_message("Restarting litehouse-server container...");
    log_window.set_message("Restarting container...");

    if let Err(e) = phase9b_start_litehouse_container(&litehouse_uid, &image_tag) {
        pb.finish_with_message("❌ Container restart failed");
        return Err(e);
    }
    pb.inc(1);

    pb.finish_with_message("✅ Upgrade completed successfully!");
    log_window.finish_and_clear();

    // Print success message
    println!("\n{}", "=".repeat(60));
    println!("✓ litehouse upgraded successfully");
    println!("  Previous: {}", current_version.trim());
    println!("  Current:  {}", new_version.trim());
    println!("{}", "=".repeat(60));

    Ok(())
}
