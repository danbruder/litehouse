# Incremental Blob Backup for App Data — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every litehouse app a `LITEHOUSE_BLOB_PATH`-designated directory on its data volume whose contents back up to their own S3 prefix incrementally (upload-once, skip-if-already-present) instead of riding along in the existing daily full tarball.

**Architecture:** Single new module of pure helpers + two small integration points in the existing backup engine (`src/backup.rs`) and app-start path (`src/commands/start.rs`). No new DB tables, no new CLI commands, no new migrations — see `docs/superpowers/specs/2026-07-14-blob-backup-design.md` for the full design rationale, especially why blob S3 keys must NOT live under the existing `apps/{app_name}/` prefix (it would corrupt the existing tarball retention/restore logic).

**Tech Stack:** Rust, `aws-sdk-s3`, `walkdir` (already a dependency), `bollard` (Docker), `sqlx`.

---

## Task 1: Pure blob-key helpers and constants

**Files:**
- Modify: `src/backup.rs` (near the top, alongside the existing `RETENTION_COUNT` const and `app_backup_key`/`app_prefix_root` helpers)

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` block in `src/backup.rs` (it currently ends after the `sql_quote_literal_doubles_single_quotes` test, right before the closing `}` of `mod tests`):

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test blob_key_layout blob_prefix_root_layout blobs_missing_from_s3`
Expected: FAIL to compile — `blob_key`, `blob_prefix_root`, `blobs_missing_from_s3` not found in this scope.

- [ ] **Step 3: Implement the helpers and constants**

Add near the top of `src/backup.rs`, directly after the existing `pub const RETENTION_COUNT: usize = 14;`:

```rust
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
```

