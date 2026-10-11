//! App object storage: lets deployed apps store objects in litehouse's S3
//! bucket (the one configured with `lh config s3 set`) without ever seeing
//! the S3 credentials.
//!
//! Every app container gets two env vars at start (see
//! `commands::start::ensure_storage_env_vars`):
//! - `LITEHOUSE_STORAGE_URL` — base URL of the storage API
//!   (`http://litehouse-server:3030/api/storage`, reachable over the shared
//!   `litehouse-network`)
//! - `LITEHOUSE_STORAGE_TOKEN` — a per-app bearer token
//!
//! The server signs or proxies every request with its own credentials and
//! confines it to the app's namespace, `{path_prefix}/storage/{app_name}/`.
//! That root is deliberately separate from the backup roots (`apps/`,
//! `blobs/`, `litehouse/`): an app can neither read nor overwrite backups, and
//! backup retention pruning (which assumes every key under `apps/{app}/` is a
//! dated tarball) never sees app objects.
//!
//! Tokens are `lhs_{app_id}_{hex(HMAC-SHA256(secret, app_id))}`, where the
//! secret is generated once and stored in `system_config`. Nothing per-app is
//! persisted, the token is stable across restarts and redeploys, and a
//! deleted-then-recreated app gets a new id and so a new token. `lh restore`
//! never copies `system_config`, so a rebuilt server mints a fresh secret;
//! that's harmless because restore starts every container afresh with the
//! new token.

use anyhow::{Context, Result};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{Pool, Sqlite};
use std::time::Duration;

use crate::models::{App, S3Config};

/// Env var carrying the storage API base URL into every app container.
pub const STORAGE_URL_ENV_VAR: &str = "LITEHOUSE_STORAGE_URL";

/// Env var carrying the app's storage bearer token into every app container.
pub const STORAGE_TOKEN_ENV_VAR: &str = "LITEHOUSE_STORAGE_TOKEN";

/// Storage API base URL as seen from inside an app container: the
/// litehouse-server container on the shared docker network.
pub const DEFAULT_STORAGE_URL: &str = "http://litehouse-server:3030/api/storage";

const TOKEN_PREFIX: &str = "lhs_";

/// Default lifetime of a presigned URL.
pub const DEFAULT_PRESIGN_EXPIRES_SECS: u64 = 3600;

/// SigV4 caps presigned URLs at 7 days.
pub const MAX_PRESIGN_EXPIRES_SECS: u64 = 7 * 24 * 3600;

const MAX_KEY_LEN: usize = 1024;

// ---------------------------------------------------------------------
// Pure logic (key layout, validation, tokens) — no I/O.
// ---------------------------------------------------------------------

/// The S3 prefix every object of one app lives under.
pub fn storage_prefix_root(prefix: Option<&str>, app_name: &str) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/storage/{app_name}/"),
        None => format!("storage/{app_name}/"),
    }
}

/// Full S3 key for an app-relative object key. `key` must already have
/// passed [`validate_object_key`].
pub fn storage_key(prefix: Option<&str>, app_name: &str, key: &str) -> String {
    format!("{}{key}", storage_prefix_root(prefix, app_name))
}

/// Reject keys that are empty, too long, absolute, contain `.`/`..`/empty
/// path segments, backslashes, or control characters. S3 itself would accept
/// most of these, but they're never what an app means and `..` in particular
/// would read as an escape from the app's namespace to anything that
/// normalizes paths.
pub fn validate_object_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("key must not be empty".into());
    }
    if key.len() > MAX_KEY_LEN {
        return Err(format!("key must be at most {MAX_KEY_LEN} bytes"));
    }
    if key.contains('\\') || key.chars().any(char::is_control) {
        return Err("key must not contain backslashes or control characters".into());
    }
    if key
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err("key must not start or end with '/', or contain '//', '.' or '..' segments".into());
    }
    Ok(())
}

/// A list prefix may be empty (list everything) or end in `/`; otherwise it
/// follows the same rules as a key.
pub fn validate_list_prefix(prefix: &str) -> Result<(), String> {
    match prefix.strip_suffix('/') {
        _ if prefix.is_empty() => Ok(()),
        Some(inner) => validate_object_key(inner),
        None => validate_object_key(prefix),
    }
}

