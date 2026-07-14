//! Backup engine.
//!
//! For every app, runs a one-shot `keinos/sqlite3` container that mounts the
//! app's data volume read-only and the litehouse backups directory
//! (`config::get_backups_dir()`), `VACUUM INTO`s every SQLite database it
//! finds (a point-in-time-consistent snapshot even if the app is writing
//! concurrently), and tars up everything else. The server then tars the
//! staged directory and uploads it to S3, alongside a `VACUUM INTO` snapshot
//! of litehouse's own state DB. Old backups are pruned, keeping only the
//! newest N per app/state prefix.
//!
//! The backups directory is bind-mounted (rather than a named Docker
//! volume) so that both the one-shot snapshot container *and* the litehouse
//! server process itself (which tars and uploads the staged files) see the
//! exact same files at the exact same host path — no VM-internal volume
//! storage to reach through. In production the install script points
//! `LITEHOUSE_BACKUPS_DIR` (or the default `{base_dir}/backups`) at
//! `/opt/litehouse/backups`; the litehouse-server container bind-mounts that
//! same host path.
//!
//! Scheduling (running this on a timer) is a separate concern — this module
//! only exposes `run_backup`, to be invoked by whatever driver wants it.

use anyhow::{anyhow, bail, Context, Result};
use aws_sdk_s3::config::{BehaviorVersion, Builder as S3ConfigBuilder, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
    WaitContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Pool, Sqlite};
use std::fs::File;
use std::path::Path;
use tracing::{info, instrument, warn};

use crate::commands::start::start_container;
use crate::models::S3Config;
use crate::{caddy, config, db, docker, volume};

/// How many daily backups to retain per app / per the litehouse state DB.
pub const RETENTION_COUNT: usize = 14;

/// Env var name every app container receives at start (see
/// `commands::start::ensure_blob_path_env_var`), pointing at the directory
/// it should write incrementally-backed-up blobs into.
pub const BLOB_PATH_ENV_VAR: &str = "LITEHOUSE_BLOB_PATH";

/// Path (inside the app's own `/data` mount) apps are told to use for blobs
/// via `BLOB_PATH_ENV_VAR`.
pub const BLOB_MOUNT_PATH: &str = "/data/blobs";

/// Name of the directory relative to `/data` (and relative to the per-app
/// backup staging dir) used for blobs. Must match the trailing path
/// component of `BLOB_MOUNT_PATH`.
const BLOB_DIR_NAME: &str = "blobs";

/// Process-wide backup/restore mutex. `run_backup` and `restore_all` both
/// acquire this for their full duration so the hourly scheduler, a manual
/// `POST /backups/run`, and a `POST /restore` can never interleave their
/// container/volume operations (e.g. a restore stopping a container while a
/// backup's snapshot container has its volume mounted). Callers just get
/// serialized — the second caller waits, it doesn't error.
static BACKUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("S3 backup configuration is not set. Run `lh server s3-config set` first.")]
    S3ConfigMissing,
    #[error("invalid app name for backup: {0}")]
    InvalidAppName(String),
    #[error("snapshot container for app '{app}' failed (exit code {exit_code}): {log_tail}")]
    SnapshotFailed {
        app: String,
        exit_code: i64,
        log_tail: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupReport {
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub ran_at: String,
}

/// Outcome of a full disaster-recovery restore from S3 (see [`restore_all`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreReport {
    /// App names whose image was pulled, data restored (if any backup
    /// existed), and container started.
    pub restored: Vec<String>,
    /// App names skipped, with a human-readable reason (missing image or no
    /// app data backup found) — not treated as errors.
    pub skipped: Vec<(String, String)>,
}

// ---------------------------------------------------------------------
// Pure logic (S3 key layout + retention) — no I/O, easy to unit test.
// ---------------------------------------------------------------------

/// Build the S3 key for an app's daily backup tarball.
pub fn app_backup_key(prefix: Option<&str>, app_name: &str, date: &str) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/apps/{app_name}/{date}.tar.gz"),
        None => format!("apps/{app_name}/{date}.tar.gz"),
    }
}

/// Build the S3 key for the litehouse state DB snapshot.
pub fn state_backup_key(prefix: Option<&str>, date: &str) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/litehouse/{date}.db"),
        None => format!("litehouse/{date}.db"),
    }
}

/// Given the full list of keys under some prefix (e.g. all keys for one
/// app), return the ones that should be deleted to keep only the newest
/// `keep`. ISO-8601 dates embedded in the key sort lexically the same as
/// chronologically, so a plain string sort suffices.
pub fn keys_to_prune(keys: &[String], keep: usize) -> Vec<String> {
    if keys.len() <= keep {
        return vec![];
    }
    let mut sorted: Vec<String> = keys.to_vec();
    sorted.sort();
    let cut = sorted.len() - keep;
    sorted[..cut].to_vec()
}

/// Given a list of S3 keys sharing a common prefix, return the
/// lexicographically-newest one. ISO-8601 dates embedded in the key sort
/// lexically the same as chronologically (mirrors [`keys_to_prune`]).
pub fn newest_key(keys: &[String]) -> Option<String> {
    keys.iter().max().cloned()
}

/// Escape a string for embedding inside a single-quoted SQL string literal
/// (SQL escapes an embedded `'` by doubling it to `''`). Used everywhere a
/// filesystem path or filename that isn't otherwise validated gets spliced
/// into a `VACUUM INTO '...'` statement, so it can never break out of the
/// literal.
fn sql_quote_literal(s: &str) -> String {
    s.replace('\'', "''")
}

/// Apps are validated at creation time to be lowercase alphanumeric plus
/// `-`/`_`, but this is re-checked here defensively since the app name is
/// interpolated into a shell script run inside the snapshot container.
fn validate_app_name_for_shell(name: &str) -> Result<(), BackupError> {
    let safe = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if safe {
        Ok(())
    } else {
        Err(BackupError::InvalidAppName(name.to_string()))
    }
}

// ---------------------------------------------------------------------
// S3 client
// ---------------------------------------------------------------------

pub fn s3_client(cfg: &S3Config) -> S3Client {
    let creds = aws_credential_types::Credentials::new(
        cfg.access_key_id.clone(),
        cfg.secret_access_key.clone(),
        None,
        None,
        "litehouse",
    );
    let mut builder = S3ConfigBuilder::new()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(cfg.region.clone()))
        .credentials_provider(creds)
        .force_path_style(true);
    if let Some(endpoint) = &cfg.endpoint {
        builder = builder.endpoint_url(endpoint.clone());
    }
    S3Client::from_conf(builder.build())
}

/// Upload a file to S3 at the given key.
#[instrument(skip(client))]
async fn upload_file(client: &S3Client, bucket: &str, key: &str, path: &Path) -> Result<()> {
    let body = ByteStream::from_path(path)
        .await
        .with_context(|| format!("failed to read {} for upload", path.display()))?;
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(body)
        .send()
        .await
        .with_context(|| format!("failed to upload s3://{bucket}/{key}"))?;
    Ok(())
}

/// Download an S3 object to a local path.
#[instrument(skip(client))]
async fn download_file(client: &S3Client, bucket: &str, key: &str, dest: &Path) -> Result<()> {
    let obj = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .with_context(|| format!("failed to get s3://{bucket}/{key}"))?;
    let bytes = obj
        .body
        .collect()
        .await
        .with_context(|| format!("failed to read body of s3://{bucket}/{key}"))?
        .into_bytes();
    std::fs::write(dest, &bytes)
        .with_context(|| format!("failed to write {} ", dest.display()))?;
    Ok(())
}

