use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use tracing::{info, instrument};

/// Parse SSH target into user and host
#[instrument]
pub fn parse_ssh_target(target: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = target.split('@').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid SSH target format. Expected: user@host");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Test SSH connectivity
#[instrument]
pub fn test_ssh_connection(target: &str) -> Result<()> {
    info!("Testing SSH connection to {}", target);

    let output = Command::new("ssh")
        .args([
            "-o", "BatchMode=yes",
            "-o", "ConnectTimeout=5",
            target,
            "echo",
            "Connection successful"
        ])
        .output()
        .context("Failed to execute SSH command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("SSH connection failed: {}", stderr);
    }

    info!("SSH connection successful");
    Ok(())
}

/// Execute a command on the remote server via SSH
#[instrument]
pub fn execute_remote(target: &str, command: &str) -> Result<String> {
    info!("Executing remote command: {}", command);

    let output = Command::new("ssh")
        .args([target, command])
        .output()
        .context("Failed to execute SSH command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(
            "Remote command failed:\nCommand: {}\nSTDOUT: {}\nSTDERR: {}",
            command,
            stdout,
            stderr
        );
    }

    let result = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(result)
}

/// Execute a command on the remote server as a different user (using sudo -u)
#[instrument]
pub fn execute_remote_as_user(target: &str, user: &str, command: &str) -> Result<String> {
    let sudo_command = format!("sudo -u {} bash -c '{}'", user, command.replace("'", "'\\''"));
    execute_remote(target, &sudo_command)
}

/// Upload a file to the remote server via SCP
#[instrument(skip(local_path))]
pub fn upload_file<P: AsRef<Path>>(target: &str, local_path: P, remote_path: &str) -> Result<()> {
    let local_path_str = local_path.as_ref().to_string_lossy();
    info!("Uploading {} to {}:{}", local_path_str, target, remote_path);

    let scp_target = format!("{}:{}", target, remote_path);

    let output = Command::new("scp")
        .args([local_path.as_ref().to_str().unwrap(), &scp_target])
        .output()
        .context("Failed to execute SCP command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("SCP upload failed: {}", stderr);
    }

    info!("Upload successful");
    Ok(())
}

/// Upload file content (as string) to remote server
#[instrument(skip(content))]
pub fn upload_content(target: &str, content: &str, remote_path: &str) -> Result<()> {
    info!("Uploading content to {}:{}", target, remote_path);

    // Create a temporary file with the content
    let temp_dir = tempfile::tempdir()?;
    let temp_file = temp_dir.path().join("upload_temp");
    std::fs::write(&temp_file, content)?;

    // Upload the temporary file
    upload_file(target, &temp_file, remote_path)?;

    Ok(())
}

/// Check if a command exists on the remote server
#[instrument]
pub fn command_exists_remote(target: &str, command: &str) -> Result<bool> {
    let check_command = format!("command -v {} >/dev/null 2>&1 && echo 'exists'", command);
    match execute_remote(target, &check_command) {
        Ok(output) => Ok(output.trim() == "exists"),
        Err(_) => Ok(false),
    }
}

/// Check if a file or directory exists on the remote server
#[instrument]
pub fn path_exists_remote(target: &str, path: &str) -> Result<bool> {
    let check_command = format!("test -e {} && echo 'exists' || echo 'notexists'", path);
    let output = execute_remote(target, &check_command)?;
    Ok(output.trim() == "exists")
}

/// Check if user has sudo access on the remote server
#[instrument]
pub fn has_sudo_access(target: &str) -> Result<bool> {
    let check_command = "sudo -n true 2>/dev/null && echo 'has_sudo' || echo 'no_sudo'";
    match execute_remote(target, check_command) {
        Ok(output) => Ok(output.trim() == "has_sudo"),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_target() {
        let (user, host) = parse_ssh_target("root@192.168.1.1").unwrap();
        assert_eq!(user, "root");
        assert_eq!(host, "192.168.1.1");

        let result = parse_ssh_target("invalid");
        assert!(result.is_err());
    }
}
