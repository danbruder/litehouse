use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracing::{info, instrument};

use crate::install::executor::run_command;
use crate::install::phases::{
    get_litehouse_uid, phase12_dial_stdio_cleanup_timer, phase13_weekly_reboot_timer,
    phase6a_pull_litehouse_image, phase9b_start_litehouse_container,
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

/// Target path for the installed `lh` binary.
const INSTALL_PATH: &str = "/usr/local/bin/lh";

/// Builds the shell commands that stage a new binary at `staging_path`
/// (on the same filesystem as `install_path`) and then atomically rename
/// it over `install_path`.
///
/// Renaming over a running executable is safe on Linux: `rename(2)` just
/// repoints the directory entry to the new inode, while any process that
/// already has the old binary open (e.g. the currently-running `lh
/// upgrade`) keeps running against the old inode until it exits. This
/// sidesteps `ETXTBSY` ("Text file busy"), which is what an in-place `cp`
/// over the running binary triggers.
fn stage_and_rename_commands(source_path: &str, install_path: &str, staging_path: &str) -> Vec<String> {
    vec![
        format!("cp '{}' '{}'", source_path, staging_path),
        format!("chmod +x '{}'", staging_path),
        format!("mv '{}' '{}'", staging_path, install_path),
    ]
}

/// Installs the new binary to /usr/local/bin/lh using a stage-then-atomic-
/// rename strategy so the currently-running `lh upgrade` process (which has
/// the old binary open) is never overwritten in place.
fn install_binary(binary_path: &str, log_window: Option<&ProgressBar>) -> Result<()> {
    if let Some(pb) = log_window {
        pb.set_message("Installing new binary...");
    }
    info!(
        "Installing binary from {} to {}",
        binary_path, INSTALL_PATH
    );

    let staging_path = format!("{}.new", INSTALL_PATH);

    // Backup current binary
    run_command(&format!(
        "cp {} {}.backup 2>/dev/null || true",
        INSTALL_PATH, INSTALL_PATH
    ))?;

    // Stage the new binary next to the target (same filesystem) and
    // atomically rename it into place.
    for cmd in stage_and_rename_commands(binary_path, INSTALL_PATH, &staging_path) {
        if let Err(e) = run_command(&cmd) {
            // Clean up any partially-staged file before bailing.
            run_command(&format!("rm -f '{}'", staging_path)).ok();
            return Err(e);
        }
    }

    // Verify it works
    match run_command(&format!("{} --version", INSTALL_PATH)) {
        Ok(version) => {
            info!("Installed version: {}", version.trim());
            // Remove backup
            run_command(&format!("rm -f {}.backup", INSTALL_PATH))?;
            Ok(())
        }
        Err(e) => {
            // Restore backup
            run_command(&format!(
                "mv {}.backup {} 2>/dev/null || true",
                INSTALL_PATH, INSTALL_PATH
            ))?;
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

    // Get the version reported by the *downloaded* binary directly (without
    // installing it yet), so we know which image tag to pull even though
    // the host binary install now happens last.
    let new_version = run_command(&format!("'{}' --version 2>&1 || echo 'unknown'", binary_path))
        .unwrap_or_else(|_| "unknown".to_string());
    info!("New version: {}", new_version.trim());

    // Phase 2: Pull the matching litehouse-server image from GHCR
    //
    // This is deliberately done *before* the host binary install (see
    // below). If the binary-install step ever fails partway through, we
    // still want the container running the new image - an upgrade that
    // updates the container but not the host CLI is far less bad than one
    // that aborts before the container is touched at all, which is what
    // used to leave the server stuck on an old image after a failed
    // install.
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
            if from_path.is_none() {
                cleanup_temp_dir(&binary_path).ok();
            }
            return Err(e);
        }
    };
    pb.inc(1);

    // Phase 3: Restart litehouse-server container
    pb.set_message("Restarting litehouse-server container...");
    log_window.set_message("Restarting container...");

    if let Err(e) = phase9b_start_litehouse_container(&litehouse_uid, &image_tag) {
        pb.finish_with_message("❌ Container restart failed");
        if from_path.is_none() {
            cleanup_temp_dir(&binary_path).ok();
        }
        return Err(e);
    }
    pb.inc(1);

    // Phase 3b: Re-apply maintenance timers. Idempotent (see phase12/13
    // doc comments) so every upgrade re-applies the latest unit-file
    // content rather than only setting it up once at install time - any
    // future edits to the cleanup cadence propagate on the next upgrade
    // instead of silently drifting from what's on disk.
    pb.set_message("Re-applying maintenance timers...");
    if let Err(e) = phase12_dial_stdio_cleanup_timer() {
        pb.finish_with_message("❌ Dial-stdio cleanup timer setup failed");
        if from_path.is_none() {
            cleanup_temp_dir(&binary_path).ok();
        }
        return Err(e);
    }
    if let Err(e) = phase13_weekly_reboot_timer() {
        pb.finish_with_message("❌ Weekly reboot timer setup failed");
        if from_path.is_none() {
            cleanup_temp_dir(&binary_path).ok();
        }
        return Err(e);
    }

    // Phase 4: Install the new host binary. The container is already
    // running the new image at this point, so a failure here is logged as
    // a warning rather than aborting the upgrade - the server itself is
    // already upgraded, and the operator can retry `lh upgrade` (or copy
    // the binary manually) without the container being stuck on the old
    // image.
    pb.set_message("Installing new binary on host...");
    let binary_install_warning = match install_binary(&binary_path, Some(&log_window)) {
        Ok(()) => None,
        Err(e) => {
            tracing::warn!("Host binary install failed (container already upgraded): {}", e);
            Some(e.to_string())
        }
    };
    pb.inc(1);

    // Cleanup temp directory (only if we downloaded)
    if from_path.is_none() {
        cleanup_temp_dir(&binary_path).ok();
    }

    if binary_install_warning.is_some() {
        pb.finish_with_message("⚠️  Upgrade completed with a warning");
    } else {
        pb.finish_with_message("✅ Upgrade completed successfully!");
    }
    log_window.finish_and_clear();

    // Print success message
    println!("\n{}", "=".repeat(60));
    if let Some(warning) = &binary_install_warning {
        println!("⚠ litehouse-server container upgraded, but host binary install failed");
        println!("  Error: {}", warning);
        println!("  The container is running the new version; re-run `lh upgrade` to retry the host binary.");
    } else {
        println!("✓ litehouse upgraded successfully");
    }
    println!("  Previous: {}", current_version.trim());
    println!("  Current:  {}", new_version.trim());
    println!("{}", "=".repeat(60));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_and_rename_stages_on_same_filesystem_and_renames_over_target() {
        let cmds = stage_and_rename_commands(
            "/tmp/new-lh",
            "/usr/local/bin/lh",
            "/usr/local/bin/lh.new",
        );

        assert_eq!(
            cmds,
            vec![
                "cp '/tmp/new-lh' '/usr/local/bin/lh.new'".to_string(),
                "chmod +x '/usr/local/bin/lh.new'".to_string(),
                "mv '/usr/local/bin/lh.new' '/usr/local/bin/lh'".to_string(),
            ]
        );

        // The staging path must live in the same directory as the install
        // path, otherwise the final `mv` could cross filesystems and stop
        // being an atomic rename.
        let staging_dir = std::path::Path::new(&cmds[0])
            .to_string_lossy()
            .to_string();
        assert!(staging_dir.contains("/usr/local/bin/"));
    }

    #[test]
    fn parse_version_extracts_trailing_semver() {
        assert_eq!(parse_version("lh 0.1.34"), "0.1.34");
        assert_eq!(parse_version("litehouse 1.2.3-beta"), "1.2.3-beta");
    }

    #[test]
    fn parse_version_falls_back_to_latest_on_garbage() {
        assert_eq!(parse_version("unknown"), "latest");
        assert_eq!(parse_version(""), "latest");
    }
}
