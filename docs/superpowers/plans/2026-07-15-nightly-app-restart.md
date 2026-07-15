# Nightly App Restart Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every running app gets restarted once a night at 3am US Eastern time, skipping apps that are mid-deploy/locked or explicitly opted out.

**Architecture:** A third `tokio::spawn` background loop in `src/commands/server.rs::execute`, following the exact shape of the existing hourly backup loop, gated by a new pure predicate (`restart::should_run_now`) that checks the Eastern hour and a persisted last-run date. The actual restart pass lives in a new `src/restart.rs` module and composes the existing `docker::stop` + `start::start_container` primitives, guarded by a non-blocking variant of the existing per-app lock (`try_lock_app`).

**Tech Stack:** Rust, Tokio, SQLx (SQLite), Bollard (Docker), `chrono` + new `chrono-tz` dependency.

**Reference spec:** `docs/superpowers/specs/2026-07-15-nightly-app-restart-design.md`

---

## Task 1: Add the `chrono-tz` dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, in the `[dependencies]` block, add this line directly under the existing `chrono = { version = "0.4", features = ["serde"] }` line:

```toml
chrono-tz = "0.9"
```

- [ ] **Step 2: Verify it resolves and builds**

Run: `cargo check`
Expected: Compiles successfully (this only adds an unused dependency at this point — no code references it yet). `Cargo.lock` will be updated with the new crate; that's expected.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add chrono-tz dependency for Eastern-time scheduling"
```

---

## Task 2: Migration for `last_nightly_restart_date`

**Files:**
- Create: `migrations/20260715_last_nightly_restart_date.sql`

- [ ] **Step 1: Write the migration**

Create `migrations/20260715_last_nightly_restart_date.sql`:

```sql
-- Add a column to system_config for tracking the Eastern-time date
-- (YYYY-MM-DD) the nightly app-restart scheduler last completed a pass.
-- Stored under its own config_type ('nightly_restart_meta') so it can be
-- updated independently of other system_config rows (see src/restart.rs /
-- src/commands/server.rs scheduler).
ALTER TABLE system_config ADD COLUMN last_nightly_restart_date TEXT NULL;
```

- [ ] **Step 2: Apply the migration to the local dev database**

Run: `DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" cargo sqlx migrate run`
Expected: Output confirms the new migration applied (e.g. `Applying 20260715_last_nightly_restart_date.sql`).

- [ ] **Step 3: Commit**

```bash
git add migrations/20260715_last_nightly_restart_date.sql
git commit -m "feat: add last_nightly_restart_date column to system_config"
```

---

## Task 3: `db::system_config` get/set for the nightly-restart date

**Files:**
- Modify: `src/db/system_config.rs`

- [ ] **Step 1: Write the failing tests**

In `src/db/system_config.rs`, inside the existing `#[cfg(test)] mod tests` block (near `test_set_and_get_last_backup_date`), add:

```rust
    #[tokio::test]
    async fn test_set_and_get_last_nightly_restart_date() {
        let pool = get_test_pool().await;
        assert!(get_last_nightly_restart_date(&pool).await.unwrap().is_none());

        set_last_nightly_restart_date(&pool, "2026-07-15").await.unwrap();
        assert_eq!(
            get_last_nightly_restart_date(&pool).await.unwrap(),
            Some("2026-07-15".to_string())
        );

        // Overwriting updates the same row rather than erroring or inserting
        // a second one.
        set_last_nightly_restart_date(&pool, "2026-07-16").await.unwrap();
        assert_eq!(
            get_last_nightly_restart_date(&pool).await.unwrap(),
            Some("2026-07-16".to_string())
        );
    }

    #[tokio::test]
    async fn test_last_nightly_restart_date_independent_of_last_backup_date() {
        let pool = get_test_pool().await;
        set_last_backup_date(&pool, "2026-07-01").await.unwrap();
        set_last_nightly_restart_date(&pool, "2026-07-02").await.unwrap();

        assert_eq!(
            get_last_backup_date(&pool).await.unwrap(),
            Some("2026-07-01".to_string())
        );
        assert_eq!(
            get_last_nightly_restart_date(&pool).await.unwrap(),
            Some("2026-07-02".to_string())
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test --lib db::system_config::tests::test_set_and_get_last_nightly_restart_date`
Expected: Compile error — `get_last_nightly_restart_date`/`set_last_nightly_restart_date` not found.

