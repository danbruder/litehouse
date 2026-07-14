# Incremental Blob Backup for App Data — Design

**Date:** 2026-07-14
**Status:** Approved

## Goal

Give every litehouse app a place on its existing per-app data volume to store binary blobs (photos, attachments, etc.) that:

1. Rides along with the existing backup/restore system — `lh restore --yes` still rebuilds it from S3 like everything else, and
2. Is uploaded to S3 **incrementally** — a given file is PUT once and never re-uploaded on a later day just because the daily backup ran again — instead of being swept into the existing full daily tarball along with the rest of `/data`.

Every app, existing or future, is handed a `LITEHOUSE_BLOB_PATH` env var at container start pointing at the directory to use. Apps don't hardcode the convention, and litehouse is free to change the underlying path later without touching every app that consumes it.

## Non-goals

- **Deletion sync.** Removing a local blob does not remove its S3 copy. Orphaned objects accumulate in the bucket; acceptable at this scale (small personal apps, cheap object storage). Revisit only if it ever matters.
- **A backup catalog / DB row per blob** (unlike the existing `backup` table for dated tarballs). S3's own object listing is the source of truth for "what's already backed up" — adding a DB-backed catalog for potentially many small files isn't worth the complexity here.
- **New CLI surface** (no `lh blobs ...` commands). This folds into the existing `lh backup run` / `lh restore --yes` flows transparently.
- **Content-addressing or hashing on the litehouse side.** Litehouse treats every relative path under the blob directory as an immutable, write-once artifact. It is the *consuming app's* responsibility to name files such that this holds (e.g., a content-hash filename) — see "Consumer contract" below.

## Why blob keys must NOT live under `apps/{app_name}/`

The existing tarball backup logic lists everything under `apps/{app_name}/` for two purposes:

- `newest_key` (in `restore_app`) takes the lexically-max key under that prefix to find "the latest backup" to restore.
- `prune_old_backups` (in `backup_app`) lists that same prefix and deletes every key except the newest 14, on the assumption that every key is a dated tarball (`YYYY-MM-DD.tar.gz`).

If blob objects were keyed as `apps/{app_name}/blobs/<relative_path>`, they'd be mixed into both of those listings. Lexically, `"blobs/..."` sorts *after* any `"YYYY-MM-DD.tar.gz"` key (`b` > a digit in ASCII), so:

- `newest_key` would return a blob key instead of the actual latest dated tarball, breaking restore.
- `prune_old_backups`'s "keep newest 14" logic would start deleting individual blob objects it mistakes for old dated snapshots — silent, permanent data loss.

**Fix:** blob keys live under their own top-level prefix, a sibling of `apps/` and `litehouse/`:

```
blobs/{app_name}/{relative_path}         (no path_prefix configured)
{prefix}/blobs/{app_name}/{relative_path} (path_prefix configured)
```

This prefix is never passed to `newest_key` or `prune_old_backups` for the tarball logic, and blob keys are never pruned by the retention job at all (they aren't dated snapshots — see Non-goals).

## Component 1: `LITEHOUSE_BLOB_PATH` env var

**File:** `src/commands/start.rs`, `start_container`

Today, `start_container` loads an app's user-configured `env_var` rows from the DB and passes them straight to `docker::run`. This adds one synthetic entry — `LITEHOUSE_BLOB_PATH=/data/blobs` — to that list before the container starts, for every app, every time it starts (fresh deploy, restart, or post-restore). Because it's computed at start time rather than stored, it applies uniformly to every existing app on its next start/restart and to every future app with zero migration or backfill.

If an app has *explicitly* set `LITEHOUSE_BLOB_PATH` itself via `lh env set` (unlikely, but mirrors the "never clobber explicit config" posture used elsewhere, e.g. `copy_apps_from_snapshot`'s `INSERT OR IGNORE`), that explicit value wins — the synthetic default is only appended if the key isn't already present in the loaded env vars.

## Component 2: Snapshot staging (`src/backup.rs`, `snapshot_script`)

The one-shot snapshot container's script currently does two passes over `/data`: a `find` for `*.db`/`*.sqlite`/`*.sqlite3` files (VACUUM'd individually), and a `tar czf files.tar.gz` of everything else. Both need to skip the blob directory:

- `find` gets a `-path './blobs' -prune -o ...` so it doesn't walk into (possibly many) blob files looking for databases that won't be there.
- `tar` gets `--exclude='./blobs'` so blob content isn't captured a second time in the full daily tarball.
- A new line stages the blob directory verbatim into the backup staging area, if present: `[ -d blobs ] && cp -a blobs /backup/blobs`.

The container already mounts `/data` read-only and the per-app staging dir read-write, so this is a straightforward addition to the same script, no new container/mount needed.

## Component 3: Incremental upload (`src/backup.rs`, `backup_app`)

After the existing tarball-and-upload step, `backup_app` gets a new step: if `{staged_dir}/blobs` exists, walk it (the `walkdir` crate is already a dependency), and for each file:

1. Compute its path relative to `{staged_dir}/blobs`.
2. Build its S3 key via a new `blob_key(prefix, app_name, relative_path)` helper (mirrors `app_backup_key`).

Before uploading anything, call `list_keys` **once** against `blob_prefix_root(prefix, app_name)` to get the set of keys already in S3. Only files whose key isn't in that set get `upload_file`'d — this single LIST-then-selective-PUT is what eliminates the daily re-upload of unchanged images. A failure uploading any blob fails the whole `backup_app` call for that app, consistent with how a tarball upload failure already fails the app's backup today (surfaced per-app in `BackupReport.failed`, doesn't abort the run for other apps).

