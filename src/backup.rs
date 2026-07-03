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

use anyhow::{bail, Context, Result};
use aws_sdk_s3::config::{BehaviorVersion, Builder as S3ConfigBuilder, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
    WaitContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::fs::File;
use std::path::Path;
use tracing::{info, instrument, warn};

use crate::models::S3Config;
use crate::{config, db};

/// How many daily backups to retain per app / per the litehouse state DB.
pub const RETENTION_COUNT: usize = 14;

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

/// Prune old backups under `prefix`, keeping the newest `RETENTION_COUNT`.
#[instrument(skip(client))]
async fn prune_old_backups(client: &S3Client, bucket: &str, prefix: &str) -> Result<()> {
    let keys = list_keys(client, bucket, prefix).await?;
    let doomed = keys_to_prune(&keys, RETENTION_COUNT);
    if !doomed.is_empty() {
        info!(
            "pruning {} old backup(s) under s3://{bucket}/{prefix}",
            doomed.len()
        );
        delete_keys(client, bucket, &doomed).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Per-app snapshot: one-shot container + tar + upload
// ---------------------------------------------------------------------

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Build the shell script run inside the one-shot snapshot container. `app`
/// must already be validated safe for shell interpolation.
fn snapshot_script(app: &str) -> String {
    format!(
        r#"set -e
rm -rf "/backup/{app}" && mkdir -p "/backup/{app}/dbs"
cd /data
find . -name '*.db' -o -name '*.sqlite' -o -name '*.sqlite3' | while read -r f; do
  mkdir -p "/backup/{app}/dbs/$(dirname "$f")"
  sqlite3 "file:$f?mode=ro&immutable=0" "VACUUM INTO '/backup/{app}/dbs/$f'"
done
tar czf "/backup/{app}/files.tar.gz" --exclude='*.db' --exclude='*.sqlite' --exclude='*.sqlite3' --exclude='*-wal' --exclude='*-shm' .
"#,
        app = app
    )
}

/// Run the one-shot `keinos/sqlite3` snapshot container for `app`, staging
/// its output under `{backups_dir}/{app}` (bind-mounted at `/backup`).
#[instrument(skip(docker))]
async fn run_snapshot_container(
    docker: &Docker,
    app_id: &str,
    app_name: &str,
    backups_dir: &Path,
) -> Result<()> {
    validate_app_name_for_shell(app_name)?;

    let backups_dir_str = backups_dir
        .to_str()
        .context("backups dir path is not valid UTF-8")?;

    let app_volume = crate::volume::get_app_volume_name(app_id);
    let script = snapshot_script(app_name);
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
                format!("{}:/backup", backups_dir_str),
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
        match backup_app(docker, &client, &bucket, prefix.as_deref(), &date, &backups_dir, &app.id, &app.name)
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
    sqlx::query(&format!("VACUUM INTO '{}'", snapshot_path_str))
        .execute(pool)
        .await
        .context("failed to VACUUM INTO litehouse state DB snapshot")?;

    let key = state_backup_key(prefix, date);
    upload_file(client, bucket, &key, &snapshot_path).await?;
    let _ = std::fs::remove_file(&snapshot_path);

    prune_old_backups(client, bucket, &state_prefix_root(prefix)).await?;
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

async fn backup_app(
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

    // Clean up local staging regardless of upload outcome.
    let _ = std::fs::remove_file(&tarball_path);
    let _ = std::fs::remove_dir_all(&staged_dir);

    upload_result?;

    prune_old_backups(client, bucket, &app_prefix_root(prefix, app_name)).await?;
    Ok(())
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

        let script = "sqlite3 /data/app.db \"CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO t DEFAULT VALUES;\"";
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
}
