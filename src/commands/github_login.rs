//! GitHub token resolution for `lh create`'s workflow/secret setup, plus the
//! explicit `lh github login` device-flow command.
//!
//! Resolution is ordered to be agent/CI-friendly first, human-fallback last:
//! an automated caller sets `$GITHUB_TOKEN` or has `gh` pre-authenticated,
//! so neither of those requires any interaction. Only a human sitting at a
//! terminal without either of those falls through to the device flow.

use crate::config::ClientConfig;
use anyhow::{anyhow, Context, Result};
use std::process::Command;

/// litehouse's registered GitHub OAuth App client id, used for the device
/// flow. Overridable via `GITHUB_CLIENT_ID` (e.g. to point at a different
/// registered app in dev).
const DEFAULT_GITHUB_CLIENT_ID: &str = "Ov23liTp4hQb5j4lzQfh";

fn client_id() -> String {
    std::env::var("GITHUB_CLIENT_ID").unwrap_or_else(|_| DEFAULT_GITHUB_CLIENT_ID.to_string())
}

/// Resolution order (agent-friendly first):
/// 1. `$GITHUB_TOKEN`
/// 2. `gh auth token` (if the `gh` CLI is installed and authenticated)
/// 3. stored client-config `github_token` (set by a prior `lh github login`)
/// 4. device flow (interactive) -> stores the result in the client config
///
/// When `allow_interactive` is false (e.g. `--json` mode, or any
/// non-interactive/CI context) step 4 is skipped and a descriptive error is
/// returned instead, listing every option concretely so the caller knows
/// exactly what to do.
pub async fn resolve_github_token(allow_interactive: bool) -> Result<String> {
    if let Some(token) = env_token() {
        return Ok(token);
    }

    if let Some(token) = gh_cli_token() {
        return Ok(token);
    }

    if let Some(token) = stored_token() {
        return Ok(token);
    }

    if allow_interactive {
        return device_flow_login().await;
    }

    Err(no_token_error())
}

fn no_token_error() -> anyhow::Error {
    anyhow!(
        "No GitHub token available and interactive login is disabled. Provide one of:\n\
         - export GITHUB_TOKEN=<token> (a GitHub PAT/App token with repo + workflow access)\n\
         - `gh auth login` (litehouse will use `gh auth token`)\n\
         - `lh github login` (device flow; stores the token in the client config for future runs)"
    )
}