- [ ] **Step 3: Implement the functions**

In `src/db/system_config.rs`, directly below the existing `get_last_backup_date` function, add:

```rust
/// Record the Eastern-time date (YYYY-MM-DD) the nightly app-restart
/// scheduler last completed a pass. Stored under its own `config_type` row
/// (`nightly_restart_meta`), independent of `backup_meta` and every other
/// system_config row.
#[instrument(skip(pool))]
pub async fn set_last_nightly_restart_date(pool: &Pool<Sqlite>, date: &str) -> Result<()> {
    let now = crate::models::now();
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query!(
        r#"
        INSERT INTO system_config (id, config_type, last_nightly_restart_date, created_at, updated_at)
        VALUES (?, 'nightly_restart_meta', ?, ?, ?)
        ON CONFLICT(config_type) DO UPDATE SET
            last_nightly_restart_date = excluded.last_nightly_restart_date,
            updated_at = excluded.updated_at
        "#,
        id,
        date,
        now,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the Eastern-time date (YYYY-MM-DD) of the last completed nightly
/// restart pass, if any has run yet.
#[instrument(skip(pool))]
pub async fn get_last_nightly_restart_date(pool: &Pool<Sqlite>) -> Result<Option<String>> {
    let record = sqlx::query!(
        r#"SELECT last_nightly_restart_date FROM system_config WHERE config_type = 'nightly_restart_meta'"#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(record.and_then(|r| r.last_nightly_restart_date))
}
```

- [ ] **Step 4: Regenerate the SQLx offline query cache**

The two new `sqlx::query!` calls need entries in `.sqlx/` (checked into the repo, used for compile-time query verification). Run:

```bash
DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" cargo sqlx prepare
```

Expected: Command succeeds and adds new `.sqlx/query-*.json` files (`git status` will show new files under `.sqlx/`).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib db::system_config::tests::test_set_and_get_last_nightly_restart_date db::system_config::tests::test_last_nightly_restart_date_independent_of_last_backup_date`
Expected: Both tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/db/system_config.rs .sqlx/
git commit -m "feat: add get/set_last_nightly_restart_date to system_config"
```

---

## Task 4: Non-blocking `try_lock_app`

**Files:**
- Modify: `src/commands/server.rs`

- [ ] **Step 1: Write the failing tests**

In `src/commands/server.rs`, inside the existing `#[cfg(test)] mod tests` block (after `lock_app_does_not_block_different_apps`), add:

```rust
    #[tokio::test]
    async fn try_lock_app_succeeds_when_free() {
        let locks: AppLocks = Arc::new(StdMutex::new(HashMap::new()));
        let guard = try_lock_app(&locks, "free-app");
        assert!(guard.is_some(), "should acquire the lock when nothing else holds it");
    }

    #[tokio::test]
    async fn try_lock_app_returns_none_when_already_held() {
        let locks: AppLocks = Arc::new(StdMutex::new(HashMap::new()));
        let _held = lock_app(&locks, "busy-app").await;
        let attempt = try_lock_app(&locks, "busy-app");
        assert!(attempt.is_none(), "should not acquire a lock already held elsewhere");
    }

    #[tokio::test]
    async fn try_lock_app_does_not_block_different_apps() {
        let locks: AppLocks = Arc::new(StdMutex::new(HashMap::new()));
        let _held = lock_app(&locks, "a").await;
        let attempt = try_lock_app(&locks, "b");
        assert!(attempt.is_some(), "a different app's lock should be free");
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test --lib commands::server::tests::try_lock_app_succeeds_when_free`
Expected: Compile error — `try_lock_app` not found.

- [ ] **Step 3: Implement `try_lock_app`**

In `src/commands/server.rs`, directly below the existing `lock_app` function (after its closing `}`, before `sync_on_boot`), add:

