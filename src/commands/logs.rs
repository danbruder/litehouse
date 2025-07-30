use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::thread;
use std::time::Duration;
use tracing::instrument;

use crate::config;
use crate::db;

/// View app logs
#[instrument]
pub async fn execute(app_name: &str, lines: usize, follow: bool) -> Result<()> {
    // Connect to database
    let pool = db::init_pool().await?;

    // Check if app exists
    let _ = db::app::get_by_name(&pool, app_name)
        .await?
        .ok_or_else(|| anyhow!("App '{}' not found", app_name))?;

    // Get log file path
    let log_path = config::get_app_log_path(app_name)?;

    // Check if log file exists
    if !log_path.exists() {
        return Err(anyhow!("Log file not found for app '{}'", app_name));
    }

    // Read last N lines
    let file = File::open(&log_path)
        .context(format!("Failed to open log file: {}", log_path.display()))?;

    let mut reader = BufReader::new(file);

    // Get file size
    let file_size = reader.seek(SeekFrom::End(0))?;

    // If file is empty, return
    if file_size == 0 {
        println!("No logs found for app '{}'", app_name);

        if follow {
            println!("Waiting for logs...");
            return follow_logs(&log_path);
        }

        return Ok(());
    }

    // Read last N lines
    let mut buffer = Vec::new();
    let mut position = file_size;
    let mut line_count = 0;

    while position > 0 && line_count < lines {
        // Move back one byte
        position -= 1;
        reader.seek(SeekFrom::Start(position))?;

        // Read one byte
        let mut byte = [0];
        reader.read_exact(&mut byte)?;

        // Check for newline
        if byte[0] == b'\n' && position < file_size - 1 {
            line_count += 1;
        }

        // Add byte to buffer (in reverse order)
        buffer.push(byte[0]);
    }

    // Reverse buffer to get lines in correct order
    buffer.reverse();

    // Skip initial newline if present
    let start_pos = if !buffer.is_empty() && buffer[0] == b'\n' {
        1
    } else {
        0
    };

    // Convert buffer to string and print
    let log_str = String::from_utf8_lossy(&buffer[start_pos..]);
    print!("{}", log_str);

    // Follow logs if requested
    if follow {
        // Seek to end of file
        reader.seek(SeekFrom::End(0))?;

        println!("Following logs (press Ctrl+C to stop)...");

        // Read new lines as they are added
        let mut line = String::new();
        loop {
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // No new data, sleep and try again
                    thread::sleep(Duration::from_millis(200));
                }
                Ok(_) => {
                    // Print new line
                    print!("{}", line);
                    line.clear();
                }
                Err(e) => {
                    return Err(anyhow!("Error reading log file: {}", e));
                }
            }
        }
    }

    Ok(())
}

/// Follow logs
fn follow_logs(log_path: &std::path::Path) -> Result<()> {
    let file =
        File::open(log_path).context(format!("Failed to open log file: {}", log_path.display()))?;

    let mut reader = BufReader::new(file);

    // Seek to end of file
    reader.seek(SeekFrom::End(0))?;

    tracing::info!("Following logs (press Ctrl+C to stop)...");

    // Read new lines as they are added
    let mut line = String::new();
    loop {
        match reader.read_line(&mut line) {
            Ok(0) => {
                // No new data, sleep and try again
                thread::sleep(Duration::from_millis(200));
            }
            Ok(_) => {
                // Print new line
                print!("{}", line);
                line.clear();
            }
            Err(e) => {
                return Err(anyhow!("Error reading log file: {}", e));
            }
        }
    }
}