Add directly after the existing `app_prefix_root` function (near `fn app_prefix_root`):

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test blob_key_layout blob_prefix_root_layout blobs_missing_from_s3`
Expected: PASS (5 tests: `blob_key_layout`, `blob_prefix_root_layout`, `blobs_missing_from_s3_skips_existing_uploads_new`, `blobs_missing_from_s3_empty_when_all_present`, `blobs_missing_from_s3_all_missing_when_none_exist`).

- [ ] **Step 5: Commit**

```bash
git add src/backup.rs
git commit -m "feat: add blob key layout and diff helpers for incremental blob backup"
```

---

## Task 2: `LITEHOUSE_BLOB_PATH` env var on every app start

**Files:**
- Modify: `src/commands/start.rs`

- [ ] **Step 1: Write the failing tests**

`src/commands/start.rs` currently has no `#[cfg(test)]` module. Add one at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EnvVar;

    #[test]
    fn ensure_blob_path_env_var_adds_default_when_absent() {
        let env_vars = ensure_blob_path_env_var(vec![], "app-1");
        assert_eq!(env_vars.len(), 1);
        assert_eq!(env_vars[0].key, "LITEHOUSE_BLOB_PATH");
        assert_eq!(env_vars[0].value, "/data/blobs");
    }

    #[test]
    fn ensure_blob_path_env_var_respects_explicit_override() {
        let explicit = EnvVar::new("app-1", "LITEHOUSE_BLOB_PATH", "/data/custom-blobs");
        let env_vars = ensure_blob_path_env_var(vec![explicit], "app-1");
        assert_eq!(env_vars.len(), 1);
        assert_eq!(env_vars[0].value, "/data/custom-blobs");
    }

    #[test]
    fn ensure_blob_path_env_var_leaves_other_vars_untouched() {
        let other = EnvVar::new("app-1", "SOME_OTHER_KEY", "some-value");
        let env_vars = ensure_blob_path_env_var(vec![other], "app-1");
        assert_eq!(env_vars.len(), 2);
        assert!(env_vars.iter().any(|e| e.key == "SOME_OTHER_KEY"));
        assert!(env_vars.iter().any(|e| e.key == "LITEHOUSE_BLOB_PATH"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ensure_blob_path_env_var`
Expected: FAIL to compile — `ensure_blob_path_env_var` not found in this scope.

- [ ] **Step 3: Implement the helper and wire it into `start_container`**

Change the imports at the top of `src/commands/start.rs` from:

```rust
use crate::models::{App, AppState};
```

to:

```rust
use crate::backup::{BLOB_MOUNT_PATH, BLOB_PATH_ENV_VAR};
use crate::models::{App, AppState, EnvVar};
```

Add this function above `start_container`:

```rust
/// Give every app a stable place to write incrementally-backed-up blobs
/// (see `backup` module docs and `docs/superpowers/specs/2026-07-14-blob-backup-design.md`)
/// without hardcoding the path. Only appends the default if the app hasn't
/// explicitly set its own value for this key via `lh env set`.
fn ensure_blob_path_env_var(mut env_vars: Vec<EnvVar>, app_id: &str) -> Vec<EnvVar> {
    if !env_vars.iter().any(|e| e.key == BLOB_PATH_ENV_VAR) {
        env_vars.push(EnvVar::new(app_id, BLOB_PATH_ENV_VAR, BLOB_MOUNT_PATH));
    }
    env_vars
}
```

In `start_container`, change:

```rust
    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| StartError::DatabaseError(e.to_string()))?;

    tracing::info!("Found {} environment variables", env_vars.len());
```

to:

```rust
    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| StartError::DatabaseError(e.to_string()))?;

    tracing::info!("Found {} environment variables", env_vars.len());
    let env_vars = ensure_blob_path_env_var(env_vars, &app.id);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test ensure_blob_path_env_var`
Expected: PASS (3 tests).

- [ ] **Step 5: Run the full test suite to check for regressions**

Run: `cargo test`
Expected: PASS (no regressions; this only adds one env var to every app's environment).

- [ ] **Step 6: Commit**

```bash
git add src/commands/start.rs
git commit -m "feat: inject LITEHOUSE_BLOB_PATH env var into every app container"
```

---

## Task 3: Exclude and stage the blob directory in the snapshot script

**Files:**
- Modify: `src/backup.rs` (`snapshot_script` function and its test)

- [ ] **Step 1: Write the failing test**

The existing test `snapshot_script_escapes_filenames_for_sql` already asserts on parts of `snapshot_script()`'s output. Add a new, separate test right after it in the same `mod tests` block:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test snapshot_script_excludes_and_stages_blobs`
Expected: FAIL — assertions fail because `snapshot_script()` doesn't yet contain any of these strings.

- [ ] **Step 3: Update `snapshot_script`**

Change the `snapshot_script` function from:

```rust
fn snapshot_script() -> String {
    r#"set -e
rm -rf /backup/dbs /backup/files.tar.gz
mkdir -p /backup/dbs
cd /data
find . -name '*.db' -o -name '*.sqlite' -o -name '*.sqlite3' | while read -r f; do
  mkdir -p "/backup/dbs/$(dirname "$f")"
  esc=$(printf '%s' "$f" | sed "s/'/''/g")
  sqlite3 "file:$f?mode=ro&immutable=0" "VACUUM INTO '/backup/dbs/$esc'"
done
tar czf /backup/files.tar.gz --exclude='*.db' --exclude='*.sqlite' --exclude='*.sqlite3' --exclude='*-wal' --exclude='*-shm' .
"#
    .to_string()
}
```

to:

```rust
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
```

(The trailing `|| true` on the blobs line matters: under `set -e`, `A && B` as a bare script line aborts the whole script the moment `A` fails, even though `A` isn't the last command in the list — the *compound statement's own* exit status (1, since `A` short-circuited it) is what `-e` reacts to. Without `|| true`, every app that doesn't yet have a `/data/blobs` directory — which is every app initially — would have its entire snapshot (including the SQLite `VACUUM INTO` that already ran above this line) aborted here. `|| true` forces this line's exit status to always be 0.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test snapshot_script`
Expected: PASS — both `snapshot_script_excludes_and_stages_blobs` and the pre-existing `snapshot_script_escapes_filenames_for_sql` (make sure the SQL-escaping test still passes; the `find` line changed shape but still pipes through the same `while read -r f` / `esc=...` / `VACUUM INTO` sequence).

- [ ] **Step 5: Commit**

```bash
git add src/backup.rs
git commit -m "feat: exclude blobs dir from db-scan and files tarball, stage it separately"
```

---

## Task 4: Incremental blob upload in `backup_app`

**Files:**
- Modify: `src/backup.rs` (imports, new `backup_blobs` function, `backup_app`)

- [ ] **Step 1: Add the `walkdir` import**

At the top of `src/backup.rs`, add:

```rust
use walkdir::WalkDir;
```

- [ ] **Step 2: Implement `backup_blobs`**

Add this function directly after `backup_app` in `src/backup.rs`:

```rust
/// Upload every file under `{staged_dir}/blobs` whose S3 key doesn't already
/// exist, skipping the rest. A no-op if the app has no blobs directory.
/// Single `list_keys` call up front (not one HEAD per file) — see the
/// design doc for why this is safe given the write-once contract on blob
/// paths.
#[instrument(skip(client))]
async fn backup_blobs(
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    app_name: &str,
    staged_dir: &Path,
) -> Result<()> {
    let blobs_dir = staged_dir.join(BLOB_DIR_NAME);
    if !blobs_dir.exists() {
        return Ok(());
    }

    let local_relative_paths: Vec<String> = WalkDir::new(&blobs_dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(&blobs_dir)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string())
        })
        .collect();

    if local_relative_paths.is_empty() {
        return Ok(());
    }

    let existing_keys = list_keys(client, bucket, &blob_prefix_root(prefix, app_name)).await?;
    let to_upload = blobs_missing_from_s3(&local_relative_paths, &existing_keys, prefix, app_name);

    info!(
        "app '{app_name}': {} blob(s) already backed up, uploading {} new",
        local_relative_paths.len() - to_upload.len(),
        to_upload.len()
    );

    for relative_path in &to_upload {
        let key = blob_key(prefix, app_name, relative_path);
        let path = blobs_dir.join(relative_path);
        upload_file(client, bucket, &key, &path).await?;
    }

    Ok(())
}
```

- [ ] **Step 3: Wire it into `backup_app`**

Change `backup_app` from:

```rust
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
```

to:

```rust
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

    // Upload any blobs first, then drop the staged copy so the outer dated
    // tarball below doesn't also capture them — blobs get their own
    // incremental S3 prefix (`blob_prefix_root`), not the daily tarball.
    let blob_result = backup_blobs(client, bucket, prefix, app_name, &staged_dir).await;
    let _ = std::fs::remove_dir_all(staged_dir.join(BLOB_DIR_NAME));
    blob_result?;

    let tarball_path = backups_dir.join(format!("{app_name}-{date}.tar.gz"));
    tar_staged_dir(&staged_dir, &tarball_path)?;
