# Server Hardening (Memory Caps + Maintenance Timers) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cap per-app container memory (with swap disabled beyond the cap), add two host-level systemd timers (hourly `dial-stdio` cleanup, weekly reboot), wire both timers into install and upgrade, and fix a stale doc comment — closing the gaps that caused the OOM incident described in `docs/superpowers/specs/2026-07-16-server-hardening-design.md`.

**Architecture:** Three independent changes bundled as one incident-response plan: (1) `docker::run()` gains a `memory_limit_mb` parameter plumbed from a new per-app env var with a 256MB default; (2) two new systemd unit pairs are generated as template strings (`src/install/templates.rs`) and installed/enabled by two new idempotent phase functions (`src/install/phases.rs`), wired into both `lh install` and `lh upgrade`; (3) a one-line `CLAUDE.md` correction.

**Tech Stack:** Rust, Bollard (Docker API client), systemd unit files, existing `sudo_write_file`/`run_command` install-phase helpers.

---

## Task 1: Memory-limit resolution (pure function + unit tests)

**Files:**
- Modify: `src/commands/start.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `src/commands/start.rs` (alongside the existing `ensure_blob_path_env_var` tests):

```rust
    #[test]
    fn resolve_memory_limit_mb_uses_default_when_unset() {
        let env_vars = vec![];
        assert_eq!(resolve_memory_limit_mb(&env_vars), docker::DEFAULT_APP_MEMORY_LIMIT_MB);
    }

    #[test]
    fn resolve_memory_limit_mb_uses_override_when_set() {
        let env_vars = vec![EnvVar::new("app-1", MEMORY_LIMIT_ENV_VAR, "512")];
        assert_eq!(resolve_memory_limit_mb(&env_vars), 512);
    }

    #[test]
    fn resolve_memory_limit_mb_falls_back_to_default_on_malformed_value() {
        let env_vars = vec![EnvVar::new("app-1", MEMORY_LIMIT_ENV_VAR, "not-a-number")];
        assert_eq!(resolve_memory_limit_mb(&env_vars), docker::DEFAULT_APP_MEMORY_LIMIT_MB);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib resolve_memory_limit_mb`
Expected: FAIL with "cannot find function `resolve_memory_limit_mb`" / "cannot find value `MEMORY_LIMIT_ENV_VAR`" (compile error). It will also complain `docker::DEFAULT_APP_MEMORY_LIMIT_MB` doesn't exist yet — that's added in Task 2; for this step, just confirm the failure is the expected "not found" compile error, not something unrelated.

- [ ] **Step 3: Write the minimal implementation**

Add above `ensure_blob_path_env_var` in `src/commands/start.rs`:

```rust
/// Env var an app can set (via `lh env set`) to override its container's
/// memory cap in megabytes. See `docker::DEFAULT_APP_MEMORY_LIMIT_MB` for
/// the default applied when this isn't set (or is malformed).
pub const MEMORY_LIMIT_ENV_VAR: &str = "LITEHOUSE_MEMORY_LIMIT_MB";

/// Resolve the container memory cap (in MB) for an app from its env vars,
/// falling back to `docker::DEFAULT_APP_MEMORY_LIMIT_MB` when the override
/// is absent or fails to parse as a positive integer.
fn resolve_memory_limit_mb(env_vars: &[EnvVar]) -> i64 {
    env_vars
        .iter()
        .find(|e| e.key == MEMORY_LIMIT_ENV_VAR)
        .and_then(|e| e.value.parse::<i64>().ok())
        .filter(|mb| *mb > 0)
        .unwrap_or(docker::DEFAULT_APP_MEMORY_LIMIT_MB)
}
```

- [ ] **Step 4: Leave the tests red for now and move straight to Task 2**

`docker::DEFAULT_APP_MEMORY_LIMIT_MB` doesn't exist until Task 2 adds it, so `cargo test --lib resolve_memory_limit_mb` still fails to compile at this point — that's expected. Do not commit yet. Proceed directly to Task 2, which adds the missing constant; its own Step 2 is where these three tests are run and confirmed green, and Task 2 ends with a single commit covering both files.

---

## Task 2: Add memory cap to `docker::run()`'s `HostConfig` and wire it end-to-end

**Files:**
- Modify: `src/docker.rs` (constant + `run()` signature + `HostConfig` construction + all in-file test call sites)
- Modify: `src/commands/start.rs` (`start_container`'s call to `docker::run()`, continuing Task 1)
- Modify: `src/restart.rs` (test call site)

- [ ] **Step 1: Add the default constant and extend the `HostConfig`**

In `src/docker.rs`, add near the top (after the `DockerError` enum, before `pub async fn connect()`):

```rust
/// Default per-app container memory cap in megabytes, applied unless the
/// app overrides it via `LITEHOUSE_MEMORY_LIMIT_MB` (see
/// `commands::start::resolve_memory_limit_mb`). 7 apps x 256MB = 1.8GB
/// worst case, leaving headroom for litehouse-server/caddy/OS on a 1.9GB
/// box. See `docs/superpowers/specs/2026-07-16-server-hardening-design.md`.
pub const DEFAULT_APP_MEMORY_LIMIT_MB: i64 = 256;
```

Change the `run()` signature (currently at `src/docker.rs:114-119`):

```rust
#[instrument]
pub async fn run(
    name: &str,
    image_tag: &str,
    env_vars: Vec<EnvVar>,
    volume_binds: Vec<String>,
    memory_limit_mb: i64,
) -> Result<()> {
```

In the `host_config` block (currently `src/docker.rs:206-235`), add the memory fields right after `let mut config = HostConfig::default();`:

```rust
        let mut config = HostConfig::default();

        // Cap container memory with swap disabled beyond the cap
        // (memory_swap == memory) so an OOMing app is contained rather
        // than swapping the host into unresponsiveness. See
        // docs/superpowers/specs/2026-07-16-server-hardening-design.md.
        let memory_bytes = memory_limit_mb * 1024 * 1024;
        config.memory = Some(memory_bytes);
        config.memory_swap = Some(memory_bytes);
```

- [ ] **Step 2: Wire `start_container` to resolve and pass the limit, then confirm Task 1's tests pass**

In `start_container` (`src/commands/start.rs`), the env vars are loaded and then mutated by `ensure_blob_path_env_var` before `docker::run()` is called. Resolve the memory limit from the vars right after they're loaded (before `ensure_blob_path_env_var` runs, since the blob-path key is irrelevant to this resolution):

```rust
    let env_vars = db::env_var::get_by_app(pool, &app.id)
        .await
        .map_err(|e| StartError::DatabaseError(e.to_string()))?;

    tracing::info!("Found {} environment variables", env_vars.len());
    let memory_limit_mb = resolve_memory_limit_mb(&env_vars);
    let env_vars = ensure_blob_path_env_var(env_vars, &app.id);
```

Update the `docker::run` call at the end of `start_container` (currently `src/commands/start.rs:119`):

```rust
    docker::run(&app.name, image_tag, env_vars, volume_binds, memory_limit_mb)
        .await
        .map_err(|e| StartError::AppStartFailed(e.to_string()))?;
```

Run: `cargo test --lib resolve_memory_limit_mb`
Expected: PASS (3 tests, from Task 1).

- [ ] **Step 3: Fix the remaining call sites so the whole crate compiles**

`src/restart.rs` — the test call site at `src/restart.rs:246`:

```rust
        crate::docker::run(app_name, image_tag, vec![], vec![], crate::docker::DEFAULT_APP_MEMORY_LIMIT_MB).await.unwrap();
```

`src/docker.rs`'s own `#[cfg(test)]` module — every `run(...)` call listed below gets a trailing `, DEFAULT_APP_MEMORY_LIMIT_MB` argument (these are all ignored Docker-integration tests, so this is a mechanical signature fix, not new test logic). Lines (pre-edit) `775`, `806`, `816`, `832`, `840`, `875`, `898`, `922`, `933`, `959`, `1016`, `1020`, `1066`, `1170`. For example, line 775:

```rust
        let run_result = run(app_name, image_tag, vec![], vec![], DEFAULT_APP_MEMORY_LIMIT_MB).await;
```

Apply the same trailing-argument addition to each of the other 13 call sites (they all follow the identical `run(<args>, vec![], vec![])` shape — insert `, DEFAULT_APP_MEMORY_LIMIT_MB` before the closing paren of the `run(...)` call itself, not inside either `vec![]`).

- [ ] **Step 4: Compile-check and run the full local test suite**

Run: `cargo build --lib`
Expected: compiles cleanly (warnings OK, no errors).

Run: `cargo test --lib`
Expected: PASS. The memory cap itself is not asserted by a new unit test — the spec explicitly scopes out a new Docker integration test for it (asserting an actual OOM-kill under a 256MB cap is disproportionate here); the existing ignored `restart_one_app_restarts_a_running_container` integration test will continue to exercise the new `HostConfig` fields via `docker::run()` whenever it's run manually with Docker available.

- [ ] **Step 5: Commit**

```bash
git add src/docker.rs src/commands/start.rs src/restart.rs
git commit -m "feat: cap per-app container memory with swap disabled beyond the cap"
```

---

## Task 3: `dial-stdio` cleanup systemd unit templates

**Files:**
- Modify: `src/install/templates.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `src/install/templates.rs`:

```rust
    #[test]
    fn dial_stdio_cleanup_timer_runs_hourly() {
        let timer = dial_stdio_cleanup_timer();
        assert!(timer.contains("OnCalendar=hourly"));
        assert!(timer.contains("[Timer]"));
    }

    #[test]
    fn dial_stdio_cleanup_service_kills_orphaned_dial_stdio_processes() {
        let service = dial_stdio_cleanup_service();
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("ExecStart=/usr/bin/pkill -f 'docker system dial-stdio'"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib dial_stdio_cleanup`
Expected: FAIL with "cannot find function `dial_stdio_cleanup_timer`" (compile error).

- [ ] **Step 3: Write the minimal implementation**

Add to `src/install/templates.rs` (near `logrotate_template`):

```rust
/// systemd timer unit for the hourly dial-stdio cleanup. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §2.
pub fn dial_stdio_cleanup_timer() -> &'static str {
    r#"[Unit]
Description=Hourly cleanup of orphaned docker dial-stdio helper processes

[Timer]
OnCalendar=hourly
Persistent=true

[Install]
WantedBy=timers.target
"#
}

/// systemd service unit for the hourly dial-stdio cleanup. Intentionally
/// blunt: it does not distinguish orphaned dial-stdio processes from ones
/// serving an active `docker` CLI command, since litehouse's production
/// code never shells out to the `docker` CLI. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §2.
pub fn dial_stdio_cleanup_service() -> &'static str {
    r#"[Unit]
Description=Kill orphaned docker system dial-stdio helper processes

[Service]
Type=oneshot
ExecStart=/usr/bin/pkill -f 'docker system dial-stdio'
# pkill exits 1 when no matching process is found - that's the common
# case (nothing to clean up), not a failure.
SuccessExitStatus=0 1
"#
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib dial_stdio_cleanup`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/install/templates.rs
git commit -m "feat: add dial-stdio cleanup systemd unit templates"
```

---

## Task 4: Weekly reboot systemd unit templates

**Files:**
- Modify: `src/install/templates.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module at the bottom of `src/install/templates.rs`:

```rust
    #[test]
    fn weekly_reboot_timer_runs_sunday_3am_eastern() {
        let timer = weekly_reboot_timer();
        assert!(timer.contains("OnCalendar=Sun *-*-* 03:00:00 America/New_York"));
        assert!(timer.contains("[Timer]"));
    }

    #[test]
    fn weekly_reboot_service_reboots_the_host() {
        let service = weekly_reboot_service();
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("ExecStart=/usr/bin/systemctl reboot"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib weekly_reboot`
Expected: FAIL with "cannot find function `weekly_reboot_timer`" (compile error).

- [ ] **Step 3: Write the minimal implementation**

Add to `src/install/templates.rs`:

```rust
/// systemd timer unit for the weekly host reboot. Matches the "3am US
/// Eastern" convention already established by the nightly app-restart
/// feature; systemd resolves the trailing timezone against tzdata
/// (DST-safe) without changing the host's system timezone. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §3.
pub fn weekly_reboot_timer() -> &'static str {
    r#"[Unit]
Description=Weekly host reboot to bound dial-stdio process accumulation

[Timer]
OnCalendar=Sun *-*-* 03:00:00 America/New_York
Persistent=true

[Install]
WantedBy=timers.target
"#
}

/// systemd service unit for the weekly host reboot. Safe because every
/// container (litehouse-server, caddy, and every app) runs with
/// --restart=unless-stopped/always, so a reboot is self-healing. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §3.
pub fn weekly_reboot_service() -> &'static str {
    r#"[Unit]
Description=Weekly host reboot

[Service]
Type=oneshot
ExecStart=/usr/bin/systemctl reboot
"#
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib weekly_reboot`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/install/templates.rs
git commit -m "feat: add weekly reboot systemd unit templates"
```

---

## Task 5: Install phase functions for both timers

**Files:**
- Modify: `src/install/phases.rs`

- [ ] **Step 1: Add the two phase functions**

Add after `phase10_enable_docker_restart` (`src/install/phases.rs:363-372`), before `phase11_verification`:

```rust
/// Phase 12: Hourly dial-stdio cleanup timer
#[instrument]
pub fn phase12_dial_stdio_cleanup_timer() -> Result<()> {
    info!("Phase 12: Dial-stdio cleanup timer");

    sudo_write_file(
        "/etc/systemd/system/litehouse-dial-stdio-cleanup.service",
        templates::dial_stdio_cleanup_service(),
    )?;
    sudo_write_file(
        "/etc/systemd/system/litehouse-dial-stdio-cleanup.timer",
        templates::dial_stdio_cleanup_timer(),
    )?;

    run_command("sudo systemctl daemon-reload")?;
    run_command("sudo systemctl enable --now litehouse-dial-stdio-cleanup.timer")?;

    info!("Phase 12 completed successfully");
    Ok(())
}

/// Phase 13: Weekly host reboot timer
#[instrument]
pub fn phase13_weekly_reboot_timer() -> Result<()> {
    info!("Phase 13: Weekly reboot timer");

    sudo_write_file(
        "/etc/systemd/system/litehouse-weekly-reboot.service",
        templates::weekly_reboot_service(),
    )?;
    sudo_write_file(
        "/etc/systemd/system/litehouse-weekly-reboot.timer",
        templates::weekly_reboot_timer(),
    )?;

    run_command("sudo systemctl daemon-reload")?;
    run_command("sudo systemctl enable --now litehouse-weekly-reboot.timer")?;

    info!("Phase 13 completed successfully");
    Ok(())
}
```

Both are idempotent by construction: `sudo_write_file` overwrites unconditionally, and `systemctl enable --now` on an already-enabled timer is a no-op — no "already applied" detection needed (per spec §4).

- [ ] **Step 2: Compile-check**

Run: `cargo build --lib`
Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/install/phases.rs
git commit -m "feat: add install phases for dial-stdio cleanup and weekly reboot timers"
```

---

## Task 6: Wire both phases into `lh install`

**Files:**
- Modify: `src/commands/install.rs`

- [ ] **Step 1: Bump `total_phases` and add the two phase calls**

Change line 73 (`src/commands/install.rs`):

```rust
    let total_phases = if skip_verify { 13 } else { 14 };
```

Insert after the Phase 10 block and before the Phase 11 block (`src/commands/install.rs:287-305`):

```rust
    // Phase 10: Docker restart configuration
    pb.set_message("Configuring Docker restart policy...");
    if let Err(e) = phase10_enable_docker_restart(&litehouse_uid) {
        pb.finish_with_message("❌ Docker restart configuration failed");
        error!("Phase 10 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 12: Hourly dial-stdio cleanup timer
    pb.set_message("Enabling dial-stdio cleanup timer...");
    if let Err(e) = phase12_dial_stdio_cleanup_timer() {
        pb.finish_with_message("❌ Dial-stdio cleanup timer setup failed");
        error!("Phase 12 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 13: Weekly reboot timer
    pb.set_message("Enabling weekly reboot timer...");
    if let Err(e) = phase13_weekly_reboot_timer() {
        pb.finish_with_message("❌ Weekly reboot timer setup failed");
        error!("Phase 13 failed: {}", e);
        return Err(e);
    }
    pb.inc(1);

    // Phase 11: Verification (optional)
    if !skip_verify {
        pb.set_message("Verifying server is responding...");
        if let Err(e) = phase11_verification(domain, &admin_label) {
            pb.finish_with_message("❌ Verification failed");
            error!("Phase 11 failed: {}", e);
            return Err(e);
        }
        pb.inc(1);
    }
```

(Phase numbers stay `12`/`13` per the spec's naming even though they now run before phase 11 in execution order — verification is deliberately last regardless of numbering, matching its existing "optional, last" role.)

- [ ] **Step 2: Compile-check**

Run: `cargo build --lib`
Expected: compiles cleanly (the `phases::*` glob import at the top of `install.rs` already covers the two new functions).

- [ ] **Step 3: Commit**

```bash
git add src/commands/install.rs
git commit -m "feat: enable dial-stdio cleanup and weekly reboot timers during install"
```

---

## Task 7: Re-apply both phases on every `lh upgrade`

**Files:**
- Modify: `src/commands/upgrade.rs`

- [ ] **Step 1: Import the two phase functions and call them idempotently**

Change the import at the top of `src/commands/upgrade.rs` (currently lines 6-8):

```rust
use crate::install::phases::{
    get_litehouse_uid, phase12_dial_stdio_cleanup_timer, phase13_weekly_reboot_timer,
    phase6a_pull_litehouse_image, phase9b_start_litehouse_container,
};
```

Insert a new step between the existing "Phase 3: Restart litehouse-server container" block and "Phase 4: Install the new host binary" block (`src/commands/upgrade.rs:276-295`) — after the `pb.inc(1);` that follows Phase 3, before the Phase 4 comment:

```rust
    pb.inc(1);

    // Phase 3b: Re-apply maintenance timers. Idempotent (see phase12/13
    // doc comments) so every upgrade re-applies the latest unit-file
    // content rather than only setting it up once at install time - any
    // future edits to the cleanup cadence propagate on the next upgrade
    // instead of silently drifting from what's on disk.
    pb.set_message("Re-applying maintenance timers...");
    if let Err(e) = phase12_dial_stdio_cleanup_timer() {
        pb.finish_with_message("❌ Dial-stdio cleanup timer setup failed");
        if from_path.is_none() {
            cleanup_temp_dir(&binary_path).ok();
        }
        return Err(e);
    }
    if let Err(e) = phase13_weekly_reboot_timer() {
        pb.finish_with_message("❌ Weekly reboot timer setup failed");
        if from_path.is_none() {
            cleanup_temp_dir(&binary_path).ok();
        }
        return Err(e);
    }

    // Phase 4: Install the new host binary. The container is already
```

This new step is bundled into the existing "restart container" progress slot rather than given its own `pb.inc(1)` — `total_phases` (`src/commands/upgrade.rs:191`, `let total_phases = if from_path.is_some() { 3 } else { 4 };`) stays unchanged.

- [ ] **Step 2: Compile-check**

Run: `cargo build --lib`
Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/commands/upgrade.rs
git commit -m "feat: re-apply maintenance timers on every lh upgrade"
```

---

## Task 8: Fix stale `CLAUDE.md` doc comment

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Replace the stale "Socket resolution" bullet**

In `CLAUDE.md`, replace line 99:

```
**Socket resolution:** Checks `DOCKER_SSH_SOCK`, `DOCKER_SOCK`, `CONTAINER_HOST` env vars, then queries `docker system connection ls` for default connection. On macOS with Docker Machine, uses `docker machine inspect` to find the forwarded socket.
```

with:

```
**Socket resolution:** Checks `DOCKER_HOST` (stripping a leading `unix://` if present), then falls back to `/var/run/docker.sock` regardless of whether that path actually exists (see `resolve_docker_socket_path()` in `src/docker.rs`).
```

- [ ] **Step 2: Verify against the actual implementation**

Run: `grep -n "fn resolve_docker_socket_path" -A 20 src/docker.rs`
Expected: confirms the function only checks `DOCKER_HOST`, then checks `/var/run/docker.sock` existence, defaulting to that path either way — matching the new doc text.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: fix stale description of resolve_docker_socket_path"
```

---

## Task 9: Build, deploy, and manually verify on production

**Files:** none (operational step)

- [ ] **Step 1: Run the full test suite locally**

Run: `cargo test --lib`
Expected: PASS. (This runs the fast unit tests only — the Docker-integration tests behind `#[ignore]` are not run here and don't require a live server.)

- [ ] **Step 2: Build and deploy to the production droplet**

Run: `./dev-deploy.sh`
Expected: builds the musl release binary, uploads it to `root@104.248.15.20`, and runs `lh upgrade --from-path /tmp/lh` on the server — which now also re-applies the two new phases per Task 7.
Expected final output: `✓ litehouse upgraded successfully` (or the binary-install-warning variant, which still means the container and timers are current).

- [ ] **Step 3: SSH in and verify both timers are active**

Run: `ssh root@104.248.15.20 "systemctl list-timers litehouse-*"`
Expected: both `litehouse-dial-stdio-cleanup.timer` and `litehouse-weekly-reboot.timer` listed, each with a `NEXT` time in the future.

Run: `ssh root@104.248.15.20 "systemctl status litehouse-dial-stdio-cleanup.timer litehouse-dial-stdio-cleanup.service litehouse-weekly-reboot.timer litehouse-weekly-reboot.service"`
Expected: all four units show `enabled`/`active` (the two `.service` units show `inactive (dead)` between runs, which is normal for `Type=oneshot` — confirm via `systemctl status` that the *last* run, if any, exited `0`).

- [ ] **Step 4: Verify the memory cap landed on a live app container**

Run: `ssh root@104.248.15.20 "docker inspect <any-app>-container --format '{{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}'"`
Expected: `268435456 268435456` (256MB in bytes) for an app that hasn't overridden the limit, once that app has been recreated at least once since this deploy (immediately if freshly deployed/started; otherwise it picks the cap up at the next nightly restart per spec §1 — don't force an unnecessary restart just to check this early).

- [ ] **Step 5: Confirm no regressions to existing apps**

Run: `ssh root@104.248.15.20 "docker ps --format '{{.Names}}\t{{.Status}}'"`
Expected: all previously-running app containers, plus `caddy-container` and `litehouse-server`, still show `Up`.
