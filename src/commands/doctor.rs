//! `lh doctor`: preflight for deploying the repo in the current directory.
//! Checks, in order, everything an agent (or person) otherwise discovers one
//! failed step at a time: the server connection, a GitHub token that can
//! commit workflows, a GitHub `origin`, and a Dockerfile litehouse can route
//! to. Exits 1 if any check fails; `--json` for machine-readable output.

use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::api_client::ApiClient;
use crate::config::ClientConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    /// What to do about a warn/fail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Check {
    fn new(name: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self { name, status, detail: detail.into(), fix: None }
    }
    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub ok: bool,
    pub checks: Vec<Check>,
}

pub async fn execute(api: &ApiClient, config: &ClientConfig, app: Option<&str>, json: bool) -> Result<()> {
    let mut checks = vec![server_check(api, config).await, github_token_check().await];

    let repo_root = git_root();
    let origin = match &repo_root {
        Some(_) => github_origin(),
        None => None,
    };
    checks.push(match (&repo_root, &origin) {
        (None, _) => Check::new("git_repo", Status::Fail, "the current directory is not in a git repository")
            .fix("run lh doctor from the app's repo (git init, and push it to GitHub)"),
        (Some(_), None) => Check::new("git_repo", Status::Fail, "the 'origin' remote is missing or not a github.com repo")
            .fix("create the GitHub repo and add it as origin, e.g. `gh repo create <name> --private --source . --push`"),
        (Some(_), Some(repo)) => Check::new("git_repo", Status::Ok, format!("origin is github.com/{repo}")),
    });

    match &repo_root {
        Some(root) => checks.extend(dockerfile_checks(root)),
        None => checks.push(Check::new("dockerfile", Status::Skip, "not in a git repository")),
    }

    if let Some(app) = app {
        checks.push(app_check(api, app, origin.as_deref()).await);
    }

    let report = Report {
        ok: !checks.iter().any(|c| c.status == Status::Fail),
        checks,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }
    if !report.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_report(report: &Report) {
    for c in &report.checks {
        let mark = match c.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
            Status::Skip => "skip",
        };
        println!("[{mark}] {:<11} {}", c.name, c.detail);
        if let Some(fix) = &c.fix {
            println!("       {:<11} fix: {fix}", "");
        }
    }
    println!();
    println!(
        "{}",
        if report.ok { "Ready to deploy." } else { "Not ready: fix the FAIL items above and re-run `lh doctor`." }
    );
}

async fn server_check(api: &ApiClient, config: &ClientConfig) -> Check {
    if config.api_token.is_none() {
        return Check::new("server", Status::Fail, "no admin token configured").fix(format!(
            "run `lh connect https://admin.<your-domain> --token <TOKEN>`, or set {} and {}",
            crate::config::URL_ENV_VAR,
            crate::config::TOKEN_ENV_VAR
        ));
    }
    match api.get_server_info().await {
        Ok(info) => {
            let mut detail = format!("litehouse {} at {}", info.version, config.base_url);
            if let Some(domain) = info.domain {
                detail.push_str(&format!(", apps at https://<app>.{domain}"));
            }
            if let Some(platform) = info.platform {
                detail.push_str(&format!(", {platform}"));
            }
            Check::new("server", Status::Ok, detail)
        }
        Err(e) => Check::new("server", Status::Fail, format!("{} is not usable: {:#}", config.base_url, e))
            .fix("check the URL and admin token (`lh connect <url> --token <TOKEN>`)"),
    }
}

async fn github_token_check() -> Check {
    let token = match crate::commands::github_login::resolve_github_token(false).await {
        Ok(t) => t,
        Err(_) => {
            return Check::new("github", Status::Fail, "no GitHub token found ($GITHUB_TOKEN, `gh auth token`, or `lh github login`)")
                .fix("`gh auth login -s workflow`, or export GITHUB_TOKEN with repo + workflow access")
        }
    };
    let resp = reqwest::Client::new()
        .get("https://api.github.com/user")
        .bearer_auth(&token)
        .header("User-Agent", "litehouse")
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let scopes = r
                .headers()
                .get("x-oauth-scopes")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let login = r
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v["login"].as_str().map(str::to_string))
                .unwrap_or_else(|| "?".to_string());
            scope_check(&login, scopes.as_deref())
        }
        Ok(r) => Check::new("github", Status::Fail, format!("GitHub rejected the token ({})", r.status()))
            .fix("`gh auth login -s workflow`, or export a valid GITHUB_TOKEN"),
        Err(e) => Check::new("github", Status::Warn, format!("could not reach GitHub to check the token: {e}")),
    }
}

