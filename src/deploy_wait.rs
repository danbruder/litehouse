//! Waiting for a deploy to settle — the verification primitive behind
//! `lh deploys --wait` and the MCP `list_deploys` tool (`wait: true`).
//!
//! Without a sha it waits on the newest deploy. That is only right for an
//! app's very first deploy: just after a `git push` the newest deploy is
//! still the *previous* one, so the caller would see its old result. With a
//! sha it waits for the deploy of that commit — and, when a GitHub token is
//! available, watches that commit's Actions run so a failed image build is
//! reported as soon as it fails instead of as a timeout minutes later.

use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

use crate::api_client::{ApiClient, DeployListItem};
use crate::github::actions::WorkflowRun;

/// How often the GitHub Actions run is checked while no deploy for the
/// commit has been registered yet.
const BUILD_CHECK_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum Outcome {
    Succeeded(DeployListItem),
    Failed(DeployListItem),
    /// The commit's GitHub Actions run finished without succeeding, so no
    /// deploy for it will ever be registered.
    BuildFailed(WorkflowRun),
    TimedOut {
        deploy: Option<DeployListItem>,
        build: Option<WorkflowRun>,
        /// Whether GitHub was actually asked about the commit's build.
        checked_build: bool,
    },
}

impl Outcome {
    /// Exit code for `lh deploys --wait`: 0 success, 1 failure, 2 timeout.
    pub fn exit_code(&self) -> i32 {
        match self {
            Outcome::Succeeded(_) => 0,
            Outcome::Failed(_) | Outcome::BuildFailed(_) => 1,
            Outcome::TimedOut { .. } => 2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Succeeded(_) => "succeeded",
            Outcome::Failed(_) => "failed",
            Outcome::BuildFailed(_) => "build_failed",
            Outcome::TimedOut { .. } => "timed_out",
        }
    }

