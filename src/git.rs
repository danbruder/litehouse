use anyhow::Result;
use std::process::Command;
use tracing::{info, instrument};
use std::path::Path;

use crate::models::Remote;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Git error: {0}")]
    GitError(String),
    #[error("Git clone error: {0}")]
    GitCloneError(String),
    #[error("Git pull error: {0}")]
    GitPullError(String),
    #[error("Command error: {0}")]
    CommandError(#[from] std::io::Error),
}

type GitResult<T> = Result<T, GitError>;

struct GitPullResult {
    pub commit: String,
}

#[instrument]
pub async fn pull(
    Remote {
        name,
        remote,
        branch,
        ..
    }: &Remote,
    build_dir: &Path,
) -> GitResult<GitPullResult> {
    info!(
        "Starting git pull for remote: {}, branch: {}, build_dir: {}",
        name, branch, build_dir.display()
    );

    // Change to the git directory
    let output = Command::new("git")
        .args(["-C", build_dir.to_str().unwrap(), "pull", remote, branch])
        .output()?;

    if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        dbg!(&error_msg);
        return Err(GitError::GitError(format!("git pull failed: {}", error_msg)));
    }

    info!("Git pull completed successfully");

    // Get the current commit hash
    let commit_output = Command::new("git")
        .args(["-C", build_dir.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()?;

    if !commit_output.status.success() {
        let error_msg = String::from_utf8_lossy(&commit_output.stderr);
        return Err(GitError::GitError(format!("Failed to get commit hash: {}", error_msg)));
    }

    let commit = String::from_utf8_lossy(&commit_output.stdout)
        .trim()
        .to_string();

    info!("Current commit: {}", commit);

    Ok(GitPullResult { commit })
}

#[instrument]
pub async fn clone(remote: &str, build_dir: &Path) -> GitResult<()> {
    info!("Cloning remote: {}", remote);

    let output = Command::new("git")
        .args(["clone", remote, build_dir.to_str().unwrap()])
        .output()?;

    if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::GitCloneError(format!("git clone failed: {}", error_msg)));
    }

    info!("Git clone completed successfully");

    Ok(())
}   