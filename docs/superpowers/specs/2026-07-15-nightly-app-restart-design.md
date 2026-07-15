# Nightly App Restart — Design

## Motivation

Long-running app containers can accumulate state, leaked connections, or
other cruft over time. This feature restarts every currently-running app
once a night, at 3am US Eastern time, as routine maintenance — the same
spirit as the existing daily S3 backup job, but for container freshness
instead of data durability.

## Scheduling

Add a third background loop in `src/commands/server.rs::execute`, spawned
alongside the existing backup loop (lines 131-161) and metrics sampler
(lines 166-183). Same shape: an hourly `tokio::time::interval`, ticking
immediately on process boot.

Each tick:

1. Compute the current wall-clock time in `America/New_York` via a new
   `chrono-tz` dependency (`chrono-tz = "0.9"` in `Cargo.toml`). This
   tracks US Eastern local time including DST, so the restart always
   happens at 3am to a person on the US East Coast, whether that's EST
   (winter, UTC-5) or EDT (summer, UTC-4).
2. Only proceed if the current Eastern hour is `3` (i.e. 03:00–03:59
   Eastern). Outside that hour, do nothing this tick.
3. Compare today's Eastern date (`YYYY-MM-DD`) against a persisted
   `last_nightly_restart_date`, read via a new
   `db::system_config::get_last_nightly_restart_date`. If they match,
   today's restart has already run — do nothing.
4. Otherwise, run the restart pass (below), then unconditionally persist
   today's Eastern date via a new `set_last_nightly_restart_date` — the
   day is marked done once the pass *completes*, regardless of individual
   app failures (see "Failure handling" below).

This mirrors the backup loop's "poll hourly, compare a persisted date"
pattern, with the addition of an hour-of-day gate, since backups have no
target time-of-day and this feature does.

### Data model change

New migration `migrations/20260715_last_nightly_restart_date.sql`:

```sql
ALTER TABLE system_config ADD COLUMN last_nightly_restart_date TEXT NULL;
```

Stored under its own `config_type` row (`'nightly_restart_meta'`), same
pattern as `last_backup_date`'s `'backup_meta'` row — independent of any
other system_config row, updated in isolation.

## Restart pass

Given the pool and docker connection, for every app in the database
(`db::app::get_all`):

1. Skip if the app has an env var opt-out: `db::env_var::get_by_app` and
   check for a `LITEHOUSE_SKIP_NIGHTLY_RESTART` key with value `"true"`.
   This reuses the existing per-app env var mechanism (no migration
   needed), the same mechanism already used for the blob-path override.
   Checked before the live Docker state so an app that has opted out
   never pays the cost of (and can't race on) a Docker inspect call.
2. Query live Docker state via `docker::live_state(&app.name)` — the
   existing source of truth for "is this app actually running," already
   used in preference to the cached `app.state` DB column elsewhere in
   the codebase. Skip apps that aren't `AppState::Running`.
3. Attempt to acquire that app's lock via a new `try_lock_app` (see
   below). If another operation currently holds it — a deploy hook
   firing, a manual start/stop/restart from the admin UI — skip this app
   for tonight and log it at `info` level. Do not wait for the lock; an
   in-flight deploy must never be blocked or interrupted by the nightly
   job.
4. With the lock held, re-check `docker::live_state` — it may have
   changed between steps 1 and 3 (e.g. a deploy just finished and
   replaced the container). If no longer `Running`, skip.
5. Restart by composing the existing lifecycle primitives, not a raw
   Bollard `restart_container` call: `docker::stop(&app)` followed by
   `start::start_container(pool, docker, &app, image_tag)`, where
   `image_tag` is `app.image` (the currently-deployed image — this is a
   restart, not a redeploy, so the image never changes). Skip (log a
   warning) if `app.image` is `None`, which shouldn't happen for a
   running app but is defensive. This is the same code path a manual
   stop+start already goes through, so `app.state` and Caddy config stay
   correctly in sync — no separate bookkeeping needed.
6. Log per-app success or failure at `info`/`error`. No new persisted
   history table (unlike backups' `deploy` report) — this is best-effort
   maintenance, not a user-facing operation that needs an audit trail.

### New `try_lock_app`

`server.rs`'s existing `lock_app` (line 45) always awaits — correct for
callers that must proceed once free (a deploy, a manual restart click).
The nightly job needs the opposite: skip immediately if busy. Add:

```rust
pub fn try_lock_app(locks: &AppLocks, name: &str) -> Option<OwnedMutexGuard<()>> {
    let entry = locks
        .lock()
        .unwrap()
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone();
    entry.try_lock_owned().ok()
}
```

### Failure handling

Unlike the backup loop — which only marks the day done when every app
succeeds, so a partial failure retries next hour — the nightly restart
marks the day done after the pass completes regardless of per-app
failures. Retrying restarts hourly through the 3am window could otherwise
restart an already-succeeded app multiple times in one night, or repeatedly
retry an app that's failing for a structural reason (bad image, etc.) —
neither is desirable for a maintenance job. A failed restart is logged and
picked up again the next night.

## Concurrency safety

Reuses the same per-app lock registry (`AppState.app_locks`) already used
by manual start/stop/restart (`ui.rs`, `api.rs`) and the deploy engine.
The nightly loop is just another actor competing for that lock, using the
non-blocking variant so it always yields to an in-progress operation
rather than contending with it.

## Opt-out

Setting `LITEHOUSE_SKIP_NIGHTLY_RESTART=true` via `lh env set <app>
LITEHOUSE_SKIP_NIGHTLY_RESTART true` (existing `lh env` command) excludes
an app from the nightly pass. No new CLI surface needed.

## Testing

- Pure unit test for the gating predicate ("is it the Eastern 3am hour,
  and has today's Eastern date already run") — independent of Docker/DB,
  same style as the existing backup-date comparison logic.
- Unit tests for `try_lock_app`: succeeds when free, returns `None` when
  `lock_app` already holds the lock elsewhere.
- Docker integration test (following the existing `#[cfg(test)]` patterns
  in `docker.rs`/`start.rs`/`stop.rs`) that calls the per-app restart
  function directly — bypassing the scheduler loop entirely — starts a
  real container, restarts it, and asserts it comes back up `Running`.
- Integration test for the opt-out: an app with
  `LITEHOUSE_SKIP_NIGHTLY_RESTART=true` set is left untouched by a full
  pass.

## Non-goals

- No admin UI surface for viewing nightly-restart history (no history is
  persisted beyond the last-run date, per "Failure handling" above).
- No configurable time-of-day — 3am US Eastern is fixed, matching what was
  requested. Making the hour configurable is easy to add later
  (another `system_config` field) but isn't needed now.
- No change to Docker's own container restart policy (`unless-stopped`,
  `docker.rs:225-226`) — that's for crash recovery, orthogonal to this
  scheduled maintenance restart.
