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
