use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::json;

const API: &str = "https://api.github.com";

fn client(token: &str) -> Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token))
            .context("building Authorization header")?,
    );
    headers.insert(USER_AGENT, HeaderValue::from_static("litehouse"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("building GitHub HTTP client")
}

/// GitHub sealed-box encryption for Actions secrets.
///
/// GitHub Actions secrets must be encrypted client-side using libsodium's
/// "sealed box" construction against the repository's public key before
/// being sent to the API.
pub fn seal_secret_for_github(repo_public_key_b64: &str, secret: &str) -> Result<String> {
    let pk_bytes = B64
        .decode(repo_public_key_b64)
        .context("decoding repository public key (expected base64)")?;
    let pk_bytes: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("repository public key had unexpected length: {}", v.len()))?;
    let public_key = crypto_box::PublicKey::from_bytes(pk_bytes);

    let sealed = public_key
        .seal(&mut crypto_box::aead::OsRng, secret.as_bytes())
        .map_err(|e| anyhow!("failed to seal secret: {}", e))?;

    Ok(B64.encode(sealed))
}

#[derive(serde::Deserialize)]
struct PublicKeyResponse {
    key: String,
    key_id: String,
}

/// PUT /repos/{owner}/{repo}/actions/secrets/{name}
///
/// Sets (creates or updates) an encrypted Actions secret on a repository.
pub async fn put_actions_secret(
    token: &str,
    owner: &str,
    repo: &str,
    name: &str,
    value: &str,
) -> Result<()> {
    let http = client(token)?;

    let pk_url = format!("{}/repos/{}/{}/actions/secrets/public-key", API, owner, repo);
    let resp = http
        .get(&pk_url)
        .send()
        .await
        .with_context(|| format!("fetching Actions public key for {}/{}", owner, repo))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "fetching Actions public key for {}/{} failed ({}): {}",
            owner,
            repo,
            status,
            body
        ));
    }

    let public_key: PublicKeyResponse = resp
        .json()
        .await
        .with_context(|| format!("parsing Actions public key response for {}/{}", owner, repo))?;

    let encrypted_value = seal_secret_for_github(&public_key.key, value)
        .with_context(|| format!("sealing secret {} for {}/{}", name, owner, repo))?;

    let put_url = format!(
        "{}/repos/{}/{}/actions/secrets/{}",
        API, owner, repo, name
    );
    let body = json!({
        "encrypted_value": encrypted_value,
        "key_id": public_key.key_id,
    });

    let resp = http
        .put(&put_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("setting Actions secret {} on {}/{}", name, owner, repo))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "setting Actions secret {} on {}/{} failed ({}): {}",
            name,
            owner,
            repo,
            status,
            body
        ));
    }

    Ok(())
}

#[derive(serde::Deserialize)]
struct ContentsResponse {
    sha: String,
}

/// Create or update a file via PUT /repos/{owner}/{repo}/contents/{path}.
///
/// Fetches the existing file's sha first (200 -> include "sha" in body to
/// update; 404 -> create a new file).
pub async fn put_file(
    token: &str,
    owner: &str,
    repo: &str,
    path: &str,
    content: &str,
    message: &str,
) -> Result<()> {
    let http = client(token)?;

    let contents_url = format!("{}/repos/{}/{}/contents/{}", API, owner, repo, path);

    let existing_sha = {
        let resp = http
            .get(&contents_url)
            .send()
            .await
            .with_context(|| format!("checking for existing file {} in {}/{}", path, owner, repo))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            None
        } else if resp.status().is_success() {
            let existing: ContentsResponse = resp.json().await.with_context(|| {
                format!("parsing existing file response for {} in {}/{}", path, owner, repo)
            })?;
            Some(existing.sha)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "checking for existing file {} in {}/{} failed ({}): {}",
                path,
                owner,
                repo,
                status,
                body
            ));
        }
    };

    let mut body = json!({
        "message": message,
        "content": B64.encode(content.as_bytes()),
    });
    if let Some(sha) = existing_sha {
        body["sha"] = json!(sha);
    }

    let resp = http
        .put(&contents_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("writing file {} to {}/{}", path, owner, repo))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "writing file {} to {}/{} failed ({}): {}",
            path,
            owner,
            repo,
            status,
            body
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_secret_roundtrip() {
        use crypto_box::{aead::OsRng, SecretKey};
        let sk = SecretKey::generate(&mut OsRng);
        let pk_b64 = base64::engine::general_purpose::STANDARD.encode(sk.public_key().as_bytes());
        let sealed = seal_secret_for_github(&pk_b64, "hunter2").unwrap();
        let sealed_bytes = base64::engine::general_purpose::STANDARD
            .decode(sealed)
            .unwrap();
        let opened = sk.unseal(&sealed_bytes).unwrap();
        assert_eq!(opened, b"hunter2");
    }
}