/// Classic/OAuth tokens (including `gh`'s) list their scopes in
/// `X-OAuth-Scopes`; committing `.github/workflows/*` needs `workflow`.
/// Fine-grained tokens send no header, so their permissions can't be read.
fn scope_check(login: &str, scopes_header: Option<&str>) -> Check {
    let Some(header) = scopes_header else {
        return Check::new("github", Status::Warn, format!("token for {login} (fine-grained; permissions not checkable)"))
            .fix("the token needs Contents, Secrets and Workflows write access on the app's repo");
    };
    let scopes: Vec<&str> = header.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let missing: Vec<&str> = ["repo", "workflow"].into_iter().filter(|s| !scopes.contains(s)).collect();
    if missing.is_empty() {
        Check::new("github", Status::Ok, format!("token for {login} has repo + workflow scopes"))
    } else {
        Check::new("github", Status::Fail, format!("token for {login} is missing scope(s): {}", missing.join(", ")))
            .fix(format!("`gh auth refresh -h github.com -s {}`", missing.join(",")))
    }
}

fn git_root() -> Option<PathBuf> {
    let out = Command::new("git").args(["rev-parse", "--show-toplevel"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

fn github_origin() -> Option<String> {
    crate::provision::infer_repo_from_git().ok()
}

fn dockerfile_checks(root: &Path) -> Vec<Check> {
    let path = root.join("Dockerfile");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return vec![Check::new("dockerfile", Status::Fail, "no Dockerfile at the repo root")
            .fix("add a Dockerfile at the repo root (the deploy workflow builds `.`); see `lh agent-guide`")];
    };
    let mut checks = vec![Check::new("dockerfile", Status::Ok, "Dockerfile at the repo root")];
    let ports = final_stage_tcp_exposes(&content);
    checks.push(match ports.iter().min() {
        Some(port) => Check::new("port", Status::Ok, format!("HTTPS traffic will go to port {port} (lowest EXPOSEd TCP port)")),
        None => Check::new("port", Status::Warn, "no EXPOSE in the final stage; litehouse will route to port 3000")
            .fix("add `EXPOSE <port>` for the port your app listens on (bind 0.0.0.0, not localhost)"),
    });
    checks
}

/// TCP ports EXPOSEd in the Dockerfile's last stage (the image that runs).
fn final_stage_tcp_exposes(dockerfile: &str) -> Vec<u32> {
    let mut ports = Vec::new();
    for line in dockerfile.lines() {
        let line = line.trim();
        let mut words = line.split_whitespace();
        let Some(instr) = words.next() else { continue };
        if instr.eq_ignore_ascii_case("FROM") {
            ports.clear();
        } else if instr.eq_ignore_ascii_case("EXPOSE") {
            for spec in words {
                let (port, proto) = spec.split_once('/').unwrap_or((spec, "tcp"));
                if proto.eq_ignore_ascii_case("tcp")
                    && let Ok(p) = port.parse()
                {
                    ports.push(p);
                }
            }
        }
    }
    ports
}

async fn app_check(api: &ApiClient, app: &str, origin: Option<&str>) -> Check {
    match api.get_app_repo(app).await {
        Ok(Some(repo)) if origin.is_some_and(|o| !o.eq_ignore_ascii_case(&repo)) => Check::new(
            "app",
            Status::Warn,
            format!("app '{app}' deploys from {repo}, but this repo's origin is {}", origin.unwrap_or("?")),
        )
        .fix(format!("run from {repo}'s checkout, or re-link with `lh create {app} --rotate-token`")),
        Ok(Some(repo)) => Check::new("app", Status::Ok, format!("app '{app}' exists, deploys from {repo}")),
        Ok(None) => Check::new("app", Status::Warn, format!("app '{app}' exists but has no linked repo"))
            .fix(format!("`lh create {app} --rotate-token` to link it and commit the deploy workflow")),
        Err(e) if e.to_string().contains("not found") => {
            Check::new("app", Status::Skip, format!("app '{app}' doesn't exist yet"))
                .fix(format!("`lh create {app}` registers it and commits the deploy workflow"))
        }
        Err(e) => Check::new("app", Status::Fail, format!("could not look up app '{app}': {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_come_from_the_final_stage_only() {
        let df = "FROM node:20 AS build\nEXPOSE 9999\nRUN npm ci\nFROM node:20-slim\nEXPOSE 8080 9000/udp\nexpose 3001/tcp\n";
        assert_eq!(final_stage_tcp_exposes(df), vec![8080, 3001]);
        assert!(final_stage_tcp_exposes("FROM x\nEXPOSE 1\nFROM y\n").is_empty());
    }

    #[test]
    fn gh_token_without_workflow_scope_fails_with_refresh_hint() {
        let c = scope_check("dan", Some("gist, read:org, repo"));
        assert_eq!(c.status, Status::Fail);
        assert!(c.fix.unwrap().contains("-s workflow"));
    }

    #[test]
    fn token_with_repo_and_workflow_passes() {
        assert_eq!(scope_check("dan", Some("repo, workflow, read:org")).status, Status::Ok);
    }

    #[test]
    fn fine_grained_token_is_a_warning_not_a_failure() {
        assert_eq!(scope_check("dan", None).status, Status::Warn);
    }
}
