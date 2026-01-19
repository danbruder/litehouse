use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use indicatif::ProgressBar;
use tracing::{info, instrument};

/// Execute a command locally
#[instrument]
pub fn run_command(cmd: &str) -> Result<String> {
    run_command_with_log(cmd, None)
}

/// Execute a command locally with optional log window
#[instrument(skip(log_window))]
pub fn run_command_with_log(cmd: &str, log_window: Option<&ProgressBar>) -> Result<String> {
    run_command_with_timeout(cmd, log_window, 300) // 5 minute default timeout
}

/// Execute a command locally with optional log window and configurable timeout
#[instrument(skip(log_window))]
pub fn run_command_with_timeout(
    cmd: &str,
    log_window: Option<&ProgressBar>,
    timeout_secs: u64,
) -> Result<String> {
    info!("Executing local command: {}", cmd);

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to execute command")?;

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

    // Wait for command to finish with timeout
    let child_id = child.id();
    let start_time = std::time::Instant::now();
    let timeout_duration = std::time::Duration::from_secs(timeout_secs);

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start_time.elapsed() > timeout_duration {
                    // Kill the process
                    let _ = child.kill();
                    let _ = child.wait(); // Reap the process

                    // Wait for threads to finish (they should complete once the process is killed)
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();

                    anyhow::bail!(
                        "Command timed out after {} seconds (pid {}). Command: {}",
                        timeout_secs,
                        child_id,
                        cmd
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                anyhow::bail!("Failed to wait for command: {}", e);
            }
        }
    };

    // Wait for threads to finish
    stdout_handle.join().unwrap();
    stderr_handle.join().unwrap();

    if !status.success() {
        let all_lines = all_output.lock().unwrap();
        let output_str = all_lines.join("\n");
        anyhow::bail!(
            "Command failed:\nCommand: {}\nOutput: {}",
            cmd,
            output_str
        );
    }

    // Return all output
    let all_lines = all_output.lock().unwrap();
    Ok(all_lines.join("\n"))
}

/// Execute a command as a specific user (using sudo -u)
#[instrument]
pub fn run_as_user(user: &str, cmd: &str) -> Result<String> {
    run_as_user_with_log(user, cmd, None)
}

/// Execute a command as a specific user with optional log window
#[instrument(skip(log_window))]
pub fn run_as_user_with_log(user: &str, cmd: &str, log_window: Option<&ProgressBar>) -> Result<String> {
    let sudo_command = format!("sudo -u {} bash -c '{}'", user, cmd.replace("'", "'\\''"));
    run_command_with_log(&sudo_command, log_window)
}

/// Write content to a file (requires appropriate permissions)
#[instrument(skip(content))]
pub fn write_file(path: &str, content: &str) -> Result<()> {
    info!("Writing file: {}", path);
    std::fs::write(path, content).context(format!("Failed to write file: {}", path))?;
    Ok(())
}

/// Write content to a file using sudo
#[instrument(skip(content))]
pub fn sudo_write_file(path: &str, content: &str) -> Result<()> {
    info!("Writing file (sudo): {}", path);

    // Create temp file, write content, then sudo mv
    let temp_dir = tempfile::tempdir()?;
    let temp_file = temp_dir.path().join("install_temp");
    std::fs::write(&temp_file, content)?;

    run_command(&format!("sudo mv {} {}", temp_file.display(), path))?;

    Ok(())
}

/// Check if a command exists locally
#[instrument]
pub fn command_exists(command: &str) -> bool {
    Command::new("which")
        .arg(command)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a file or directory exists
#[instrument]
pub fn path_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Check if running as root
pub fn is_root() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_command() {
        let output = run_command("echo 'hello world'").unwrap();
        assert!(output.contains("hello world"));
    }

    #[test]
    fn test_command_exists() {
        assert!(command_exists("sh"));
        assert!(!command_exists("nonexistent_command_12345"));
    }
}