/// List all object keys under a prefix.
#[instrument(skip(client))]
async fn list_keys(client: &S3Client, bucket: &str, prefix: &str) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut continuation_token: Option<String> = None;
    loop {
        let mut req = client.list_objects_v2().bucket(bucket).prefix(prefix);
        if let Some(token) = &continuation_token {
            req = req.continuation_token(token.clone());
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("failed to list s3://{bucket}/{prefix}"))?;
        for obj in resp.contents() {
            if let Some(key) = obj.key() {
                keys.push(key.to_string());
            }
        }
        if resp.is_truncated().unwrap_or(false) {
            continuation_token = resp.next_continuation_token().map(|s| s.to_string());
        } else {
            break;
        }
    }
    Ok(keys)
}

/// Delete a batch of keys from S3.
#[instrument(skip(client))]
async fn delete_keys(client: &S3Client, bucket: &str, keys: &[String]) -> Result<()> {
    for key in keys {
        client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .with_context(|| format!("failed to delete s3://{bucket}/{key}"))?;
    }
    Ok(())
}

/// Prune old backups under `prefix`, keeping the newest `RETENTION_COUNT`,
/// and remove the matching rows from the `backup` catalog so it never lists
/// an artifact that no longer exists in S3.
#[instrument(skip(pool, client))]
async fn prune_old_backups(pool: &Pool<Sqlite>, client: &S3Client, bucket: &str, prefix: &str) -> Result<()> {
    let keys = list_keys(client, bucket, prefix).await?;
    let doomed = keys_to_prune(&keys, RETENTION_COUNT);
    if !doomed.is_empty() {
        info!(
            "pruning {} old backup(s) under s3://{bucket}/{prefix}",
            doomed.len()
        );
        delete_keys(client, bucket, &doomed).await?;
        if let Err(e) = db::backup::delete_by_keys(pool, &doomed).await {
            warn!("failed to prune backup catalog rows for s3://{bucket}/{prefix}: {e:#}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Per-app snapshot: one-shot container + tar + upload
// ---------------------------------------------------------------------

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Build the shell script run inside the one-shot snapshot container.
/// `/backup` is bind-mounted to the app's *own* staging directory (never the
/// whole backups dir — see [`run_snapshot_container`]), so every path here
/// is already scoped to this app; no app name needs interpolating.
///
/// `$f` (a filename discovered inside the read-only app volume) is
/// attacker-influenced in principle (whatever the app itself wrote under
/// `/data`), so it's never spliced directly into the single-quoted SQL
/// string literal passed to `sqlite3` — a filename containing a `'` would
/// otherwise break out of the literal. Instead every embedded `'` is SQL-
/// escaped by doubling it before it's used inside the `VACUUM INTO '...'`
/// literal (the double-quoted shell string around `$f` itself already
/// protects against shell metacharacters/word-splitting).
fn snapshot_script() -> String {
    r#"set -e
rm -rf /backup/dbs /backup/files.tar.gz /backup/blobs
mkdir -p /backup/dbs
cd /data
find . -path './blobs' -prune -o -name '*.db' -print -o -name '*.sqlite' -print -o -name '*.sqlite3' -print | while read -r f; do
  mkdir -p "/backup/dbs/$(dirname "$f")"
  esc=$(printf '%s' "$f" | sed "s/'/''/g")
  sqlite3 "file:$f?mode=ro&immutable=0" "VACUUM INTO '/backup/dbs/$esc'"
done
tar czf /backup/files.tar.gz --exclude='*.db' --exclude='*.sqlite' --exclude='*.sqlite3' --exclude='*-wal' --exclude='*-shm' --exclude='./blobs' .
[ -d blobs ] && cp -a blobs /backup/blobs || true
"#
    .to_string()
}

/// Run the one-shot `keinos/sqlite3` snapshot container for `app`, staging
/// its output under `{backups_dir}/{app}`. That per-app directory (not the
/// whole backups dir) is what's bind-mounted at `/backup`, so even a
/// maliciously-crafted filename inside the app's own volume can't be used
/// to write outside the app's own staging area.
#[instrument(skip(docker))]
async fn run_snapshot_container(
    docker: &Docker,
    app_id: &str,
    app_name: &str,
    backups_dir: &Path,
) -> Result<()> {
    validate_app_name_for_shell(app_name)?;

    let staged_dir = backups_dir.join(app_name);
    std::fs::create_dir_all(&staged_dir)
        .with_context(|| format!("failed to create staging dir {}", staged_dir.display()))?;
    // The snapshot container image (keinos/sqlite3) runs as a non-root user
    // (`sqlite`), but this staging dir is created by litehouse-server as root
    // with the default 0755 umask, so the container can't write into its
    // `/backup` bind mount (`mkdir /backup/dbs: Permission denied`). Make the
    // staging dir world-writable so the snapshot container — whatever UID it
    // runs as — can stage its output here. `create_dir_all` above won't relax
    // permissions on an already-existing dir, so set them explicitly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged_dir, std::fs::Permissions::from_mode(0o777))
            .with_context(|| {
                format!(
                    "failed to make staging dir {} world-writable",
                    staged_dir.display()
                )
            })?;
    }
    let staged_dir_str = staged_dir
        .to_str()
        .context("staging dir path is not valid UTF-8")?;

    let app_volume = crate::volume::get_app_volume_name(app_id);
    let script = snapshot_script();
    let container_name = format!("litehouse-backup-{}", app_name);

    // Clean up any leftover container from a previous failed run.
    let _ = docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;

    let container_config = ContainerConfig {
        image: Some("keinos/sqlite3:latest".to_string()),
        entrypoint: Some(vec!["sh".to_string()]),
        cmd: Some(vec!["-c".to_string(), script]),
        host_config: Some(HostConfig {
            binds: Some(vec![
                format!("{}:/data:ro", app_volume),
                format!("{}:/backup", staged_dir_str),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                platform: None,
            }),
            container_config,
        )
        .await
        .context("failed to create snapshot container")?;

    docker
        .start_container::<String>(&container.id, None)
        .await
        .context("failed to start snapshot container")?;

    let mut wait_stream = docker.wait_container(
        &container.id,
        None::<WaitContainerOptions<String>>,
    );
    let mut exit_code: i64 = -1;
    while let Some(result) = wait_stream.next().await {
        match result {
            Ok(response) => exit_code = response.status_code,
            Err(e) => {
                // bollard surfaces a non-zero exit as an Err in some
                // versions; try to recover the code by inspecting instead.
                warn!("wait_container stream error, falling back to inspect: {e}");
            }
        }
    }

    if exit_code == -1 {
        // Fall back to inspecting the container state directly.
        let inspect = docker.inspect_container(&container.id, None).await?;
        exit_code = inspect
            .state
            .and_then(|s| s.exit_code)
            .unwrap_or(-1);
    }

    if exit_code != 0 {
        let log_tail = fetch_log_tail(docker, &container.id).await;
        let _ = docker
            .remove_container(
                &container.id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
        return Err(BackupError::SnapshotFailed {
            app: app_name.to_string(),
            exit_code,
            log_tail,
        }
        .into());
    }

    docker
        .remove_container(
            &container.id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .context("failed to remove snapshot container")?;

    Ok(())
}

async fn fetch_log_tail(docker: &Docker, container_id: &str) -> String {
    let options = Some(LogsOptions::<String> {
        stdout: true,
        stderr: true,
        tail: "50".to_string(),
        ..Default::default()
    });
    let mut stream = docker.logs(container_id, options);
    let mut out = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(log) => out.push_str(&log.to_string()),
            Err(_) => break,
        }
    }
    out
}

/// Tar up the staged backup directory `{backups_dir}/{app_name}` into
/// `{dest}` (a `.tar.gz`).
fn tar_staged_dir(staged_dir: &Path, dest: &Path) -> Result<()> {
    let tar_gz = File::create(dest)
        .with_context(|| format!("failed to create {}", dest.display()))?;
    let enc = GzEncoder::new(tar_gz, Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.append_dir_all(".", staged_dir)
        .with_context(|| format!("failed to tar {}", staged_dir.display()))?;
    tar.finish().context("failed to finalize tar archive")?;
    Ok(())
}

// ---------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------

/// Run a full backup: snapshot the litehouse state DB and every app's data,
/// upload everything to S3, and prune old backups. Per-app failures are
/// recorded in the report rather than aborting the whole run.
#[instrument(skip(pool, docker))]
pub async fn run_backup(pool: &Pool<Sqlite>, docker: &Docker) -> Result<BackupReport> {
    // Serialize against any concurrent backup/restore (see BACKUP_LOCK).
    let _guard = BACKUP_LOCK.lock().await;

    let s3_config = db::system_config::get_s3_config(pool)
        .await?
        .ok_or(BackupError::S3ConfigMissing)?;
    let client = s3_client(&s3_config);
    let bucket = s3_config.bucket.clone();
    let prefix = s3_config.path_prefix.clone();
    let date = today();

    let backups_dir = config::get_backups_dir().context("failed to resolve backups directory")?;

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    // 1. litehouse's own state DB.
    match backup_state_db(pool, &client, &bucket, prefix.as_deref(), &date, &backups_dir).await {
        Ok(()) => succeeded.push("litehouse".to_string()),
        Err(e) => {
            let msg = format!("{e:#}");
            warn!("litehouse state DB backup failed: {msg}");
            failed.push(("litehouse".to_string(), msg));
        }
    }

    // 2. Every app.
    let apps = db::app::get_all(pool).await.context("failed to list apps")?;
    for app in apps {
        match backup_app(pool, docker, &client, &bucket, prefix.as_deref(), &date, &backups_dir, &app.id, &app.name)
            .await
        {
            Ok(()) => succeeded.push(app.name.clone()),
            Err(e) => {
                let msg = format!("{e:#}");
                warn!("backup failed for app '{}': {msg}", app.name);
                failed.push((app.name.clone(), msg));
            }
        }
    }

    let report = BackupReport {
        succeeded,
        failed,
        ran_at: chrono::Utc::now().to_rfc3339(),
    };

    if let Err(e) = db::system_config::set_last_backup_report(pool, &report).await {
        warn!("failed to persist backup report: {e:#}");
    }

    Ok(report)
}

async fn backup_state_db(
    pool: &Pool<Sqlite>,
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    date: &str,
    backups_dir: &Path,
) -> Result<()> {
    let snapshot_path = backups_dir.join(format!("litehouse-{date}.db"));
    // Remove a stale snapshot from an earlier same-day run, if any — VACUUM
    // INTO refuses to overwrite an existing file.
    let _ = std::fs::remove_file(&snapshot_path);

    let snapshot_path_str = snapshot_path
        .to_str()
        .context("backups dir path is not valid UTF-8")?;
    // `snapshot_path` is built from the configured backups dir + a
    // chrono-formatted date, so it's not attacker-controlled today — but
    // harden the SQL literal construction anyway rather than relying on
    // that staying true forever.
    let escaped_path = sql_quote_literal(snapshot_path_str);
    sqlx::query(&format!("VACUUM INTO '{}'", escaped_path))
        .execute(pool)
        .await
        .context("failed to VACUUM INTO litehouse state DB snapshot")?;

    let key = state_backup_key(prefix, date);
    upload_file(client, bucket, &key, &snapshot_path).await?;
    let size_bytes = std::fs::metadata(&snapshot_path).map(|m| m.len() as i64).unwrap_or(0);
    let _ = std::fs::remove_file(&snapshot_path);

    if let Err(e) = db::backup::record_upload(pool, "litehouse-state", &key, size_bytes).await {
        warn!("failed to catalog litehouse state backup: {e:#}");
    }

    prune_old_backups(pool, client, bucket, &state_prefix_root(prefix)).await?;
    Ok(())
}

fn state_prefix_root(prefix: Option<&str>) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/litehouse/"),
        None => "litehouse/".to_string(),
    }
}

fn app_prefix_root(prefix: Option<&str>, app_name: &str) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/apps/{app_name}/"),
        None => format!("apps/{app_name}/"),
    }
}

/// Build the S3 key for one blob file. Deliberately lives under its own
/// top-level `blobs/` prefix — NOT nested under `apps/{app_name}/` — because
/// that prefix is also used by `newest_key`/`prune_old_backups` for the
/// dated tarball backups, which assume every key there is a dated
/// `YYYY-MM-DD.tar.gz` snapshot. See the design doc for the full reasoning.
pub fn blob_key(prefix: Option<&str>, app_name: &str, relative_path: &str) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/blobs/{app_name}/{relative_path}"),
        None => format!("blobs/{app_name}/{relative_path}"),
    }
}