```

The rest of `backup_app` (tarball upload, cleanup, catalog record, `prune_old_backups`) is unchanged.

- [ ] **Step 4: Run the full unit test suite to check for regressions**

Run: `cargo test`
Expected: PASS (this task adds no new unit tests of its own — `backup_blobs` is exercised by the integration test below, since it needs real S3/Docker).

- [ ] **Step 5: Write the MinIO integration test**

Add to the `integration_tests` module in `src/backup.rs`, after `test_backup_roundtrip_minio`:

```rust
    /// Verifies the incremental-upload behavior end to end: seeds a blob
    /// file in the app's data volume, runs `run_backup` twice, and checks
    /// (a) the object lands under `blobs/{app}/`, NOT `apps/{app}/`, and
    /// (b) the second run doesn't re-upload it (its S3 `last_modified`
    /// timestamp is unchanged).
    ///
    /// Requires Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test test_backup_blobs_incremental_minio -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_backup_blobs_incremental_minio() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.expect("connect to docker");

        let minio_container = "litehouse-backup-blobs-test-minio";
        let minio_port = 19001u16;
        cleanup_minio(minio_container);

        let status = std::process::Command::new("docker")
            .args([
                "run", "-d", "--rm", "--name", minio_container,
                "-p", &format!("{minio_port}:9000"),
                "-e", "MINIO_ROOT_USER=minioadmin",
                "-e", "MINIO_ROOT_PASSWORD=minioadmin",
                "minio/minio", "server", "/data",
            ])
            .status()
            .expect("start minio");
        assert!(status.success(), "failed to start minio container");
        wait_for_port(minio_port).await;

        let s3_config = S3Config {
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            bucket: "litehouse-blobs-test".to_string(),
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

        let app_name = "backup-blobs-test-app";
        let app = crate::models::App::new(app_name).expect("valid app name");
        db::app::save(&pool, &app).await.expect("save app");

        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();
        crate::volume::create_app_volume(&docker, &app.id)
            .await
            .expect("create app volume");
        crate::volume::init_app_volume(&docker, &app.id, &crate::volume::get_app_volume_name(&app.id), None)
            .await
            .expect("init volume permissions");

        // Seed a SQLite DB (so backup_app has something to do) and one blob file.
        seed_app_db(&docker, &app.id).await;
        seed_blob_file(&docker, &app.id, "photo.jpg", "fake-jpeg-bytes").await;

        // First backup run: the blob should be uploaded.
        let report = run_backup(&pool, &docker).await.expect("first run_backup");
        assert!(report.succeeded.contains(&app_name.to_string()), "first backup failed: {:?}", report.failed);

        let stored_config = db::system_config::get_s3_config(&pool).await.unwrap().unwrap();
        let prefix = stored_config.path_prefix.as_deref();
        let blob_prefix = blob_prefix_root(prefix, app_name);

        let keys_after_first = list_keys(&client, &s3_config.bucket, &blob_prefix).await.expect("list keys");
        assert_eq!(keys_after_first.len(), 1, "expected exactly one blob key, got {keys_after_first:?}");
        let expected_key = blob_key(prefix, app_name, "photo.jpg");
        assert!(keys_after_first.contains(&expected_key), "expected {expected_key} in {keys_after_first:?}");

        // Also assert the tarball backup did NOT capture the blob (it's excluded from files.tar.gz).
        let app_prefix = app_prefix_root(prefix, app_name);
        let tarball_keys = list_keys(&client, &s3_config.bucket, &app_prefix).await.expect("list app keys");
        assert_eq!(tarball_keys.len(), 1, "expected exactly one dated tarball, got {tarball_keys:?}");

        let last_modified_after_first = client
            .head_object()
            .bucket(&s3_config.bucket)
            .key(&expected_key)
            .send()
            .await
            .expect("head object")
            .last_modified()
            .cloned();

        // Second backup run: the same blob must NOT be re-uploaded.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let report2 = run_backup(&pool, &docker).await.expect("second run_backup");
        assert!(report2.succeeded.contains(&app_name.to_string()), "second backup failed: {:?}", report2.failed);

        let keys_after_second = list_keys(&client, &s3_config.bucket, &blob_prefix).await.expect("list keys");
        assert_eq!(keys_after_second.len(), 1, "blob count must not change on a second run");

        let last_modified_after_second = client
            .head_object()
            .bucket(&s3_config.bucket)
            .key(&expected_key)
            .send()
            .await
            .expect("head object")
            .last_modified()
            .cloned();
        assert_eq!(
            last_modified_after_first, last_modified_after_second,
            "blob must not be re-uploaded on the second backup run"
        );

        // Cleanup.
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();
        cleanup_minio(minio_container);
    }

    /// Seed a single blob file into the app's data volume at `/data/blobs/<name>`.
    async fn seed_blob_file(docker: &Docker, app_id: &str, name: &str, contents: &str) {
        let app_volume = crate::volume::get_app_volume_name(app_id);
        let container_name = "litehouse-backup-test-seed-blob";
        let _ = docker
            .remove_container(container_name, Some(RemoveContainerOptions { force: true, ..Default::default() }))
            .await;

        let script = format!("mkdir -p /data/blobs && printf '%s' '{contents}' > /data/blobs/{name}");
        let container_config = ContainerConfig {
            image: Some("keinos/sqlite3:latest".to_string()),
            entrypoint: Some(vec!["sh".to_string()]),
            cmd: Some(vec!["-c".to_string(), script]),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/data", app_volume)]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container = docker
            .create_container(Some(CreateContainerOptions { name: container_name.to_string(), platform: None }), container_config)
            .await
            .expect("create seed-blob container");
        docker.start_container::<String>(&container.id, None).await.expect("start seed-blob container");

        let mut wait_stream = docker.wait_container(&container.id, None::<WaitContainerOptions<String>>);
        while let Some(result) = wait_stream.next().await {
            let _ = result;
        }
        let inspect = docker.inspect_container(&container.id, None).await.unwrap();
        assert_eq!(inspect.state.and_then(|s| s.exit_code).unwrap_or(-1), 0, "seeding blob file failed");

        let _ = docker
            .remove_container(&container.id, Some(RemoveContainerOptions { force: true, ..Default::default() }))
            .await;
    }
```

- [ ] **Step 6: Run the integration test**

Run: `DOCKER_API_VERSION=1.42 cargo test test_backup_blobs_incremental_minio -- --ignored --nocapture`
Expected: PASS. Requires Docker running locally.

- [ ] **Step 7: Commit**

```bash
git add src/backup.rs
git commit -m "feat: upload new blobs incrementally, skip already-backed-up ones"
```

---

## Task 5: Restore blobs alongside the tarball

**Files:**
- Modify: `src/backup.rs` (`restore_app`, `run_restore_container`)

- [ ] **Step 1: Implement `restore_blobs`**

Add this function directly after `extract_outer_tarball` in `src/backup.rs`:

```rust
/// Download every object under this app's blob prefix into
/// `{stage_dir}/blobs/<relative_path>`, recreating the directory structure.
/// A no-op if the app has no blobs. Always downloads everything (restore is
/// a rare disaster-recovery path, not a daily job — no incrementality
/// needed here, unlike `backup_blobs`).
#[instrument(skip(client))]
async fn restore_blobs(
    client: &S3Client,
    bucket: &str,
    prefix: Option<&str>,
    app_name: &str,
    stage_dir: &Path,
) -> Result<()> {
    let blob_prefix = blob_prefix_root(prefix, app_name);
    let keys = list_keys(client, bucket, &blob_prefix).await?;
    if keys.is_empty() {
        return Ok(());
    }

    let blobs_dir = stage_dir.join(BLOB_DIR_NAME);
    for key in &keys {
        let relative_path = key
            .strip_prefix(&blob_prefix)
            .with_context(|| format!("blob key {key} missing expected prefix {blob_prefix}"))?;
        let dest = blobs_dir.join(relative_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        download_file(client, bucket, key, &dest).await?;
    }

    Ok(())
}
```

- [ ] **Step 2: Wire it into `restore_app`'s Phase 1**

Change the `staged` block in `restore_app` from:

```rust
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
```

to:

```rust
    let staged = async {
        let tarball_path = stage_dir.join("backup.tar.gz");
        download_file(client, bucket, &newest, &tarball_path)
            .await
            .context("failed to download app data backup")?;
        extract_outer_tarball(&tarball_path, &stage_dir)?;
        let _ = std::fs::remove_file(&tarball_path);

        restore_blobs(client, bucket, prefix, &app.name, &stage_dir)
            .await
            .context("failed to download blob backups")?;

        volume::discover_image_user(docker, &image)
            .await
            .context("failed to discover image user")
    }
    .await;
```

- [ ] **Step 3: Add the third restore leg to the restore container script**

In `run_restore_container`, change:

```rust
    let script = format!(
        r#"set -e
mkdir -p /data
if [ -f /restore/files.tar.gz ]; then tar xzf /restore/files.tar.gz -C /data; fi
if [ -d /restore/dbs ]; then cp -a /restore/dbs/. /data/; fi
{perm_fix}
"#
    );
```

to:

```rust
    let script = format!(
        r#"set -e
mkdir -p /data
if [ -f /restore/files.tar.gz ]; then tar xzf /restore/files.tar.gz -C /data; fi
if [ -d /restore/dbs ]; then cp -a /restore/dbs/. /data/; fi
if [ -d /restore/blobs ]; then mkdir -p /data/blobs && cp -a /restore/blobs/. /data/blobs/; fi
{perm_fix}
"#
    );
```

- [ ] **Step 4: Run the full unit test suite to check for regressions**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Write the MinIO restore integration test**

Add to the `integration_tests` module in `src/backup.rs`, after `test_backup_blobs_incremental_minio`:

```rust
    /// Full backup-then-restore round trip for blobs: seeds a blob, backs
    /// up, deletes the app's volume entirely (simulating disaster), then
    /// restores and checks the blob file is back at `/data/blobs/photo.jpg`
    /// with its original contents.
    ///
    /// Requires Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test test_restore_blobs_roundtrip_minio -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_restore_blobs_roundtrip_minio() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.expect("connect to docker");

        let minio_container = "litehouse-restore-blobs-test-minio";
        let minio_port = 19002u16;
        cleanup_minio(minio_container);

        let status = std::process::Command::new("docker")
            .args([
                "run", "-d", "--rm", "--name", minio_container,
                "-p", &format!("{minio_port}:9000"),
                "-e", "MINIO_ROOT_USER=minioadmin",
                "-e", "MINIO_ROOT_PASSWORD=minioadmin",
                "minio/minio", "server", "/data",
            ])
            .status()
            .expect("start minio");
        assert!(status.success(), "failed to start minio container");
        wait_for_port(minio_port).await;

        let s3_config = S3Config {
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            bucket: "litehouse-restore-blobs-test".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some(format!("http://localhost:{minio_port}")),
            path_prefix: None,
        };
        let client = s3_client(&s3_config);
        let _ = client.create_bucket().bucket(&s3_config.bucket).send().await;

        let system_config = crate::models::SystemConfig::new_s3_config(&s3_config);
        db::system_config::save_s3_config(&pool, &system_config).await.expect("save s3 config");

        let app_name = "restore-blobs-test-app";
        let mut app = crate::models::App::new(app_name).expect("valid app name");
        app.image = Some("alpine:3.20".to_string());
        db::app::save(&pool, &app).await.expect("save app");

        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();
        crate::volume::create_app_volume(&docker, &app.id).await.expect("create app volume");
        crate::volume::init_app_volume(&docker, &app.id, &crate::volume::get_app_volume_name(&app.id), None)
            .await
            .expect("init volume permissions");

        seed_app_db(&docker, &app.id).await;
        seed_blob_file(&docker, &app.id, "photo.jpg", "original-photo-bytes").await;

        let report = run_backup(&pool, &docker).await.expect("run_backup");
        assert!(report.succeeded.contains(&app_name.to_string()), "backup failed: {:?}", report.failed);

        // Simulate disaster: wipe the app's volume entirely.
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();

        let report = restore_all(&pool, &docker).await.expect("restore_all");
        assert!(report.restored.contains(&app_name.to_string()), "restore skipped/failed: {:?}", report.skipped);

        // Read back the restored blob file directly from the volume via a one-shot container.
        let read_container = "litehouse-restore-blobs-verify";
        let _ = docker
            .remove_container(read_container, Some(RemoveContainerOptions { force: true, ..Default::default() }))
            .await;
        let verify_config = ContainerConfig {
            image: Some("alpine:3.20".to_string()),
            cmd: Some(vec!["cat".to_string(), "/data/blobs/photo.jpg".to_string()]),
            host_config: Some(HostConfig {
                binds: Some(vec![format!("{}:/data:ro", crate::volume::get_app_volume_name(&app.id))]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let container = docker
            .create_container(Some(CreateContainerOptions { name: read_container.to_string(), platform: None }), verify_config)
            .await
            .expect("create verify container");
        docker.start_container::<String>(&container.id, None).await.expect("start verify container");

        let logs_options = Some(LogsOptions::<String> { stdout: true, stderr: true, tail: "10".to_string(), ..Default::default() });
        let mut stream = docker.logs(&container.id, logs_options);
        let mut out = String::new();
        while let Some(chunk) = stream.next().await {
            if let Ok(log) = chunk {
                out.push_str(&log.to_string());
            }
        }
        assert_eq!(out, "original-photo-bytes", "restored blob file contents must match what was backed up");

        let _ = docker
            .remove_container(&container.id, Some(RemoveContainerOptions { force: true, ..Default::default() }))
            .await;

        // Stop the restored app container and clean up.
        let _ = crate::docker::stop_and_remove_container(&docker, app_name).await;
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &crate::volume::get_app_volume_name(&app.id)])
            .output();
        cleanup_minio(minio_container);
    }
```

- [ ] **Step 6: Run the integration test**

Run: `DOCKER_API_VERSION=1.42 cargo test test_restore_blobs_roundtrip_minio -- --ignored --nocapture`
Expected: PASS. Requires Docker running locally.

- [ ] **Step 7: Commit**

```bash
git add src/backup.rs
git commit -m "feat: restore blobs from S3 alongside the dated tarball"
```

---

## Task 6: Update docs

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `VISION.md`

- [ ] **Step 1: Update `README.md`**

Find the existing backup description (starts with `- A built-in daily job snapshots every app's SQLite data...`). Add a new bullet directly after it:

```markdown
- Apps get a `LITEHOUSE_BLOB_PATH` env var (currently `/data/blobs`) for storing binary blobs (photos, attachments, etc.) that don't belong in the daily SQLite/tarball snapshot. Files written there back up incrementally — uploaded to S3 once, never re-uploaded on later days if unchanged — instead of riding along in the full daily tarball. See `docs/superpowers/specs/2026-07-14-blob-backup-design.md`.
```

- [ ] **Step 2: Update `CLAUDE.md`**

Find the bullet `- Daily backups (VACUUM INTO snapshots, tar.gz to S3, 14-day retention); \`lh backup run\` / \`lh backup status --json\`` and add directly after it:

```markdown
- Incremental blob backup: apps get `LITEHOUSE_BLOB_PATH=/data/blobs` and anything written there is backed up to its own S3 prefix (`blobs/{app_name}/...`, NOT nested under `apps/{app_name}/`) on an upload-once basis — unchanged files are never re-uploaded. Restored automatically as part of `lh restore --yes`. See `docs/superpowers/specs/2026-07-14-blob-backup-design.md`.
```

- [ ] **Step 3: Update `VISION.md`**

Find the `### Daily S3 Backups — SHIPPED` section. Add a new subsection directly after it:

```markdown
### Incremental Blob Backup — SHIPPED

Every app is handed a `LITEHOUSE_BLOB_PATH` env var (`/data/blobs`) at container start. Files written there are excluded from the daily full tarball and instead backed up individually to their own S3 prefix (`blobs/{app_name}/...`), uploaded once and never re-uploaded once present — unlike the tarball snapshot, which re-uploads everything every day regardless of change. Restored automatically by `lh restore --yes` alongside the dated tarball. No deletion sync and no per-file backup catalog — see `docs/superpowers/specs/2026-07-14-blob-backup-design.md` for the full design and rationale (in particular, why blob keys live under their own top-level prefix rather than nested under `apps/{app_name}/`).
```

- [ ] **Step 4: Commit**

```bash
git add README.md CLAUDE.md VISION.md
git commit -m "docs: document incremental blob backup and LITEHOUSE_BLOB_PATH"
```

---

## Summary of new public API surface (for the executing engineer's reference)

- `backup::BLOB_PATH_ENV_VAR: &str` = `"LITEHOUSE_BLOB_PATH"`
- `backup::BLOB_MOUNT_PATH: &str` = `"/data/blobs"`
- `backup::blob_key(prefix: Option<&str>, app_name: &str, relative_path: &str) -> String`
- `backup::blob_prefix_root(prefix: Option<&str>, app_name: &str) -> String`
- `backup::blobs_missing_from_s3(local_relative_paths: &[String], existing_keys: &[String], prefix: Option<&str>, app_name: &str) -> Vec<String>`
- `commands::start::ensure_blob_path_env_var(env_vars: Vec<EnvVar>, app_id: &str) -> Vec<EnvVar>` (private to the module, called from `start_container`)