No changes to `BackupReport`'s shape — blob upload/skip counts are logged via `info!`, not persisted, keeping this addition small.

## Component 4: Restore (`src/backup.rs`, `restore_app`)

`restore_app`'s Phase 1 ("everything fallible, before touching the running container") gets a new step alongside downloading the dated tarball: list keys under `blob_prefix_root(prefix, app_name)` and download each into `stage_dir/blobs/<relative_path>` (recreating the directory structure).

`run_restore_container`'s script gets a third conditional leg, alongside the existing `files.tar.gz` untar and `dbs/` copy:

```sh
if [ -d /restore/blobs ]; then mkdir -p /data/blobs && cp -a /restore/blobs/. /data/blobs/; fi
```

This is gated by the same existing check that already gates the whole per-app restore: an app only restores at all if it has at least one dated tarball backup (`newest_key` under `apps/{app_name}/` is `Some`). In practice every real app has a SQLite DB and therefore always produces a tarball, so this gate is not a meaningful limitation for blobs specifically.

## Consumer contract (for apps using this, e.g. Butler)

- Write files under the directory named by `LITEHOUSE_BLOB_PATH` (currently `/data/blobs`), never hardcode the path.
- Name files such that the same relative path is **never reused for different content** — e.g. name by content hash. Litehouse assumes write-once; if bytes at an existing path change after the first backup captured them, the new content is silently not backed up (the key already "exists" as far as the skip-if-present check is concerned).
- Deleting a file locally does not delete its S3 backup copy (see Non-goals).

## New pure-logic helpers (unit-testable, no I/O)

- `blob_key(prefix: Option<&str>, app_name: &str, relative_path: &str) -> String`
- `blob_prefix_root(prefix: Option<&str>, app_name: &str) -> String`

Mirror the existing `app_backup_key` / `app_prefix_root` tests: prefix present/absent, correct separators.

## Testing

- Unit tests for `blob_key` / `blob_prefix_root` (pure string logic, like the existing `s3_key_layout` test).
- A unit test asserting the updated `snapshot_script()` contains the `blobs` prune/exclude/copy additions (mirrors the existing `snapshot_script_escapes_filenames_for_sql` pattern of asserting on the script's literal contents).
- Extend the existing MinIO-backed `test_backup_roundtrip_minio` integration test (`#[ignore]`, real Docker + local MinIO) to also seed a file under the app volume's `blobs/` directory, run `run_backup` twice, and assert: the object exists in S3 under `blobs/{app}/...` (not under `apps/{app}/`), and the second run does not re-PUT it (e.g. assert on a counter, or that the object's S3 `LastModified` from the first run is unchanged after the second).
- A restore-side integration test extending the same fixture: after `restore_all`, the blob file is present at `/data/blobs/<relative_path>` in the app's volume.

## Risks / open questions

- **Write-once assumption is unenforced.** Nothing stops an app from overwriting a path after its first backup; litehouse has no way to detect that and will not re-capture it. Documented in the consumer contract; not technically enforced.
- **No orphan cleanup.** S3 storage cost grows unbounded for deleted-but-never-pruned blobs. Not a concern at current scale (a family app's kid-wishlist photos); would need a follow-up if this pattern is reused for a higher-volume app.