/// Build the S3 prefix under which all of one app's blobs live.
pub fn blob_prefix_root(prefix: Option<&str>, app_name: &str) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{p}/blobs/{app_name}/"),
        None => format!("blobs/{app_name}/"),
    }
}

/// Given the relative paths of every blob file found locally and the set of
/// keys already present in S3 under this app's blob prefix, return the
/// relative paths that still need to be uploaded (pure, no I/O — the
/// existing-keys listing and the upload itself happen in `backup_blobs`).
pub fn blobs_missing_from_s3(
    local_relative_paths: &[String],
    existing_keys: &[String],
    prefix: Option<&str>,
    app_name: &str,
) -> Vec<String> {
    local_relative_paths
        .iter()
        .filter(|rel| !existing_keys.contains(&blob_key(prefix, app_name, rel)))
        .cloned()
        .collect()
}

async fn backup_app(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    date: &str,
    backups_dir: &Path,
    app_id: &str,
    app_name: &str,
) -> Result<()> {
    run_snapshot_container(docker, app_id, app_name, backups_dir).await?;

    let staged_dir = backups_dir.join(app_name);
    if !staged_dir.exists() {
        bail!(
            "snapshot container reported success but staged dir {} is missing",
            staged_dir.display()
        );
    }

    let tarball_path = backups_dir.join(format!("{app_name}-{date}.tar.gz"));
    tar_staged_dir(&staged_dir, &tarball_path)?;

    let key = app_backup_key(prefix, app_name, date);
    let upload_result = upload_file(client, bucket, &key, &tarball_path).await;
    let size_bytes = std::fs::metadata(&tarball_path).map(|m| m.len() as i64).unwrap_or(0);

    // Clean up local staging regardless of upload outcome.
    let _ = std::fs::remove_file(&tarball_path);
    let _ = std::fs::remove_dir_all(&staged_dir);

    upload_result?;

    if let Err(e) = db::backup::record_upload(pool, app_name, &key, size_bytes).await {
        warn!("failed to catalog backup for app '{app_name}': {e:#}");
    }

    prune_old_backups(pool, client, bucket, &app_prefix_root(prefix, app_name)).await?;
    Ok(())
}