    /// One human-readable line (or two) describing the outcome.
    pub fn message(&self, app: &str, sha: Option<&str>, timeout_secs: u64) -> String {
        let what = match sha {
            Some(s) => format!("a deploy of {} for app '{}'", short(s), app),
            None => format!("a deploy for app '{}'", app),
        };
        match self {
            Outcome::Succeeded(d) => format!("Deploy {} succeeded", d.id),
            Outcome::Failed(d) => format!(
                "Deploy {} failed: {}",
                d.id,
                d.error.as_deref().unwrap_or("unknown error")
            ),
            Outcome::BuildFailed(run) => format!(
                "GitHub Actions run for {} failed ({}), so no deploy was registered.\n\
                 Build log: {}  (or `gh run view {} --log-failed`)",
                short(&run.head_sha),
                run.conclusion.as_deref().unwrap_or("unknown"),
                run.html_url,
                run.id
            ),
            Outcome::TimedOut { deploy: Some(_), .. } => format!(
                "Timed out after {}s waiting for deploy to finish (still in_progress)",
                timeout_secs
            ),
            Outcome::TimedOut { deploy: None, build, checked_build } => {
                let mut msg = format!(
                    "Timed out after {}s waiting for {} to be registered",
                    timeout_secs, what
                );
                match build {
                    Some(run) => msg.push_str(&format!(
                        "\nGitHub Actions run is {}: {}",
                        run.status, run.html_url
                    )),
                    None if sha.is_some() && *checked_build => msg.push_str(
                        "\nNo GitHub Actions run found for that commit — was it pushed to main/master, \
                         and does the repo have .github/workflows/litehouse-deploy.yml?",
                    ),
                    None => msg.push_str(
                        "\nCheck the build: gh run list --workflow litehouse-deploy.yml",
                    ),
                }
                msg
            }
        }
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// Where to look for the commit's GitHub Actions run.
pub struct BuildWatch {
    token: String,
    owner: String,
    repo: String,
}

impl BuildWatch {
    /// Best effort: needs the app's linked repo (from the server) and a GitHub
    /// token that can be found without prompting. `None` otherwise — waiting
    /// still works, a failed build just surfaces as a timeout.
    pub async fn discover(api: &ApiClient, app: &str) -> Option<Self> {
        let repo = api.get_app_repo(app).await.ok().flatten()?;
        let (owner, repo) = repo.split_once('/')?;
        let token = crate::commands::github_login::resolve_github_token(false)
            .await
            .ok()?;
        Some(Self {
            token,
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    async fn run_for(&self, sha: &str) -> Result<Option<WorkflowRun>> {
        crate::github::actions::deploy_run_for_sha(&self.token, &self.owner, &self.repo, sha).await
    }
}

/// Turn what the caller passed (a full or short sha, or a git ref like
/// `HEAD` / `main`) into a sha to match deploys against. Refs are resolved
/// with `git rev-parse` in the current directory.
pub fn resolve_sha(input: &str) -> Result<String> {
    let input = input.trim();
    let parsed = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &format!("{input}^{{commit}}")])
        .output();
    if let Ok(out) = parsed
        && out.status.success()
    {
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !sha.is_empty() {
            return Ok(sha);
        }
    }
    if is_sha_like(input) {
        return Ok(input.to_lowercase());
    }
    Err(anyhow!(
        "'{}' is not a commit sha, and not a ref in a git repo in the current directory",
        input
    ))
}

fn is_sha_like(s: &str) -> bool {
    (4..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The deploy being waited on: the newest one for `sha` (deploys arrive
/// newest first), or simply the newest one when no sha is given.
fn target<'a>(deploys: &'a [DeployListItem], sha: Option<&str>) -> Option<&'a DeployListItem> {
    match sha {
        None => deploys.first(),
        Some(sha) => {
            let sha = sha.to_lowercase();
            deploys.iter().find(|d| {
                d.git_sha.as_deref().is_some_and(|g| {
                    let g = g.to_lowercase();
                    g.starts_with(&sha) || (g.len() >= 7 && sha.starts_with(&g))
                })
            })
        }
    }
}

/// Poll until the target deploy settles, its build fails, or `timeout_secs`
/// pass. Also returns the last deploy list fetched, for display.
pub async fn wait(
    api: &ApiClient,
    app: &str,
    sha: Option<&str>,
    limit: u32,
    timeout_secs: u64,
    build: Option<&BuildWatch>,
) -> Result<(Outcome, Vec<DeployListItem>)> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_build_check: Option<Instant> = None;
    let mut last_run: Option<WorkflowRun> = None;
    let mut checked_build = false;

    loop {
        let deploys = api.list_deploys(app, limit).await?;
        let current = target(&deploys, sha).cloned();

        if let Some(d) = &current {
            if d.status != "in_progress" {
                let outcome = if d.status == "succeeded" {
                    Outcome::Succeeded(d.clone())
                } else {
                    Outcome::Failed(d.clone())
                };
                return Ok((outcome, deploys));
            }
        } else if let (Some(sha), Some(build)) = (sha, build) {
            // No deploy for this commit yet: the image may still be building.
            let due = last_build_check.is_none_or(|t| t.elapsed() >= BUILD_CHECK_INTERVAL);
            if due {
                last_build_check = Some(Instant::now());
                match build.run_for(sha).await {
                    Ok(Some(run)) if run.failed() => {
                        return Ok((Outcome::BuildFailed(run), deploys));
                    }
                    Ok(run) => {
                        checked_build = true;
                        last_run = run;
                    }
                    Err(e) => tracing::debug!("checking GitHub Actions run failed: {e:#}"),
                }
            }
        }

        if Instant::now() >= deadline {
            let outcome = Outcome::TimedOut {
                deploy: current,
                build: last_run,
                checked_build,
            };
            return Ok((outcome, deploys));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deploy(id: &str, sha: Option<&str>, status: &str) -> DeployListItem {
        DeployListItem {
            id: id.to_string(),
            image: "img".to_string(),
            git_sha: sha.map(str::to_string),
            status: status.to_string(),
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn without_sha_targets_newest_deploy() {
        let deploys = vec![deploy("2", Some("bbb"), "succeeded"), deploy("1", Some("aaa"), "failed")];
        assert_eq!(target(&deploys, None).unwrap().id, "2");
    }

    #[test]
    fn with_sha_ignores_the_previous_deploy() {
        // Right after a push the newest deploy is the old commit's; it must
        // not be mistaken for the new one.
        let deploys = vec![deploy("1", Some("aaaaaaaaaa"), "succeeded")];
        assert!(target(&deploys, Some("bbbbbbb")).is_none());
    }

    #[test]
    fn sha_matches_by_prefix_either_way_and_case_insensitively() {
        let full = "ABCDEF1234567890abcdef1234567890abcdef12";
        let deploys = vec![deploy("1", Some(full), "in_progress")];
        assert_eq!(target(&deploys, Some("abcdef1")).unwrap().id, "1");
        let short = vec![deploy("2", Some("abcdef1"), "succeeded")];
        assert_eq!(target(&short, Some(&full.to_lowercase())).unwrap().id, "2");
    }

    #[test]
    fn deploys_without_sha_never_match_a_sha() {
        let deploys = vec![deploy("1", None, "succeeded")];
        assert!(target(&deploys, Some("abcdef1")).is_none());
    }

    #[test]
    fn resolve_sha_accepts_hex_and_rejects_nonsense() {
        assert_eq!(resolve_sha("ABCDEF1").unwrap(), "abcdef1");
        assert!(resolve_sha("definitely-not-a-ref-xyz").is_err());
    }

    /// Serve a scripted sequence of deploy lists at `/api/apps/app/deploys`
    /// (one per request, repeating the last) and return an ApiClient for it.
    async fn scripted_server(responses: Vec<serde_json::Value>) -> ApiClient {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let calls = Arc::new(AtomicUsize::new(0));
        let responses = Arc::new(responses);
        let app = axum::Router::new().route(
            "/api/apps/app/deploys",
            axum::routing::get(move || {
                let calls = calls.clone();
                let responses = responses.clone();
                async move {
                    let i = calls.fetch_add(1, Ordering::SeqCst).min(responses.len() - 1);
                    axum::Json(responses[i].clone())
                }
            }),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::Server::from_tcp(listener).unwrap().serve(app.into_make_service()));

        ApiClient::new(crate::config::ClientConfig {
            base_url: format!("http://{addr}/api"),
            api_token: Some("t".to_string()),
            github_token: None,
        })
    }

    fn json_deploy(id: &str, sha: &str, status: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "image": "img", "git_sha": sha, "status": status,
            "error": null, "created_at": "", "updated_at": ""
        })
    }

    // The bug this module exists to fix: right after a push, the previous
    // deploy is newest and already succeeded. Waiting on the new sha must
    // hold until the new commit's own deploy settles.
    #[tokio::test]
    async fn waits_for_the_pushed_commit_not_the_previous_deploy() {
        let old = json_deploy("1", "aaaaaaa111", "succeeded");
        let api = scripted_server(vec![
            serde_json::json!([old.clone()]),
            serde_json::json!([json_deploy("2", "bbbbbbb222", "in_progress"), old.clone()]),
            serde_json::json!([json_deploy("2", "bbbbbbb222", "failed"), old]),
        ])
        .await;

        let (outcome, _) = wait(&api, "app", Some("bbbbbbb"), 20, 30, None).await.unwrap();
        match outcome {
            Outcome::Failed(d) => assert_eq!(d.id, "2"),
            other => panic!("expected the new deploy's failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn times_out_when_the_commit_never_deploys() {
        let api = scripted_server(vec![serde_json::json!([json_deploy("1", "aaaaaaa", "succeeded")])]).await;
        let (outcome, _) = wait(&api, "app", Some("bbbbbbb"), 20, 0, None).await.unwrap();
        assert!(matches!(outcome, Outcome::TimedOut { deploy: None, .. }));
        assert_eq!(outcome.exit_code(), 2);
    }

    #[test]
    fn exit_codes_follow_the_documented_contract() {
        assert_eq!(Outcome::Succeeded(deploy("1", None, "succeeded")).exit_code(), 0);
        assert_eq!(Outcome::Failed(deploy("1", None, "failed")).exit_code(), 1);
        let timed_out = Outcome::TimedOut { deploy: None, build: None, checked_build: false };
        assert_eq!(timed_out.exit_code(), 2);
        // Never claim "no run found" when GitHub wasn't asked.
        assert!(!timed_out.message("app", Some("abc1234"), 5).contains("No GitHub Actions run found"));
        let checked = Outcome::TimedOut { deploy: None, build: None, checked_build: true };
        assert!(checked.message("app", Some("abc1234"), 5).contains("No GitHub Actions run found"));
    }
}