fn env_token() -> Option<String> {
    let token = std::env::var("GITHUB_TOKEN").ok()?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn gh_cli_token() -> Option<String> {
    // Test-only escape hatch: unit tests run on developer/CI machines that
    // may have `gh` installed and authenticated, which would otherwise make
    // this resolution step nondeterministic. Real callers never set this.
    if std::env::var("LITEHOUSE_DISABLE_GH_CLI").is_ok() {
        return None;
    }

    let output = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn stored_token() -> Option<String> {
    let config = ClientConfig::load().ok()?;
    let token = config.github_token?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Run the GitHub device authorization flow interactively: print the
/// verification URL and user code, poll until the user authorizes (or the
/// flow times out/is denied), then persist the resulting token in the
/// client config so future runs skip straight to `stored_token`.
pub async fn device_flow_login() -> Result<String> {
    let client_id = client_id();

    let auth = crate::github::oauth::start_device_flow(&client_id)
        .await
        .context("starting GitHub device authorization flow")?;

    println!(
        "First, authenticate with GitHub. Open {} and enter code: {}",
        auth.verification_uri, auth.user_code
    );
    println!("Waiting for authorization...");

    let (token, _scope) = crate::github::oauth::poll_for_token(
        &client_id,
        &auth.device_code,
        auth.interval,
        auth.expires_in,
    )
    .await
    .context("waiting for GitHub device authorization")?;

    let mut config = ClientConfig::load().unwrap_or_default();
    config.github_token = Some(token.clone());
    config
        .save()
        .context("saving GitHub token to client config")?;

    println!("GitHub authorization successful; token stored.");

    Ok(token)
}

/// `lh github login`: explicitly run the device flow (ignoring
/// `$GITHUB_TOKEN` / `gh auth token`, since the user is asking litehouse
/// specifically to authenticate and remember a token).
pub async fn execute() -> Result<()> {
    device_flow_login().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env vars (and the on-disk client config they key off) are
    // process-global; serialize tests that touch GITHUB_TOKEN or
    // LITEHOUSE_DISABLE_GH_CLI to avoid cross-test interference. A poisoned
    // lock (from a prior test panicking mid-mutation) must not cascade into
    // spurious failures in every later test, so we recover the guard rather
    // than unwrap it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn setup_test_dirs() -> (tempfile::TempDir, tempfile::TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        crate::config::set_test_dirs(data_dir.path().to_path_buf(), config_dir.path().to_path_buf())
            .unwrap();
        (data_dir, config_dir)
    }

    /// Disable the `gh auth token` resolution step for the duration of a
    /// test. Without this, tests exercising later resolution steps
    /// (stored config, device flow error) would be nondeterministic on any
    /// machine (dev or CI) with `gh` installed and pre-authenticated.
    struct GhCliDisabled;
    impl GhCliDisabled {
        fn new() -> Self {
            unsafe { std::env::set_var("LITEHOUSE_DISABLE_GH_CLI", "1") };
            Self
        }
    }
    impl Drop for GhCliDisabled {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("LITEHOUSE_DISABLE_GH_CLI") };
        }
    }

    #[test]
    fn env_token_reads_github_token() {
        let _guard = lock_env();
        unsafe { std::env::set_var("GITHUB_TOKEN", "from-env") };
        assert_eq!(env_token(), Some("from-env".to_string()));
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
    }

    #[test]
    fn env_token_ignores_blank() {
        let _guard = lock_env();
        unsafe { std::env::set_var("GITHUB_TOKEN", "   ") };
        assert_eq!(env_token(), None);
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
    }

    #[test]
    fn env_token_none_when_unset() {
        let _guard = lock_env();
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        assert_eq!(env_token(), None);
    }

    #[test]
    fn stored_token_reads_client_config() {
        let _guard = lock_env();
        let (_data, _config) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.github_token = Some("from-config".to_string());
        config.save().unwrap();

        assert_eq!(stored_token(), Some("from-config".to_string()));
    }

    #[test]
    fn stored_token_none_when_unset() {
        let _guard = lock_env();
        let (_data, _config) = setup_test_dirs();
        let config = ClientConfig::default();
        config.save().unwrap();

        assert_eq!(stored_token(), None);
    }

    #[tokio::test]
    async fn resolve_github_token_prefers_env_over_everything() {
        let _guard = lock_env();
        let _gh_disabled = GhCliDisabled::new();
        let (_data, _config) = setup_test_dirs();
        let mut config = ClientConfig::default();
        config.github_token = Some("from-config".to_string());
        config.save().unwrap();

        unsafe { std::env::set_var("GITHUB_TOKEN", "from-env") };
        let token = resolve_github_token(false).await.unwrap();
        unsafe { std::env::remove_var("GITHUB_TOKEN") };

        assert_eq!(token, "from-env");
    }

    #[tokio::test]
    async fn resolve_github_token_falls_back_to_stored_config() {
        let _guard = lock_env();
        let _gh_disabled = GhCliDisabled::new();
        let (_data, _config) = setup_test_dirs();
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        let mut config = ClientConfig::default();
        config.github_token = Some("from-config".to_string());
        config.save().unwrap();

        let token = resolve_github_token(false).await.unwrap();
        assert_eq!(token, "from-config");
    }

    #[tokio::test]
    async fn resolve_github_token_errors_with_concrete_options_when_noninteractive() {
        let _guard = lock_env();
        let _gh_disabled = GhCliDisabled::new();
        let (_data, _config) = setup_test_dirs();
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        let config = ClientConfig::default();
        config.save().unwrap();

        let err = resolve_github_token(false)
            .await
            .expect_err("expected no-token error with gh cli and device flow both unavailable");
        let msg = err.to_string();
        assert!(msg.contains("GITHUB_TOKEN"));
        assert!(msg.contains("gh auth login"));
        assert!(msg.contains("lh github login"));
    }
}