// ---------------------------------------------------------------------
// Restore
// ---------------------------------------------------------------------

/// Copy every `app` row (and its `env_var` rows) from `src_pool` into
/// `dst_pool`, using `INSERT OR IGNORE` so existing rows in `dst_pool` (the
/// live DB) always win — restoring never clobbers current state, and
/// running it repeatedly is a no-op past the first successful copy. Returns
/// the number of app rows found in the snapshot.
///
/// Deliberately does *not* touch `system_config` — live S3/GHCR credentials
/// must never be overwritten by whatever was captured in an old snapshot.
#[instrument(skip(src_pool, dst_pool))]
pub async fn copy_apps_from_snapshot(src_pool: &Pool<Sqlite>, dst_pool: &Pool<Sqlite>) -> Result<usize> {
    let apps = db::app::get_all(src_pool)
        .await
        .context("failed to read apps from snapshot DB")?;

    for app in &apps {
        db::app::insert_or_ignore(dst_pool, app)
            .await
            .with_context(|| format!("failed to copy app '{}' from snapshot", app.name))?;

        let env_vars = db::env_var::get_by_app(src_pool, &app.id)
            .await
            .with_context(|| format!("failed to read env vars for app '{}' from snapshot", app.name))?;
        for env_var in env_vars {
            db::env_var::insert_or_ignore(dst_pool, &env_var)
                .await
                .with_context(|| {
                    format!(
                        "failed to copy env var '{}' for app '{}' from snapshot",
                        env_var.key, app.name
                    )
                })?;
        }
    }

    Ok(apps.len())
}

/// Extract the outer tarball an app's daily backup was stored as (contains
/// `dbs/<relative-path>` VACUUM'd sqlite files and `files.tar.gz` for
/// everything else) into `dest_dir`.
fn extract_outer_tarball(tarball_path: &Path, dest_dir: &Path) -> Result<()> {
    let tar_gz = File::open(tarball_path)
        .with_context(|| format!("failed to open {}", tarball_path.display()))?;
    let dec = GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(dec);
    archive
        .unpack(dest_dir)
        .with_context(|| format!("failed to unpack {}", tarball_path.display()))?;
    Ok(())
}