/// Filename of the deploy workflow `lh create` commits into an app's repo.
pub const DEPLOY_WORKFLOW_FILE: &str = "litehouse-deploy.yml";

/// A GitHub Actions run of the litehouse deploy workflow.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    pub head_sha: String,
    /// "queued", "in_progress", "completed", ...
    pub status: String,
    /// Set once `status == "completed"`: "success", "failure", "cancelled", ...
    pub conclusion: Option<String>,
    pub html_url: String,
}

impl WorkflowRun {
    /// True once the run finished without succeeding — the image was never
    /// built/pushed, or the deploy hook call failed, so no litehouse deploy
    /// for this commit is coming.
    pub fn failed(&self) -> bool {
        self.status == "completed" && self.conclusion.as_deref() != Some("success")
    }
}

/// The newest run of the litehouse deploy workflow for a commit whose sha
/// starts with `sha_prefix` (short or full sha). `Ok(None)` when there is no
/// such run yet, or the repo has no litehouse workflow.
pub async fn deploy_run_for_sha(
    token: &str,
    owner: &str,
    repo: &str,
    sha_prefix: &str,
) -> Result<Option<WorkflowRun>> {
    #[derive(serde::Deserialize)]
    struct Runs {
        workflow_runs: Vec<WorkflowRun>,
    }

    let url = format!(
        "{}/repos/{}/{}/actions/workflows/{}/runs?per_page=30",
        API, owner, repo, DEPLOY_WORKFLOW_FILE
    );
    let resp = client(token)?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("listing deploy workflow runs for {}/{}", owner, repo))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "listing deploy workflow runs for {}/{} failed ({}): {}",
            owner,
            repo,
            status,
            body
        ));
    }
    let runs: Runs = resp.json().await.context("parsing workflow runs")?;
    Ok(newest_run_for_sha(runs.workflow_runs, sha_prefix))
}

/// Runs come back newest first; pick the first whose head sha matches.
fn newest_run_for_sha(runs: Vec<WorkflowRun>, sha_prefix: &str) -> Option<WorkflowRun> {
    let prefix = sha_prefix.to_lowercase();
    runs.into_iter()
        .find(|r| r.head_sha.to_lowercase().starts_with(&prefix))
}

#[cfg(test)]
mod run_tests {
    use super::*;

    fn run(id: u64, sha: &str, status: &str, conclusion: Option<&str>) -> WorkflowRun {
        WorkflowRun {
            id,
            head_sha: sha.to_string(),
            status: status.to_string(),
            conclusion: conclusion.map(str::to_string),
            html_url: format!("https://github.com/o/r/actions/runs/{id}"),
        }
    }

    #[test]
    fn picks_newest_run_matching_short_sha() {
        let runs = vec![
            run(3, "ffff000", "completed", Some("success")),
            run(2, "abc1234def", "completed", Some("failure")),
            run(1, "abc1234def", "completed", Some("success")),
        ];
        let found = newest_run_for_sha(runs, "ABC1234").unwrap();
        assert_eq!(found.id, 2);
        assert!(found.failed());
    }

    #[test]
    fn in_progress_and_successful_runs_are_not_failed() {
        assert!(!run(1, "a", "in_progress", None).failed());
        assert!(!run(1, "a", "completed", Some("success")).failed());
        assert!(run(1, "a", "completed", Some("cancelled")).failed());
    }
}
