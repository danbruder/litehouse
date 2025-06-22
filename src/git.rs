use anyhow::Result;
use std::process::Command;
use tracing::{info, instrument};

pub struct GitPullResult {
    pub commit: String,
}

#[instrument]
pub async fn pull(remote_name: &str, branch: &str, directory: &str) -> Result<GitPullResult> {
    info!("Starting git pull for remote: {}, branch: {}, directory: {}", remote_name, branch, directory);
    
    // Change to the git directory
    let output = Command::new("git")
        .args(["-C", directory, "pull", remote_name, branch])
        .output()?;
    
    if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("git pull failed: {}", error_msg));
    }
    
    info!("Git pull completed successfully");
    
    // Get the current commit hash
    let commit_output = Command::new("git")
        .args(["-C", directory, "rev-parse", "HEAD"])
        .output()?;
    
    if !commit_output.status.success() {
        let error_msg = String::from_utf8_lossy(&commit_output.stderr);
        return Err(anyhow::anyhow!("Failed to get commit hash: {}", error_msg));
    }
    
    let commit = String::from_utf8_lossy(&commit_output.stdout)
        .trim()
        .to_string();
    
    info!("Current commit: {}", commit);
    
    Ok(GitPullResult { commit })
}
