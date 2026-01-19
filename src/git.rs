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
    #[error("Config error: {0}")]
    ConfigError(#[from] crate::config::ConfigError),
}

pub type GitResult<T> = Result<T, GitError>;

pub struct GitPullResult {
    pub commit: String,
}

#[instrument(skip(token))]
pub async fn pull(
    Remote {
        name,
        remote,
        branch,
        ..
    }: &Remote,
    build_dir: &Path,
    token: Option<&str>,
) -> GitResult<GitPullResult> {
    info!(
        "Starting git pull for remote: {}, branch: {}, build_dir: {}",
        name, branch, build_dir.display()
    );

    // Inject token into URL if provided and it's a GitHub HTTPS URL
    let pull_url = match token {
        Some(t) if remote.starts_with("https://github.com/") => inject_github_token(remote, t),
        _ => remote.to_string(),
    };

    // Change to the git directory
    let output = Command::new("git")
        .args(["-C", build_dir.to_str().unwrap(), "pull", &pull_url, branch])
        .output()?;

    if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        // Check if it's an auth error and provide helpful message
        if error_msg.contains("could not read Username") || error_msg.contains("Authentication failed") {
            return Err(GitError::GitPullError(
                "Authentication failed. Please connect your GitHub account using 'lh github connect' or ensure the repository is public.".to_string()
            ));
        }
        return Err(GitError::GitPullError(format!("git pull failed: {}", error_msg)));
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

/// Inject a GitHub token into an HTTPS GitHub URL
/// Converts `https://github.com/user/repo` to `https://x-access-token:TOKEN@github.com/user/repo`
fn inject_github_token(url: &str, token: &str) -> String {
    if url.starts_with("https://github.com/") {
        url.replace("https://github.com/", &format!("https://x-access-token:{}@github.com/", token))
    } else {
        url.to_string()
    }
}

#[instrument(skip(token))]
pub async fn clone(remote: &str, build_dir: &Path, token: Option<&str>) -> GitResult<()> {
    info!("Cloning remote: {}", remote);

    let clone_url = match token {
        Some(t) if remote.starts_with("https://github.com/") => inject_github_token(remote, t),
        _ => remote.to_string(),
    };

    let output = Command::new("git")
        .args(["clone", &clone_url, build_dir.to_str().unwrap()])
        .output()?;

    if !output.status.success() {
        let error_msg = String::from_utf8_lossy(&output.stderr);
        // Check if it's an auth error and provide helpful message
        if error_msg.contains("could not read Username") || error_msg.contains("Authentication failed") {
            return Err(GitError::GitCloneError(
                "Authentication failed. Please connect your GitHub account using 'lh github connect' or ensure the repository is public.".to_string()
            ));
        }
        return Err(GitError::GitCloneError(format!("git clone failed: {}", error_msg)));
    }

    info!("Git clone completed successfully");

    Ok(())
}   