```rust
/// Non-blocking variant of [`lock_app`] — for callers that must yield to an
/// in-progress operation rather than wait for it (the nightly restart pass:
/// an app mid-deploy should never be delayed or interrupted by a scheduled
/// maintenance restart).
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

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib commands::server::tests::try_lock_app_succeeds_when_free commands::server::tests::try_lock_app_returns_none_when_already_held commands::server::tests::try_lock_app_does_not_block_different_apps`
Expected: All three PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands/server.rs
git commit -m "feat: add non-blocking try_lock_app"
```

---

## Task 5: `restart` module — the scheduling predicate

**Files:**
- Create: `src/restart.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Register the module**

In `src/lib.rs`, add this line in alphabetical position among the existing `pub mod` lines (between `pub mod models;` and `pub mod docker;` is fine since the list isn't strictly alphabetical — place it right after `pub mod models;`):

```rust
pub mod restart;
```

- [ ] **Step 2: Write the failing tests**

Create `src/restart.rs` with just the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn eastern(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono_tz::Tz> {
        chrono_tz::America::New_York
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .unwrap()
    }

    #[test]
    fn should_run_at_3am_when_never_run_before() {
        assert!(should_run_now(eastern(2026, 7, 15, 3), None));
    }

    #[test]
    fn should_not_run_outside_the_3am_hour() {
        assert!(!should_run_now(eastern(2026, 7, 15, 2), None));
        assert!(!should_run_now(eastern(2026, 7, 15, 4), None));
        assert!(!should_run_now(eastern(2026, 7, 15, 0), None));
    }

    #[test]
    fn should_not_run_twice_in_the_same_eastern_day() {
        assert!(!should_run_now(eastern(2026, 7, 15, 3), Some("2026-07-15")));
    }

    #[test]
    fn should_run_again_the_next_day() {
        assert!(should_run_now(eastern(2026, 7, 16, 3), Some("2026-07-15")));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail to compile**

Run: `cargo test --lib restart::tests`
Expected: Compile error — `should_run_now` not found (module has no non-test code yet).

- [ ] **Step 4: Implement `should_run_now`**

At the top of `src/restart.rs`, above the `#[cfg(test)]` block, add:

```rust
use chrono::Timelike;

/// True if, given the current Eastern time and the Eastern date
/// (`YYYY-MM-DD`) the nightly restart last completed (`None` if it has
/// never run), tonight's pass should run now. Pure and independent of
/// Docker/DB so it's directly unit-testable; the scheduler loop in
/// `commands::server::execute` calls this once per hourly tick.
pub fn should_run_now(now_eastern: chrono::DateTime<chrono_tz::Tz>, last_run_date: Option<&str>) -> bool {
    if now_eastern.hour() != 3 {
        return false;
    }
    let today = now_eastern.format("%Y-%m-%d").to_string();
    last_run_date != Some(today.as_str())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib restart::tests`
Expected: All 4 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/restart.rs src/lib.rs
git commit -m "feat: add nightly restart scheduling predicate"
```

---

## Task 6: `restart` module — the restart pass

**Files:**
- Modify: `src/restart.rs`
- Modify: `src/docker.rs`

- [ ] **Step 1: Write the failing unit tests for skip behavior**

In `src/restart.rs`, add these imports at the top (below the `use chrono::Timelike;` line already there):

```rust
use bollard::Docker;
use sqlx::{Pool, Sqlite};
use tracing::instrument;

use crate::commands::server::{try_lock_app, AppLocks};
use crate::commands::start::start_container;
use crate::db;
use crate::docker;
use crate::models::App;

/// Env var key an app can set (via `lh env set`) to exclude itself from the
/// nightly restart pass.
pub const SKIP_ENV_VAR: &str = "LITEHOUSE_SKIP_NIGHTLY_RESTART";

#[derive(Debug, Default)]
pub struct RestartReport {
    pub restarted: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

#[derive(Debug, PartialEq)]
pub enum RestartOutcome {
    Restarted,
    Skipped(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum RestartError {
    #[error("failed to stop container: {0}")]
    StopFailed(String),
    #[error("failed to start container: {0}")]
    StartFailed(String),
    #[error("app has no deployed image")]
    NoImage,
}
```

Then add these tests to the `#[cfg(test)] mod tests` block (below the existing `should_run_now` tests):

```rust
    use crate::db::test::get_test_pool;
    use crate::models::EnvVar;

    async fn make_app(pool: &sqlx::Pool<sqlx::Sqlite>, name: &str) -> App {
        let app = App::new(name).unwrap();
        crate::db::app::save(pool, &app).await.unwrap();
        app
    }

    #[tokio::test]
    async fn restart_one_app_skips_when_not_running() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.unwrap();
        let locks: AppLocks = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        // No container exists for this app name, so live_state reports Stopped.
        let app = make_app(&pool, "restart-test-not-running").await;

        let outcome = restart_one_app(&pool, &docker, &locks, &app).await.unwrap();
        assert_eq!(outcome, RestartOutcome::Skipped("not running"));
    }

    #[tokio::test]
    async fn restart_one_app_skips_when_opted_out() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.unwrap();
        let locks: AppLocks = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let app = make_app(&pool, "restart-test-opted-out").await;
        crate::db::env_var::save(
            &pool,
            &EnvVar::new(&app.id, SKIP_ENV_VAR, "true"),
        )
        .await
        .unwrap();

        // Opt-out is checked before the live Docker state, so no real
        // container is needed to exercise this path.
        let outcome = restart_one_app(&pool, &docker, &locks, &app).await.unwrap();
        assert_eq!(outcome, RestartOutcome::Skipped("opted out via LITEHOUSE_SKIP_NIGHTLY_RESTART"));
    }
```

(`App::new(name: &str) -> Result<Self, AppError>` and `db::app::save(pool, &app) -> Result<()>` — `save` is an upsert, the only write path `db/app.rs` exposes; there is no separate `create`.)

- [ ] **Step 2: Run the new tests to verify they fail to compile**

Run: `cargo test --lib restart::tests::restart_one_app_skips_when_not_running`
Expected: Compile error — `restart_one_app` not found.

- [ ] **Step 3: Implement `restart_one_app` and `restart_running_apps`**

In `src/restart.rs`, below the `should_run_now` function (still above the `#[cfg(test)]` block), add:

```rust
/// Restart a single app if it's currently running, not locked by another
/// operation, and not opted out. Never blocks waiting for the app's lock —
/// if a deploy or manual action currently holds it, this returns
/// `Skipped` immediately rather than delaying or contending with it.
#[instrument(skip(pool, docker, app_locks, app), fields(app = %app.name))]
pub async fn restart_one_app(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    app_locks: &AppLocks,
    app: &App,
) -> Result<RestartOutcome, RestartError> {
    match docker::live_state(&app.name).await {
        Ok(crate::models::AppState::Running) => {}
        Ok(_) => return Ok(RestartOutcome::Skipped("not running")),
        Err(e) => {
            tracing::warn!(app = %app.name, "nightly restart: failed to check live state: {e:#}");
            return Ok(RestartOutcome::Skipped("live state check failed"));
        }
    }

    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| RestartError::StopFailed(format!("failed to load env vars: {e}")))?;
    if env_vars
        .iter()
        .any(|e| e.key == SKIP_ENV_VAR && e.value.eq_ignore_ascii_case("true"))
    {
        return Ok(RestartOutcome::Skipped(
            "opted out via LITEHOUSE_SKIP_NIGHTLY_RESTART",
        ));
    }

    let Some(_guard) = try_lock_app(app_locks, &app.name) else {
        return Ok(RestartOutcome::Skipped("app lock held by another operation"));
    };

    // Re-check now that we hold the lock — it may have changed between the
    // check above and acquiring the lock (e.g. a deploy just replaced the
    // container).
    match docker::live_state(&app.name).await {
        Ok(crate::models::AppState::Running) => {}
        _ => return Ok(RestartOutcome::Skipped("no longer running")),
    }

    let image = app.image.as_deref().ok_or(RestartError::NoImage)?;

    docker::stop(app)
        .await
        .map_err(|e| RestartError::StopFailed(e.to_string()))?;

    start_container(pool, docker, app, image)
        .await
        .map_err(|e| RestartError::StartFailed(e.to_string()))?;

    Ok(RestartOutcome::Restarted)
}

/// Restart every currently-running app, skipping ones that are locked or
/// opted out. Called once per completed scheduling tick (see
/// `commands::server::execute`).
#[instrument(skip(pool, docker, app_locks))]
pub async fn restart_running_apps(
    pool: &Pool<Sqlite>,
    docker: &Docker,
    app_locks: &AppLocks,
) -> RestartReport {
    let mut report = RestartReport::default();

    let apps = match db::app::get_all(pool).await {
        Ok(apps) => apps,
        Err(e) => {
            tracing::error!("nightly restart: failed to list apps: {e:#}");
            return report;
        }
    };

    for app in apps {
        match restart_one_app(pool, docker, app_locks, &app).await {
            Ok(RestartOutcome::Restarted) => report.restarted.push(app.name.clone()),
            Ok(RestartOutcome::Skipped(reason)) => {
                report.skipped.push((app.name.clone(), reason.to_string()))
            }
            Err(e) => report.failed.push((app.name.clone(), e.to_string())),
        }
    }

    report
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib restart::tests`
Expected: All tests PASS (the two new ones plus the four `should_run_now` tests from Task 5).

- [ ] **Step 5: Make `docker::test_helpers` visible to other modules**

`src/docker.rs:588-589` currently declares `#[cfg(test)] mod test_helpers { ... }` with no visibility modifier, so it's private to `docker.rs` — restart.rs's own test module can't reach it yet. Change the declaration in `src/docker.rs` from:

```rust
#[cfg(test)]
mod test_helpers {
```

to:

```rust
#[cfg(test)]
pub(crate) mod test_helpers {
```

This only affects test builds (still gated by `#[cfg(test)]`) and only widens visibility within the crate, matching how `restart.rs`'s tests need to reuse the same container cleanup/inspection helpers `docker.rs`'s own tests already use.

Run: `cargo test --lib docker::tests` to confirm this alone doesn't break anything.
Expected: PASS (no behavior change, only visibility).

- [ ] **Step 6: Write the ignored Docker integration test**

Add this test to the same `mod tests` block in `src/restart.rs`, following the `#[ignore]` convention used by `backup.rs`'s MinIO tests:

```rust
    /// Requires Docker. Run with:
    ///   DOCKER_API_VERSION=1.42 cargo test restart_one_app_restarts_a_running_container -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn restart_one_app_restarts_a_running_container() {
        let pool = get_test_pool().await;
        let docker = crate::docker::connect().await.unwrap();
        let locks: AppLocks = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        let app_name = "restart-test-live";
        let container_name = format!("{}-container", app_name);
        let image_tag = "redis:7.4-alpine";

        // Clean up any leftover container from a previous failed/interrupted run.
        let _ = crate::docker::test_helpers::cleanup_container(&container_name);

        crate::docker::run(app_name, image_tag, vec![], vec![]).await.unwrap();
        assert!(crate::docker::test_helpers::is_container_started(&container_name).unwrap());

        let mut app = make_app(&pool, app_name).await;
        app.image = Some(image_tag.to_string());
        crate::db::app::save(&pool, &app).await.unwrap();

        let outcome = restart_one_app(&pool, &docker, &locks, &app).await.unwrap();
        assert_eq!(outcome, RestartOutcome::Restarted);

        let state = crate::docker::test_helpers::get_container_state(&container_name).unwrap();
        assert_eq!(state, "running");

        crate::docker::test_helpers::cleanup_container(&container_name).unwrap();
    }
```

- [ ] **Step 7: Run the ignored test to verify it passes (requires Docker)**

Run: `DOCKER_API_VERSION=1.42 cargo test --lib restart_one_app_restarts_a_running_container -- --ignored --nocapture`
Expected: PASS (starts a real `redis:7.4-alpine` container, restarts it via `restart_one_app`, confirms it's running again, cleans up).

- [ ] **Step 8: Commit**

```bash
git add src/restart.rs src/docker.rs
git commit -m "feat: implement nightly restart pass with lock and opt-out handling"
```

---

## Task 7: Wire the scheduler loop into `server.rs::execute`

**Files:**
- Modify: `src/commands/server.rs`

- [ ] **Step 1: Add the third background loop**

In `src/commands/server.rs`, directly after the closing `}` of the existing daily-backup loop block (right before the `// Resource-usage sampler...` comment), add:

```rust
    // Nightly app restart: check hourly whether it's the 3am US-Eastern
    // hour and today's (Eastern) restart pass hasn't run yet. Skips apps
    // that are locked (mid-deploy/manual action) or opted out via
    // LITEHOUSE_SKIP_NIGHTLY_RESTART — see
    // docs/superpowers/specs/2026-07-15-nightly-app-restart-design.md.
    //
    // Unlike the backup loop, the day is marked done once the pass
    // *completes*, regardless of individual app failures — this is
    // best-effort maintenance, not something that should retry hourly and
    // potentially restart an app multiple times in one night.
    {
        let pool = pool.clone();
        let docker_conn = docker_conn.clone();
        let app_locks = state.read().await.app_locks.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let now_eastern = chrono::Utc::now().with_timezone(&chrono_tz::America::New_York);
                let last = db::system_config::get_last_nightly_restart_date(&pool)
                    .await
                    .ok()
                    .flatten();
                if crate::restart::should_run_now(now_eastern, last.as_deref()) {
                    let today = now_eastern.format("%Y-%m-%d").to_string();
                    let report =
                        crate::restart::restart_running_apps(&pool, &docker_conn, &app_locks).await;
                    tracing::info!(?report, "nightly app restart complete");
                    if let Err(e) =
                        db::system_config::set_last_nightly_restart_date(&pool, &today).await
                    {
                        tracing::error!("failed to record last_nightly_restart_date: {e:#}");
                    }
                }
            }
        });
    }
```

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build`
Expected: Compiles with no errors. (If `state.read().await.app_locks.clone()` doesn't compile because `state` isn't in scope at that point or `AppState`'s field name differs, check the surrounding code around line 114 of `src/commands/server.rs` — the field is `app_locks` on the `AppState` struct constructed a few lines above the backup loop — and adjust the access expression to match, not the field name itself.)

- [ ] **Step 3: Run the full test suite**

Run: `cargo test`
Expected: All non-ignored tests PASS (ignored Docker/MinIO tests are skipped by default, consistent with existing CI behavior).

- [ ] **Step 4: Commit**

```bash
git add src/commands/server.rs
git commit -m "feat: schedule nightly app restart at 3am US Eastern"
```

---

## Task 8: Document the feature

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add a bullet under the v2 highlights list**

In `CLAUDE.md`, in the "v2 highlights" bullet list (in the "Current State (v2 shipped)" section), add a new bullet after the incremental blob backup one:

```markdown
- Nightly app restart: every running app is restarted once a night at 3am US Eastern time (best-effort maintenance, not a redeploy — same image, just a fresh container). An app can opt out via `lh env set <app> LITEHOUSE_SKIP_NIGHTLY_RESTART true`. Apps mid-deploy or otherwise locked are skipped for that night rather than delayed. See `docs/superpowers/specs/2026-07-15-nightly-app-restart-design.md`.
```

- [ ] **Step 2: Add a troubleshooting note**

In the "Common Troubleshooting" section of `CLAUDE.md`, add:

```markdown
- If an app restarted unexpectedly overnight: check `docker logs litehouse-server` for `"nightly app restart complete"` around 3am Eastern — it logs which apps were restarted, skipped (and why), or failed. Opt an app out with `lh env set <app> LITEHOUSE_SKIP_NIGHTLY_RESTART true`.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document the nightly app restart feature"
```

---

## Task 9: Final verification

- [ ] **Step 1: Run the full non-ignored test suite**

Run: `cargo test`
Expected: All tests PASS.

- [ ] **Step 2: Run the ignored Docker integration test one more time**

Run: `DOCKER_API_VERSION=1.42 cargo test --lib restart_one_app_restarts_a_running_container -- --ignored --nocapture`
Expected: PASS.

- [ ] **Step 3: Build the release binary to catch any target-specific issues**

Run: `cargo build --release`
Expected: Compiles successfully.

- [ ] **Step 4: Review the full diff**

Run: `git log --oneline main..HEAD` and `git diff main..HEAD --stat`
Expected: A clean sequence of commits matching Tasks 1–8, touching only the files listed in this plan.
