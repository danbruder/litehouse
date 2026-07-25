//! Client-side "create an app and wire up its GitHub deploy workflow" flow,
//! shared by `lh create` (CLI) and the MCP `create_app` tool. Registers the
//! app on the server, sets the `LITEHOUSE_DEPLOY_TOKEN` repo secret, and
//! commits `.github/workflows/litehouse-deploy.yml`.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::api_client::ApiClient;
use crate::config::ClientConfig;

/// Result of a successful provision — a machine-readable summary suitable for
/// JSON output (CLI `--json`, MCP tool result).
#[derive(Debug, Serialize)]
pub struct ProvisionOutcome {
    pub name: String,
    pub url: String,
    pub repo: String,
    pub workflow_committed: bool,
}

/// Register `app_name` on the server (linked to `repo`), then set the deploy
/// secret and commit the deploy workflow to the repo. `repo` may be `None` to
/// infer "owner/name" from the current directory's `origin` git remote.
///
/// `allow_interactive` is passed straight to GitHub token resolution: `false`
/// (the MCP / `--json` path) never blocks on a device-flow prompt and returns
/// a descriptive error if no token is already available.
pub async fn provision_app(
    api_client: &ApiClient,
    config: &ClientConfig,
    app_name: &str,
    repo: Option<String>,
    rotate_token: bool,
    allow_interactive: bool,
) -> Result<ProvisionOutcome> {
    let repo = match repo {
        Some(r) => r,
        None => infer_repo_from_git()?,
    };

    let (owner, repo_name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("repo must be in 'owner/name' form, got '{}'", repo))?;

    let create_result = match api_client.create_app(app_name, Some(&repo), rotate_token).await {
        Ok(r) => r,
        Err(e) if !rotate_token && e.to_string().contains("already exists") => {
            return Err(anyhow!(
                "App '{}' already exists. Pass rotate_token=true to re-link it \
                 (mints a fresh deploy token and re-commits the deploy workflow).",
                app_name
            ));
        }
        Err(e) => return Err(e),
    };

    // The server's base_url already ends in /api; the deploy hook lives at
    // /api/hooks/deploy alongside the rest of the admin API.
    let hook_url = format!("{}/hooks/deploy", config.base_url.trim_end_matches('/'));

    let setup = async {
        let token =
            crate::commands::github_login::resolve_github_token(allow_interactive).await?;
        crate::github::actions::put_actions_secret(
            &token,
            owner,
            repo_name,
            "LITEHOUSE_DEPLOY_TOKEN",
            &create_result.deploy_token,
        )
        .await
        .context("setting LITEHOUSE_DEPLOY_TOKEN secret")?;

        let workflow =
            crate::workflow::render_deploy_workflow(owner, repo_name, app_name, &hook_url);
        crate::github::actions::put_file(
            &token,
            owner,
            repo_name,
            ".github/workflows/litehouse-deploy.yml",
            &workflow,
            "Add litehouse deploy workflow",
        )
        .await
        .context("committing .github/workflows/litehouse-deploy.yml")?;

        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(e) = setup {
        // The app already exists on the server at this point — say so, and
        // surface the most common cause (a token without `workflow` scope).
        return Err(anyhow!(
            "App '{}' was created on the server, but setting up the GitHub workflow for {} \
             failed: {:#}\nHint: committing workflow files needs a GitHub token with the \
             `workflow` scope (e.g. `gh auth refresh -h github.com -s workflow`).",
            app_name,
            repo,
            e
        ));
    }

    Ok(ProvisionOutcome {
        name: create_result.name,
        url: create_result.url,
        repo,
        workflow_committed: true,
    })
}

/// Infer "owner/name" from the `origin` git remote in the current directory.
/// Supports both GitHub HTTPS and SSH remote URL forms.
pub fn infer_repo_from_git() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("running `git remote get-url origin`")?;

    if !output.status.success() {
        return Err(anyhow!(
            "Could not find a git remote named 'origin' in the current directory. \
             Pass the repo explicitly as 'owner/name'."
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (owner, repo) = crate::github::api::parse_repo_url(&url).map_err(|_| {
        anyhow!(
            "The 'origin' remote ('{}') is not a github.com repo. Pass the repo explicitly \
             as 'owner/name'.",
            url
        )
    })?;

    Ok(format!("{}/{}", owner, repo))
}
