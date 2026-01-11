use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use tracing::{info, instrument};
use indicatif::ProgressBar;

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
    execute_remote_with_log(target, command, None)
}

/// Execute a command on the remote server via SSH with optional log window
#[instrument(skip(log_window))]
pub fn execute_remote_with_log(
    target: &str,
    command: &str,
    log_window: Option<&ProgressBar>,
) -> Result<String> {
    info!("Executing remote command: {}", command);

    let mut child = Command::new("ssh")
        .args([target, command])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to execute SSH command")?;

    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

    let log_buffer: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::with_capacity(20)));
    let all_output = Arc::new(Mutex::new(Vec::new()));

    // Clone Arcs for threads
    let log_buffer_stdout = Arc::clone(&log_buffer);
    let log_buffer_stderr = Arc::clone(&log_buffer);
    let all_output_stdout = Arc::clone(&all_output);
    let all_output_stderr = Arc::clone(&all_output);

    // Spawn thread to read stdout
    let log_window_stdout = log_window.map(|pb| pb.clone());
    let stdout_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                // Add to all output
                all_output_stdout.lock().unwrap().push(line.clone());

                // Update log buffer (keep last 20 lines)
                let mut buffer = log_buffer_stdout.lock().unwrap();
                if buffer.len() >= 20 {
                    buffer.pop_front();
                }
                buffer.push_back(line.clone());

                // Update log window if provided
                if let Some(ref pb) = log_window_stdout {
                    let log_display: Vec<String> = buffer.iter().cloned().collect();
                    pb.set_message(log_display.join("\n"));
                }
            }
        }
    });

    // Spawn thread to read stderr
    let log_window_stderr = log_window.map(|pb| pb.clone());
    let stderr_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                // Add to all output
                all_output_stderr.lock().unwrap().push(line.clone());

                // Update log buffer (keep last 20 lines)
                let mut buffer = log_buffer_stderr.lock().unwrap();
                if buffer.len() >= 20 {
                    buffer.pop_front();
                }
                buffer.push_back(format!("STDERR: {}", line));

                // Update log window if provided
                if let Some(ref pb) = log_window_stderr {
                    let log_display: Vec<String> = buffer.iter().cloned().collect();
                    pb.set_message(log_display.join("\n"));
                }
            }
        }
    });

    // Wait for threads to finish
    stdout_handle.join().unwrap();
    stderr_handle.join().unwrap();

    // Wait for command to finish
    let status = child.wait().context("Failed to wait for SSH command")?;

    if !status.success() {
        let all_lines = all_output.lock().unwrap();
        let output_str = all_lines.join("\n");
        anyhow::bail!(
            "Remote command failed:\nCommand: {}\nOutput: {}",
            command,
            output_str
        );
    }

    // Return all output
    let all_lines = all_output.lock().unwrap();
    Ok(all_lines.join("\n"))
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