fn token_mac(secret: &str, app_id: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(app_id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Mint the storage token for an app.
pub fn mint_token(secret: &str, app_id: &str) -> String {
    format!("{TOKEN_PREFIX}{app_id}_{}", token_mac(secret, app_id))
}

/// Verify a presented token and return the app id it was minted for.
pub fn verify_token(secret: &str, token: &str) -> Option<String> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let (app_id, mac) = rest.rsplit_once('_')?;
    if app_id.is_empty() {
        return None;
    }
    crate::auth::constant_time_eq(mac, &token_mac(secret, app_id)).then(|| app_id.to_string())
}

// ---------------------------------------------------------------------
// Secret + env vars
// ---------------------------------------------------------------------

async fn read_secret(pool: &Pool<Sqlite>) -> Result<Option<String>> {
    let secret: Option<Option<String>> = sqlx::query_scalar(
        "SELECT app_storage_secret FROM system_config WHERE config_type = 'app_storage'",
    )
    .fetch_optional(pool)
    .await
    .context("failed to read app storage secret")?;
    Ok(secret.flatten().filter(|s| !s.is_empty()))
}

/// The server-wide secret storage tokens are derived from, generated on
/// first use. Safe to race: the insert is a no-op if another caller won, and
/// everyone then reads the one stored row.
pub async fn get_or_create_secret(pool: &Pool<Sqlite>) -> Result<String> {
    if let Some(secret) = read_secret(pool).await? {
        return Ok(secret);
    }
    let now = crate::models::now();
    sqlx::query(
        r#"
        INSERT INTO system_config (id, config_type, app_storage_secret, created_at, updated_at)
        VALUES (?, 'app_storage', ?, ?, ?)
        ON CONFLICT(config_type) DO NOTHING
        "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(crate::auth::generate_token())
    .bind(now.clone())
    .bind(now)
    .execute(pool)
    .await
    .context("failed to initialize app storage secret")?;

    read_secret(pool).await?.context("app storage secret is missing")
}

/// The storage token for `app` (what its container sees as
/// `LITEHOUSE_STORAGE_TOKEN`).
pub async fn token_for_app(pool: &Pool<Sqlite>, app: &App) -> Result<String> {
    Ok(mint_token(&get_or_create_secret(pool).await?, &app.id))
}

/// Resolve a presented bearer token to its app, or `None` if the token is
/// invalid or its app no longer exists.
pub async fn authenticate(pool: &Pool<Sqlite>, token: &str) -> Result<Option<App>> {
    let secret = get_or_create_secret(pool).await?;
    let Some(app_id) = verify_token(&secret, token) else {
        return Ok(None);
    };
    Ok(crate::db::app::get_by_id(pool, &app_id).await?)
}

// ---------------------------------------------------------------------
// S3 operations, all scoped to one app's namespace
// ---------------------------------------------------------------------

/// An app's view of the storage bucket. Every method takes an app-relative
/// key; the namespace prefix is applied here and only here.
pub struct AppStorage {
    client: S3Client,
    bucket: String,
    root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PresignMethod {
    Get,
    Put,
    Delete,
}

#[derive(Debug, Serialize)]
pub struct PresignedUrl {
    pub method: String,
    pub url: String,
    /// Headers that were signed and must be sent with the request.
    pub headers: std::collections::BTreeMap<String, String>,
    pub key: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct ObjectInfo {
    pub key: String,
    pub size: i64,
    pub last_modified: Option<String>,
}

pub struct ObjectBody {
    pub bytes: bytes::Bytes,
    pub content_type: Option<String>,
}

impl AppStorage {
    pub fn new(cfg: &S3Config, app_name: &str) -> Self {
        Self {
            client: crate::backup::s3_client(cfg),
            bucket: cfg.bucket.clone(),
            root: storage_prefix_root(cfg.path_prefix.as_deref(), app_name),
        }
    }

    fn full_key(&self, key: &str) -> String {
        format!("{}{key}", self.root)
    }

    pub async fn presign(
        &self,
        method: PresignMethod,
        key: &str,
        expires_in_secs: u64,
        content_type: Option<&str>,
    ) -> Result<PresignedUrl> {
        let cfg = PresigningConfig::expires_in(Duration::from_secs(expires_in_secs))
            .context("invalid presign expiry")?;
        let full_key = self.full_key(key);
        let req = match method {
            PresignMethod::Get => self
                .client
                .get_object()
                .bucket(&self.bucket)
                .key(&full_key)
                .presigned(cfg)
                .await
                .context("failed to presign GET")?,
            PresignMethod::Put => self
                .client
                .put_object()
                .bucket(&self.bucket)
                .key(&full_key)
                .set_content_type(content_type.map(str::to_string))
                .presigned(cfg)
                .await
                .context("failed to presign PUT")?,
            PresignMethod::Delete => self
                .client
                .delete_object()
                .bucket(&self.bucket)
                .key(&full_key)
                .presigned(cfg)
                .await
                .context("failed to presign DELETE")?,
        };
        Ok(PresignedUrl {
            method: req.method().to_string(),
            url: req.uri().to_string(),
            headers: req
                .headers()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            key: key.to_string(),
            expires_in_secs,
        })
    }

    pub async fn put(&self, key: &str, body: bytes::Bytes, content_type: Option<&str>) -> Result<()> {
        let full_key = self.full_key(key);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .set_content_type(content_type.map(str::to_string))
            .body(ByteStream::from(body))
            .send()
            .await
            .with_context(|| format!("failed to upload s3://{}/{full_key}", self.bucket))?;
        Ok(())
    }

    /// `Ok(None)` when the object doesn't exist.
    pub async fn get(&self, key: &str) -> Result<Option<ObjectBody>> {
        let full_key = self.full_key(key);
        let resp = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) if e.as_service_error().is_some_and(|se| se.is_no_such_key()) => return Ok(None),
            Err(e) => {
                return Err(e).with_context(|| format!("failed to get s3://{}/{full_key}", self.bucket))
            }
        };
        let content_type = resp.content_type().map(str::to_string);
        let bytes = resp
            .body
            .collect()
            .await
            .with_context(|| format!("failed to read s3://{}/{full_key}", self.bucket))?
            .into_bytes();
        Ok(Some(ObjectBody { bytes, content_type }))
    }

    /// Idempotent: deleting a missing key succeeds (S3 semantics).
    pub async fn delete(&self, key: &str) -> Result<()> {
        let full_key = self.full_key(key);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .send()
            .await
            .with_context(|| format!("failed to delete s3://{}/{full_key}", self.bucket))?;
        Ok(())
    }

    /// List objects whose app-relative key starts with `prefix`, returning
    /// app-relative keys.
    pub async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>> {
        let list_prefix = self.full_key(prefix);
        let mut out = Vec::new();
        let mut continuation_token: Option<String> = None;
        loop {
            let resp = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&list_prefix)
                .set_continuation_token(continuation_token.take())
                .send()
                .await
                .with_context(|| format!("failed to list s3://{}/{list_prefix}", self.bucket))?;
            for obj in resp.contents() {
                let Some(key) = obj.key().and_then(|k| k.strip_prefix(&self.root)) else {
                    continue;
                };
                out.push(ObjectInfo {
                    key: key.to_string(),
                    size: obj.size().unwrap_or(0),
                    last_modified: obj.last_modified().map(|t| t.to_string()),
                });
            }
            if resp.is_truncated().unwrap_or(false) {
                continuation_token = resp.next_continuation_token().map(str::to_string);
            } else {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_layout_is_separate_from_backups() {
        assert_eq!(storage_prefix_root(Some("prod"), "hello"), "prod/storage/hello/");
        assert_eq!(storage_prefix_root(None, "hello"), "storage/hello/");
        assert_eq!(storage_prefix_root(Some(""), "hello"), "storage/hello/");
        assert_eq!(
            storage_key(Some("prod"), "hello", "avatars/1.png"),
            "prod/storage/hello/avatars/1.png"
        );

        // Never under any backup root.
        let root = storage_prefix_root(Some("prod"), "hello");
        for backup_root in ["prod/apps/", "prod/blobs/", "prod/litehouse/"] {
            assert!(!root.starts_with(backup_root), "{root} under {backup_root}");
        }
    }

    #[test]
    fn validate_object_key_accepts_normal_keys() {
        for key in ["a", "avatars/1.png", "a/b/c.txt", "with space.txt", "ünïcode/ok", "..hidden", "a.b"] {
            assert!(validate_object_key(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn validate_object_key_rejects_escapes_and_junk() {
        let long = "x".repeat(MAX_KEY_LEN + 1);
        for key in [
            "", "/abs", "trailing/", "a//b", "..", "../x", "a/../b", "./a", "a/.", "a\\b", "a\nb",
            long.as_str(),
        ] {
            assert!(validate_object_key(key).is_err(), "{key:?}");
        }
    }

    #[test]
    fn validate_list_prefix_allows_empty_and_trailing_slash() {
        assert!(validate_list_prefix("").is_ok());
        assert!(validate_list_prefix("avatars/").is_ok());
        assert!(validate_list_prefix("avatars/ab").is_ok());
        assert!(validate_list_prefix("/").is_err());
        assert!(validate_list_prefix("../").is_err());
        assert!(validate_list_prefix("a//").is_err());
    }

    #[test]
    fn token_roundtrip() {
        let token = mint_token("secret", "app-123");
        assert!(token.starts_with("lhs_app-123_"));
        assert_eq!(verify_token("secret", &token).as_deref(), Some("app-123"));
    }

    #[test]
    fn token_rejects_wrong_secret_tamper_and_garbage() {
        let token = mint_token("secret", "app-123");
        assert_eq!(verify_token("other-secret", &token), None);

        // Swapping the app id invalidates the MAC.
        let mac = token.rsplit_once('_').unwrap().1;
        assert_eq!(verify_token("secret", &format!("lhs_app-456_{mac}")), None);

        for bad in ["", "lhs_", "lhs__abc", "nope", "lhs_app-123", "lhs_app-123_"] {
            assert_eq!(verify_token("secret", bad), None, "{bad:?}");
        }
    }

    #[tokio::test]
    async fn secret_is_created_once_and_stable() {
        let pool = crate::db::test::get_test_pool().await;
        let a = get_or_create_secret(&pool).await.unwrap();
        let b = get_or_create_secret(&pool).await.unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