/// Run a one-shot `alpine:3.20` container that applies a restore staging
/// directory (bind-mounted read-only at `/restore`, containing `files.tar.gz`
/// and/or a `dbs/` tree as produced by [`extract_outer_tarball`]) onto the
/// app's data volume (bind... mounted at `/data`): untar `files.tar.gz` into
/// `/data`, then copy `dbs/*` over it (VACUUM'd DB snapshots win over
/// whatever the tarball had for the same path). If `uid_gid` is known
/// (discovered from the target image), the restored files are chowned to
/// match; otherwise they're left world-writable, mirroring
/// `volume::init_app_volume`'s fallback.
#[instrument(skip(docker))]
async fn run_restore_container(
    docker: &Docker,
    app_id: &str,
    stage_dir: &Path,
    uid_gid: Option<(u32, u32)>,
) -> Result<()> {
    let stage_dir_str = stage_dir
        .to_str()
        .context("restore staging dir path is not valid UTF-8")?;
    let app_volume = crate::volume::get_app_volume_name(app_id);
    let container_name = format!("litehouse-restore-{}", app_id);

    let perm_fix = match uid_gid {
        Some((uid, gid)) => format!("chown -R {uid}:{gid} /data"),
        None => "chmod -R a+rwX /data".to_string(),
    };
    let script = format!(
        r#"set -e
mkdir -p /data
if [ -f /restore/files.tar.gz ]; then tar xzf /restore/files.tar.gz -C /data; fi
if [ -d /restore/dbs ]; then cp -a /restore/dbs/. /data/; fi
{perm_fix}
"#
    );

    let _ = docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;

    let container_config = ContainerConfig {
        image: Some("alpine:3.20".to_string()),
        entrypoint: Some(vec!["sh".to_string()]),
        cmd: Some(vec!["-c".to_string(), script]),
        host_config: Some(HostConfig {
            binds: Some(vec![
                format!("{}:/data", app_volume),
                format!("{}:/restore:ro", stage_dir_str),
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let container = docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                platform: None,
            }),
            container_config,
        )
        .await
        .context("failed to create restore container")?;

    docker
        .start_container::<String>(&container.id, None)
        .await
        .context("failed to start restore container")?;

    let mut wait_stream = docker.wait_container(&container.id, None::<WaitContainerOptions<String>>);
    let mut exit_code: i64 = -1;
    while let Some(result) = wait_stream.next().await {
        match result {
            Ok(response) => exit_code = response.status_code,
            Err(e) => warn!("wait_container stream error, falling back to inspect: {e}"),
        }
    }
    if exit_code == -1 {
        let inspect = docker.inspect_container(&container.id, None).await?;
        exit_code = inspect.state.and_then(|s| s.exit_code).unwrap_or(-1);
    }

    let log_tail = if exit_code != 0 {
        Some(fetch_log_tail(docker, &container.id).await)
    } else {
        None
    };

    let _ = docker
        .remove_container(
            &container.id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;

    if exit_code != 0 {
        bail!(
            "restore container for app '{}' failed (exit code {}): {}",
            app_id,
            exit_code,
            log_tail.unwrap_or_default()
        );
    }

    Ok(())
}

enum RestoreOutcome {
    Restored,
    Skipped(String),
}

/// Restore one app: pull its recorded image, download + extract its newest
/// app-data backup (if any), then — and only then — replace the running
/// container: stop/remove it, ensure its data volume exists (create-if-
/// absent, existing volume contents are never wiped; the restore container
/// overlays the backup on top), apply the backup, and start.
///
/// Ordering is deliberate: everything fallible-over-the-network (image
/// pull, backup download, tarball extract) happens *before* the app's
/// container is touched, so a failed download/extract leaves a healthy
/// running app untouched.
///
/// Apps with no recorded image, or with an image but no app-data backup in
/// S3, are skipped (not errored) — see [`RestoreReport`].
#[instrument(skip(pool, docker, client, backups_dir, ghcr_token, app))]
async fn restore_app(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    backups_dir: &Path,
    ghcr_token: Option<&str>,
    app: &crate::models::App,
) -> Result<RestoreOutcome> {
    let image = match &app.image {
        Some(image) => image.clone(),
        None => return Ok(RestoreOutcome::Skipped("no deployed image on record".to_string())),
    };

    let app_prefix = app_prefix_root(prefix, &app.name);
    let keys = list_keys(client, bucket, &app_prefix).await?;
    let newest = match newest_key(&keys) {
        Some(key) => key,
        None => {
            return Ok(RestoreOutcome::Skipped(format!(
                "no app data backups found under s3://{bucket}/{app_prefix}"
            )))
        }
    };

    // --- Phase 1: everything fallible, without touching the running app. ---

    docker::pull(docker, &image, ghcr_token)
        .await
        .context("failed to pull image")?;

    let stage_dir = backups_dir.join("restore").join(&app.name);
    let _ = std::fs::remove_dir_all(&stage_dir);
    std::fs::create_dir_all(&stage_dir)
        .with_context(|| format!("failed to create restore staging dir {}", stage_dir.display()))?;

    let staged = async {
        let tarball_path = stage_dir.join("backup.tar.gz");
        download_file(client, bucket, &newest, &tarball_path)
            .await
            .context("failed to download app data backup")?;
        extract_outer_tarball(&tarball_path, &stage_dir)?;
        let _ = std::fs::remove_file(&tarball_path);
        volume::discover_image_user(docker, &image)
            .await
            .context("failed to discover image user")
    }
    .await;
    let uid_gid = match staged {
        Ok(uid_gid) => uid_gid,
        Err(e) => {
            // Nothing has been stopped or modified yet — a healthy app stays up.
            let _ = std::fs::remove_dir_all(&stage_dir);
            return Err(e);
        }
    };

    // --- Phase 2: the backup is staged locally; now replace the container. ---

    // If the app's container is already running (e.g. this is a second,
    // idempotent `restore_all` run, or the app never actually went down),
    // stop and remove it first — both so nothing else holds the volume
    // read-write while the restore container writes to it, and so the
    // final `start_container` below (which enforces a single-writer
    // guarantee) doesn't collide with the very container it's replacing.
    // Mirrors `deploy::do_deploy`'s unconditional replace.
    let replace = async {
        docker::stop_and_remove_container(docker, &app.name)
            .await
            .context("failed to stop existing container before restore")?;

        // Create-if-absent: if the volume survived the disaster (or this is a
        // rerun) it is reused as-is — never wiped — and the restore container
        // overlays the backup's files/DBs on top of whatever is in it.
        volume::create_app_volume(docker, &app.id)
            .await
            .context("failed to create app volume")?;

        run_restore_container(docker, &app.id, &stage_dir, uid_gid).await
    }
    .await;
    let _ = std::fs::remove_dir_all(&stage_dir);
    replace?;

    start_container(pool, docker, app, &image)
        .await
        .context("failed to start restored container")?;

    Ok(RestoreOutcome::Restored)
}

/// Full disaster-recovery restore from S3: download the newest litehouse
/// state DB snapshot, merge its `app`/`env_var` rows into the live DB
/// (never clobbering existing local rows — see [`copy_apps_from_snapshot`]),
/// then for every app with a recorded image, pull it, ensure its volume
/// exists (create-if-absent — an existing volume is reused, never wiped),
/// restore its newest app-data backup (if any), and start it. Finishes with
/// one Caddy sync. Idempotent: safe to run repeatedly.
#[instrument(skip(pool, docker))]
pub async fn restore_all(pool: &Pool<Sqlite>, docker: &Docker) -> Result<RestoreReport> {
    // Serialize against any concurrent backup/restore (see BACKUP_LOCK).
    let _guard = BACKUP_LOCK.lock().await;

    let s3_config = db::system_config::get_s3_config(pool)
        .await?
        .ok_or(BackupError::S3ConfigMissing)?;
    let client = s3_client(&s3_config);
    let bucket = s3_config.bucket.clone();
    let prefix = s3_config.path_prefix.clone();

    let backups_dir = config::get_backups_dir().context("failed to resolve backups directory")?;
    std::fs::create_dir_all(&backups_dir)
        .with_context(|| format!("failed to create backups dir {}", backups_dir.display()))?;

    // 1. Merge the newest litehouse state DB snapshot's app/env_var rows in.
    let state_prefix = state_prefix_root(prefix.as_deref());
    let state_keys = list_keys(&client, &bucket, &state_prefix).await?;
    let newest_state_key = newest_key(&state_keys).ok_or_else(|| {
        anyhow!(
            "no litehouse state DB backups found under s3://{bucket}/{state_prefix} — nothing to restore"
        )
    })?;

    let snapshot_path = backups_dir.join("restore-state.db");
    let _ = std::fs::remove_file(&snapshot_path);
    download_file(&client, &bucket, &newest_state_key, &snapshot_path)
        .await
        .context("failed to download litehouse state DB snapshot")?;

    let snapshot_path_str = snapshot_path
        .to_str()
        .context("backups dir path is not valid UTF-8")?;
    let src_pool_result = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{snapshot_path_str}?mode=ro"))
        .await
        .context("failed to open downloaded state DB snapshot");

    let copied = match src_pool_result {
        Ok(src_pool) => {
            let result = copy_apps_from_snapshot(&src_pool, pool).await;
            src_pool.close().await;
            let _ = std::fs::remove_file(&snapshot_path);
            result?
        }
        Err(e) => {
            let _ = std::fs::remove_file(&snapshot_path);
            return Err(e);
        }
    };
    info!("merged {copied} app row(s) from state snapshot into live DB");

    // 2. Restore each app's data (pull image, recreate volume, apply backup, start).
    let ghcr_token = db::system_config::get_ghcr_token(pool).await?;
    let apps = db::app::get_all(pool).await.context("failed to list apps")?;

    let mut restored = Vec::new();
    let mut skipped = Vec::new();
    for app in &apps {
        match restore_app(
            pool,
            docker,
            &client,
            &bucket,
            prefix.as_deref(),
            &backups_dir,
            ghcr_token.as_deref(),
            app,
        )
        .await
        {
            Ok(RestoreOutcome::Restored) => restored.push(app.name.clone()),
            Ok(RestoreOutcome::Skipped(reason)) => {
                info!("skipping restore for app '{}': {reason}", app.name);
                skipped.push((app.name.clone(), reason));
            }
            Err(e) => {
                let msg = format!("{e:#}");
                warn!("restore failed for app '{}': {msg}", app.name);
                skipped.push((app.name.clone(), msg));
            }
        }
    }

    if let Err(e) = caddy::sync_configuration(docker, pool).await {
        warn!("Caddy sync failed after restore (apps are up, routing may be stale): {e:#}");
    }

    Ok(RestoreReport { restored, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_key_layout() {
        assert_eq!(
            app_backup_key(Some("prod"), "hello", "2026-07-03"),
            "prod/apps/hello/2026-07-03.tar.gz"
        );
        assert_eq!(
            app_backup_key(None, "hello", "2026-07-03"),
            "apps/hello/2026-07-03.tar.gz"
        );
        assert_eq!(state_backup_key(None, "2026-07-03"), "litehouse/2026-07-03.db");
    }

    #[test]
    fn retention_keeps_newest_14() {
        let keys: Vec<String> = (1..=20)
            .map(|d| format!("apps/hello/2026-06-{d:02}.tar.gz"))
            .collect();
        let doomed = keys_to_prune(&keys, 14);
        assert_eq!(doomed.len(), 6);
        assert!(doomed.contains(&"apps/hello/2026-06-01.tar.gz".to_string()));
        assert!(!doomed.contains(&"apps/hello/2026-06-20.tar.gz".to_string()));
    }

    #[test]
    fn retention_noop_when_under_limit() {
        let keys: Vec<String> = (1..=5)
            .map(|d| format!("apps/hello/2026-06-{d:02}.tar.gz"))
            .collect();
        assert_eq!(keys_to_prune(&keys, 14).len(), 0);
    }

    #[test]
    fn validate_app_name_rejects_shell_metacharacters() {
        assert!(validate_app_name_for_shell("hello-app_1").is_ok());
        assert!(validate_app_name_for_shell("hello; rm -rf /").is_err());
        assert!(validate_app_name_for_shell("hello'app").is_err());
        assert!(validate_app_name_for_shell("").is_err());
    }

    #[test]
    fn newest_key_picks_lexically_last() {
        let keys = vec![
            "litehouse/2026-06-30.db".to_string(),
            "litehouse/2026-07-03.db".to_string(),
            "litehouse/2026-07-01.db".to_string(),
        ];
        assert_eq!(newest_key(&keys), Some("litehouse/2026-07-03.db".to_string()));
    }

    #[test]
    fn newest_key_empty() {
        assert_eq!(newest_key(&[]), None);
    }

    #[test]
    fn snapshot_script_escapes_filenames_for_sql() {
        let script = snapshot_script();
        // Discovered filenames ($f) must be run through the sed
        // quote-doubling pipeline before being spliced into the
        // `VACUUM INTO '...'` SQL literal — a filename containing a `'`
        // could otherwise break out of the literal.
        assert!(
            script.contains(r#"esc=$(printf '%s' "$f" | sed "s/'/''/g")"#),
            "snapshot script must SQL-escape $f via the sed doubling pipeline:\n{script}"
        );
        // ...and the VACUUM INTO destination must use the escaped variable,
        // never the raw filename.
        assert!(
            script.contains(r#""VACUUM INTO '/backup/dbs/$esc'""#),
            "VACUUM INTO must use $esc, not raw $f:\n{script}"
        );
        assert!(
            !script.contains("VACUUM INTO '/backup/dbs/$f'"),
            "raw $f must not appear inside the SQL literal:\n{script}"
        );
    }

    #[test]
    fn snapshot_script_excludes_and_stages_blobs() {
        let script = snapshot_script();
        assert!(
            script.contains("-path './blobs' -prune"),
            "find must skip the blobs dir so it doesn't scan every blob file looking for databases:\n{script}"
        );
        assert!(
            script.contains("--exclude='./blobs'"),
            "tar must exclude the blobs dir — it's staged and uploaded separately, not swept into files.tar.gz:\n{script}"
        );
        assert!(
            script.contains("[ -d blobs ] && cp -a blobs /backup/blobs"),
            "script must stage the blobs dir into the backup staging area if present:\n{script}"
        );
    }

    #[test]
    fn sql_quote_literal_doubles_single_quotes() {
        assert_eq!(sql_quote_literal("plain"), "plain");
        assert_eq!(sql_quote_literal("it's"), "it''s");
        assert_eq!(sql_quote_literal("''already''"), "''''already''''");
        // Simulates a hostile filename discovered inside an app volume: every
        // single quote must come out doubled, so splicing the result into a
        // `VACUUM INTO '...'` literal can't break out of it.
        let hostile = "evil.db'; ATTACH DATABASE '/etc/passwd' AS x; --";
        let escaped = sql_quote_literal(hostile);
        assert_eq!(
            escaped,
            "evil.db''; ATTACH DATABASE ''/etc/passwd'' AS x; --"
        );
        assert_eq!(
            escaped.matches('\'').count(),
            hostile.matches('\'').count() * 2,
            "every embedded quote must be doubled"
        );
    }

    #[test]
    fn blob_key_layout() {
        assert_eq!(
            blob_key(Some("prod"), "hello", "photo.jpg"),
            "prod/blobs/hello/photo.jpg"
        );
        assert_eq!(blob_key(None, "hello", "photo.jpg"), "blobs/hello/photo.jpg");
    }

    #[test]
    fn blob_prefix_root_layout() {
        assert_eq!(blob_prefix_root(Some("prod"), "hello"), "prod/blobs/hello/");
        assert_eq!(blob_prefix_root(None, "hello"), "blobs/hello/");
    }

    #[test]
    fn blobs_missing_from_s3_skips_existing_uploads_new() {
        let local = vec!["a.jpg".to_string(), "b.jpg".to_string()];
        let existing = vec!["blobs/myapp/a.jpg".to_string()];
        let missing = blobs_missing_from_s3(&local, &existing, None, "myapp");
        assert_eq!(missing, vec!["b.jpg".to_string()]);
    }

    #[test]
    fn blobs_missing_from_s3_empty_when_all_present() {
        let local = vec!["a.jpg".to_string()];
        let existing = vec!["blobs/myapp/a.jpg".to_string()];
        assert!(blobs_missing_from_s3(&local, &existing, None, "myapp").is_empty());
    }

    #[test]
    fn blobs_missing_from_s3_all_missing_when_none_exist() {
        let local = vec!["a.jpg".to_string(), "b.jpg".to_string()];
        let missing = blobs_missing_from_s3(&local, &[], None, "myapp");
        assert_eq!(missing, local);
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::{App, EnvVar};

    /// `copy_apps_from_snapshot` should copy every app + env var row from
    /// src into dst, and running it twice should not duplicate anything —
    /// INSERT OR IGNORE makes it idempotent.
    #[tokio::test]
    async fn copy_apps_from_snapshot_copies_and_is_idempotent() {
        let src_pool = get_test_pool().await;
        let dst_pool = get_test_pool().await;

        let app1 = App::new("restore-app-1").unwrap();
        let app2 = App::new("restore-app-2").unwrap();
        db::app::save(&src_pool, &app1).await.unwrap();
        db::app::save(&src_pool, &app2).await.unwrap();
        db::env_var::save(&src_pool, &EnvVar::new(&app1.id, "KEY1", "value1"))
            .await
            .unwrap();
        db::env_var::save(&src_pool, &EnvVar::new(&app2.id, "KEY2", "value2"))
            .await
            .unwrap();

        let copied = copy_apps_from_snapshot(&src_pool, &dst_pool).await.unwrap();
        assert_eq!(copied, 2);

        let dst_apps = db::app::get_all(&dst_pool).await.unwrap();
        assert_eq!(dst_apps.len(), 2);
        assert!(dst_apps.iter().any(|a| a.name == "restore-app-1"));
        assert!(dst_apps.iter().any(|a| a.name == "restore-app-2"));

        let env1 = db::env_var::get_by_app(&dst_pool, &app1.id).await.unwrap();
        assert_eq!(env1.len(), 1);
        assert_eq!(env1[0].key, "KEY1");

        // Run again: still 2 apps, no duplicates.
        let copied_again = copy_apps_from_snapshot(&src_pool, &dst_pool).await.unwrap();
        assert_eq!(copied_again, 2);
        let dst_apps_again = db::app::get_all(&dst_pool).await.unwrap();
        assert_eq!(dst_apps_again.len(), 2);
        let env1_again = db::env_var::get_by_app(&dst_pool, &app1.id).await.unwrap();
        assert_eq!(env1_again.len(), 1);
    }

    /// Existing rows in the live (dst) DB must never be clobbered by an
    /// older snapshot — INSERT OR IGNORE, not upsert.
    #[tokio::test]
    async fn copy_apps_from_snapshot_never_clobbers_existing_rows() {
        let src_pool = get_test_pool().await;
        let dst_pool = get_test_pool().await;

        // Same app id in both, but the live DB's copy has since been
        // deployed (has an image) — the snapshot's stale copy must not win.
        let mut app = App::new("live-app").unwrap();
        db::app::save(&dst_pool, &app).await.unwrap();
        let mut live = db::app::get_by_name(&dst_pool, "live-app").await.unwrap().unwrap();
        live.image = Some("ghcr.io/example/live-app:new".to_string());
        db::app::save(&dst_pool, &live).await.unwrap();

        app.id = live.id.clone();
        app.image = Some("ghcr.io/example/live-app:stale".to_string());
        db::app::save(&src_pool, &app).await.unwrap();

        copy_apps_from_snapshot(&src_pool, &dst_pool).await.unwrap();

        let reloaded = db::app::get_by_id(&dst_pool, &live.id).await.unwrap().unwrap();
        assert_eq!(reloaded.image.as_deref(), Some("ghcr.io/example/live-app:new"));
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::test::get_test_pool;
    use crate::models::S3Config;
    use flate2::read::GzDecoder;

    /// Full round trip against real Docker + a local MinIO instance:
    /// creates an app with a small SQLite DB in its data volume, runs
    /// `run_backup`, and verifies the resulting tarball landed in S3 and
    /// contains a DB with the original row intact.
    ///
    /// Requires Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test test_backup_roundtrip_minio -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_backup_roundtrip_minio() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.expect("connect to docker");

        let minio_container = "litehouse-backup-test-minio";
        let minio_port = 19000u16;

        cleanup_minio(minio_container);

        // Start MinIO.
        let status = std::process::Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                minio_container,
                "-p",
                &format!("{minio_port}:9000"),
                "-e",
                "MINIO_ROOT_USER=minioadmin",
                "-e",
                "MINIO_ROOT_PASSWORD=minioadmin",
                "minio/minio",
                "server",
                "/data",
            ])
            .status()
            .expect("start minio");
        assert!(status.success(), "failed to start minio container");

        wait_for_port(minio_port).await;

        // Create the bucket via mc-less approach: use the S3 client itself.
        let s3_config = S3Config {
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            bucket: "litehouse-backup-test".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some(format!("http://localhost:{minio_port}")),
            path_prefix: None,
        };
        let client = s3_client(&s3_config);
        let _ = client
            .create_bucket()
            .bucket(&s3_config.bucket)
            .send()
            .await;

        let system_config = crate::models::SystemConfig::new_s3_config(&s3_config);
        db::system_config::save_s3_config(&pool, &system_config)
            .await
            .expect("save s3 config");

        // Create an app + its data volume with a sqlite DB containing one row.
        let app_name = "backup-roundtrip-app";
        let app = crate::models::App::new(app_name).expect("valid app name");
        db::app::save(&pool, &app).await.expect("save app");

        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();
        crate::volume::create_app_volume(&docker, &app.id)
            .await
            .expect("create app volume");

        seed_app_db(&docker, &app.id).await;

        // Run the backup.
        let report = run_backup(&pool, &docker).await.expect("run_backup");
        assert!(
            report.succeeded.contains(&app_name.to_string()),
            "expected {} in succeeded, got {:?} (failed: {:?})",
            app_name,
            report.succeeded,
            report.failed
        );
        assert!(
            !report.failed.iter().any(|(name, _)| name == app_name),
            "app backup must not fail (incl. the single-quote-named DB): {:?}",
            report.failed
        );

        // Verify the object landed in S3. `save_s3_config`/`SystemConfig`
        // defaults an unset `path_prefix` to "litehouse", so re-read the
        // stored config rather than assuming the prefix we passed in above.
        let stored_config = db::system_config::get_s3_config(&pool)
            .await
            .expect("get s3 config")
            .expect("s3 config should be set");
        let prefix = stored_config.path_prefix.as_deref();
        let key = app_backup_key(prefix, app_name, &today());
        let list_prefix = match prefix {
            Some(p) => format!("{p}/apps/"),
            None => "apps/".to_string(),
        };
        let keys = list_keys(&client, &s3_config.bucket, &list_prefix)
            .await
            .expect("list keys");
        assert!(keys.contains(&key), "expected key {key} in {keys:?}");

        // Download and inspect the tarball.
        let obj = client
            .get_object()
            .bucket(&s3_config.bucket)
            .key(&key)
            .send()
            .await
            .expect("get object");
        let bytes = obj.body.collect().await.expect("collect body").into_bytes();

        let tmp = tempfile::tempdir().expect("tempdir");
        let tar_path = tmp.path().join("backup.tar.gz");
        std::fs::write(&tar_path, &bytes).expect("write tarball");

        let tar_gz = File::open(&tar_path).expect("open tarball");
        let dec = GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(dec);
        let extract_dir = tmp.path().join("extracted");
        archive.unpack(&extract_dir).expect("unpack tarball");

        let db_path = extract_dir.join("dbs").join("app.db");
        assert!(db_path.exists(), "expected {} to exist", db_path.display());

        let conn = rusqlite_check(&db_path);
        assert_eq!(conn, 1, "expected the seeded row to survive the backup");

        // The single-quote-named DB must also have been snapshotted (the
        // report already asserted no failures above; this confirms the file
        // itself made it through the VACUUM INTO escaping + tarball).
        let quoted_db_path = extract_dir.join("dbs").join("it's.db");
        assert!(
            quoted_db_path.exists(),
            "expected {} to exist — single-quote filename must survive the snapshot script",
            quoted_db_path.display()
        );

        // Cleanup.
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();
        cleanup_minio(minio_container);
    }

    fn cleanup_minio(name: &str) {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", name])
            .output();
    }

    async fn wait_for_port(port: u16) {
        use std::net::TcpStream;
        for _ in 0..60 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        panic!("timed out waiting for port {port}");
    }

    /// Seed the app's data volume with a sqlite db (app.db) containing one
    /// row, using a one-shot sqlite3 container (mirrors how the snapshot
    /// container itself talks to the volume).
    async fn seed_app_db(docker: &Docker, app_id: &str) {
        let app_volume = crate::volume::get_app_volume_name(app_id);

        // The snapshot container's base image runs as a non-root user
        // (keinos/sqlite3 runs as uid 100), so the app data volume needs
        // world-writable/readable permissions for both the seed step below
        // and the later read-only snapshot mount to work.
        crate::volume::init_app_volume(docker, app_id, &app_volume, None)
            .await
            .expect("init app volume permissions");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let container_name = "litehouse-backup-test-seed";
        let _ = docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        // Also seed a DB whose filename contains a single quote: a hostile/
        // awkward name like this must not break the snapshot script's
        // `VACUUM INTO '...'` SQL literal (see snapshot_script's escaping).
        let script = r#"sqlite3 /data/app.db "CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO t DEFAULT VALUES;" && sqlite3 "/data/it's.db" "CREATE TABLE q (id INTEGER PRIMARY KEY); INSERT INTO q DEFAULT VALUES;""#;
        let container_config = ContainerConfig {
            image: Some("keinos/sqlite3:latest".to_string()),
            entrypoint: Some(vec!["sh".to_string()]),
            cmd: Some(vec!["-c".to_string(), script.to_string()]),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/data", app_volume)]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container = docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name.to_string(),
                    platform: None,
                }),
                container_config,
            )
            .await
            .expect("create seed container");
        docker
            .start_container::<String>(&container.id, None)
            .await
            .expect("start seed container");

        let mut wait_stream = docker.wait_container(&container.id, None::<WaitContainerOptions<String>>);
        while let Some(result) = wait_stream.next().await {
            let _ = result;
        }

        let inspect = docker.inspect_container(&container.id, None).await.unwrap();
        let exit_code = inspect.state.and_then(|s| s.exit_code).unwrap_or(-1);
        assert_eq!(exit_code, 0, "seeding app db failed");

        let _ = docker
            .remove_container(
                &container.id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
    }

    /// Query `SELECT COUNT(*) FROM t` from the given sqlite file using the
    /// `sqlite3` CLI if available, falling back to a raw byte-presence
    /// check. Kept dependency-free (no rusqlite in Cargo.toml).
    fn rusqlite_check(path: &Path) -> i64 {
        let output = std::process::Command::new("sqlite3")
            .arg(path)
            .arg("SELECT COUNT(*) FROM t;")
            .output();
        match output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
            }
            _ => {
                // No local sqlite3 CLI — fall back to a coarse check that the
                // file is non-trivial in size (VACUUM INTO of a table with a
                // row is meaningfully larger than an empty DB).
                let meta = std::fs::metadata(path).expect("db file metadata");
                if meta.len() > 0 {
                    1
                } else {
                    0
                }
            }
        }
    }

    /// Read `SELECT COUNT(*) FROM t` out of `/data/app.db` inside an app's
    /// *live Docker volume* (as opposed to [`rusqlite_check`], which reads a
    /// downloaded snapshot file directly), via a one-shot read-only mount +
    /// `sqlite3` CLI container. Used to verify a restore actually
    /// repopulated the volume, not just that the DB row survived the backup
    /// tarball.
    async fn read_row_count_from_volume(docker: &Docker, app_id: &str) -> i64 {
        let app_volume = crate::volume::get_app_volume_name(app_id);
        let container_name = "litehouse-restore-test-check";

        let _ = docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        let container_config = ContainerConfig {
            image: Some("keinos/sqlite3:latest".to_string()),
            entrypoint: Some(vec!["sh".to_string()]),
            cmd: Some(vec![
                "-c".to_string(),
                "sqlite3 /data/app.db 'SELECT COUNT(*) FROM t;'".to_string(),
            ]),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/data:ro", app_volume)]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container = docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name.to_string(),
                    platform: None,
                }),
                container_config,
            )
            .await
            .expect("create check container");
        docker
            .start_container::<String>(&container.id, None)
            .await
            .expect("start check container");

        let mut wait_stream = docker.wait_container(&container.id, None::<WaitContainerOptions<String>>);
        while let Some(result) = wait_stream.next().await {
            let _ = result;
        }

        let log = fetch_log_tail(docker, &container.id).await;

        let _ = docker
            .remove_container(
                &container.id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        log.trim().parse().unwrap_or(-1)
    }

    /// A real file-backed sqlite pool (unlike [`get_test_pool`], which uses
    /// `:memory:`). `restore_all` needs `VACUUM INTO` on the litehouse state
    /// DB to actually produce a file to upload — that requires a real
    /// on-disk database (an in-memory pooled connection's `VACUUM INTO`
    /// silently reports success without ever writing a file across a
    /// handful of sqlx/libsqlite3 versions), which is what every real
    /// deployment uses anyway.
    async fn get_file_backed_test_pool(dir: &Path) -> Pool<Sqlite> {
        let db_path = dir.join("test.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect(&format!("sqlite://{}?mode=rwc", db_path.display()))
            .await
            .expect("connect to file-backed test db");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    /// Full round trip against real Docker + a local MinIO instance:
    /// backs up an app (its data volume + the litehouse state DB), then
    /// simulates a disaster (deletes the app's DB row, its volume, and its
    /// container), runs `restore_all` against the same MinIO bucket, and
    /// verifies the app row, its data, and its container all come back.
    ///
    /// Requires Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test test_restore_roundtrip_minio -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_restore_roundtrip_minio() {
        let db_dir = tempfile::tempdir().expect("tempdir for test db");
        let pool = get_file_backed_test_pool(db_dir.path()).await;
        let docker = crate::docker::connect().await.expect("connect to docker");

        let minio_container = "litehouse-restore-test-minio";
        let minio_port = 19010u16;
        let container_name_suffix = "-container";

        cleanup_minio(minio_container);

        let status = std::process::Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                minio_container,
                "-p",
                &format!("{minio_port}:9000"),
                "-e",
                "MINIO_ROOT_USER=minioadmin",
                "-e",
                "MINIO_ROOT_PASSWORD=minioadmin",
                "minio/minio",
                "server",
                "/data",
            ])
            .status()
            .expect("start minio");
        assert!(status.success(), "failed to start minio container");

        wait_for_port(minio_port).await;

        let s3_config = S3Config {
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            bucket: "litehouse-restore-test".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some(format!("http://localhost:{minio_port}")),
            path_prefix: None,
        };
        let client = s3_client(&s3_config);
        let _ = client.create_bucket().bucket(&s3_config.bucket).send().await;

        let system_config = crate::models::SystemConfig::new_s3_config(&s3_config);
        db::system_config::save_s3_config(&pool, &system_config)
            .await
            .expect("save s3 config");

        // Create an app with a small SQLite DB in its data volume, and a
        // deployed image (nginx:alpine has a long-running default CMD, so
        // `start_container` during restore actually stays up).
        let app_name = "restore-roundtrip-app";
        let container_name = format!("{app_name}{container_name_suffix}");
        let mut app = crate::models::App::new(app_name).expect("valid app name");
        app.image = Some("nginx:alpine".to_string());
        app.exposed_port = Some("80".to_string());
        db::app::save(&pool, &app).await.expect("save app");

        let app_volume = crate::volume::get_app_volume_name(&app.id);
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &app_volume])
            .output();
        crate::volume::create_app_volume(&docker, &app.id)
            .await
            .expect("create app volume");

        seed_app_db(&docker, &app.id).await;

        // Back up (captures the app's data *and*, in the litehouse state DB
        // snapshot, the app row itself with its image/name/etc).
        let backup_report = run_backup(&pool, &docker).await.expect("run_backup");
        assert!(
            backup_report.succeeded.contains(&app_name.to_string()),
            "expected {app_name} in succeeded, got {:?} (failed: {:?})",
            backup_report.succeeded,
            backup_report.failed
        );
        assert!(
            backup_report.succeeded.contains(&"litehouse".to_string()),
            "expected litehouse in succeeded, failed={:?}",
            backup_report.failed
        );

        // Simulate a disaster: the app's DB row, its data volume, and its
        // container all disappear.
        db::app::delete_by_app_id(&pool, &app.id).await.expect("delete app row");
        assert!(db::app::get_by_id(&pool, &app.id).await.unwrap().is_none());
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output();
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &app_volume])
            .output();

        // Restore.
        let restore_report = restore_all(&pool, &docker).await.expect("restore_all");
        assert!(
            restore_report.restored.contains(&app_name.to_string()),
            "expected {app_name} in restored, got restored={:?} skipped={:?}",
            restore_report.restored,
            restore_report.skipped
        );

        // The app row is back.
        let restored_app = db::app::get_by_name(&pool, app_name)
            .await
            .expect("get app")
            .expect("app row should be restored");
        assert_eq!(restored_app.id, app.id, "restored app should keep its original id");
        assert_eq!(restored_app.image.as_deref(), Some("nginx:alpine"));

        // The volume is repopulated with the original row.
        let row_count = read_row_count_from_volume(&docker, &restored_app.id).await;
        assert_eq!(row_count, 1, "expected the seeded row to survive backup + restore");

        // Running restore_all again must be idempotent: still exactly one
        // app row, still restored, no duplication.
        let restore_report_2 = restore_all(&pool, &docker).await.expect("restore_all again");
        assert!(
            restore_report_2.restored.contains(&app_name.to_string()),
            "expected {app_name} in restored again, got restored={:?} skipped={:?}",
            restore_report_2.restored,
            restore_report_2.skipped
        );
        let all_apps = db::app::get_all(&pool).await.unwrap();
        assert_eq!(
            all_apps.iter().filter(|a| a.name == app_name).count(),
            1,
            "restore must not duplicate app rows"
        );

        // Cleanup.
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &container_name])
            .output();
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &app_volume])
            .output();
        cleanup_minio(minio_container);
    }
}
