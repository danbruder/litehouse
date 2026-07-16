# Server Hardening: Memory Caps + Host Maintenance Timers — Design

## Motivation

An OOM incident took down every app on the litehouse droplet (2GB RAM). Root causes:

1. **No per-app container memory cap** — one app's memory growth (or a kernel-level OOM sweep triggered by something else) could take down the whole host with no isolation between apps.
2. **A slow leak of orphaned `docker system dial-stdio` helper processes** from manual SSH-based `docker` CLI debugging sessions, accumulated over ~13 days (322 processes, ~2.7GB RAM+swap) until the host tipped into OOM. Confirmed NOT caused by litehouse's own code — its production binary never shells out to the `docker` CLI (all Docker operations go through Bollard directly against the socket); every `Command::new("docker")` call site in the repo is inside `#[cfg(test)]` blocks.
3. **A restart-loop app (`gex`) with no ceiling** — once OOM-killed, its unconditional `restart: always` policy relaunched it immediately, uncapped, so it cycled fastest during the crisis and was the last container to stabilize.

This spec covers three independent hardening changes plus one documentation fix, bundled together since they're all part of the same incident response.

## 1. Per-app container memory cap

**File:** `src/docker.rs`, in `run()`'s `HostConfig` construction (currently `HostConfig::default()` with no memory field — see the `HostConfig`/`RestartPolicy` block already there).

- Add `memory: Some(bytes)` and `memory_swap: Some(bytes)` (same value as `memory`) to the `HostConfig`. Setting `memory_swap == memory` means the container gets **no swap beyond its memory limit** — this directly prevents the swap-exhaustion pattern from the incident, where OOM'd containers kept swapping instead of being contained.
- Default: **256MB** (`DEFAULT_APP_MEMORY_LIMIT_MB: i64 = 256`). Rationale: 7 apps × 256MB = 1.8GB worst-case ceiling if every app hit its cap simultaneously, leaving headroom for `litehouse-server`/`caddy`/OS on a 1.9GB box. Loose enough for a typical small Phoenix/Node app's baseline footprint.
- Per-app override: a new env var, `LITEHOUSE_MEMORY_LIMIT_MB`, read the same way the existing `LITEHOUSE_SKIP_NIGHTLY_RESTART`/`LITEHOUSE_BLOB_PATH` opt-outs are — `start_container` (`src/commands/start.rs`) already loads `env_vars` via `db::env_var::get_by_app` before calling `docker::run()`; check for this key there, parse it as an integer count of MB, and pass the resolved limit (override or default) through to `docker::run()`. Set via `lh env set <app> LITEHOUSE_MEMORY_LIMIT_MB 512` — no new CLI surface needed.
- **No migration/backfill needed for already-running containers.** The cap only applies at container-creation time. Since the nightly-restart feature (shipped previously) already stops+starts every running app once a night, every app organically picks up the new cap within 24 hours of this change deploying — no special one-time recreation logic required (YAGNI).
- Out of scope: capping `litehouse-server`'s or `caddy`'s own containers — those are platform infrastructure, not user apps, and are provisioned via a separate code path (`src/install/templates.rs`'s container-start scripts, not `docker::run()`). Not addressed here.

## 2. Hourly dial-stdio cleanup (host-level systemd timer)

New systemd unit pair: `litehouse-dial-stdio-cleanup.service` + `.timer`.

- `.timer`: `OnCalendar=hourly`.
- `.service`: `Type=oneshot`, `ExecStart=/usr/bin/pkill -f 'docker system dial-stdio'` (or equivalent — exact match string must not also match the `grep`/`pkill` invocation itself; use `pkill -f` with the literal string, which `pgrep`/`pkill` already exclude their own process from by default).
- Intentionally blunt: it does not try to distinguish orphaned dial-stdio processes from ones serving an active `docker` CLI command. Given litehouse's production code never invokes the `docker` CLI and nothing else on the box holds a legitimate long-running CLI session, the only realistic collateral is interrupting someone's manual `docker logs -f`/`docker stats` session — self-evident and harmless (just rerun the command). This is deemed an acceptable, simple tradeoff vs. building parent-process-liveness detection.

## 3. Weekly host reboot (host-level systemd timer)

New systemd unit pair: `litehouse-weekly-reboot.service` + `.timer`.

- `.timer`: `OnCalendar=Sun *-*-* 03:00:00 America/New_York`. systemd resolves the trailing timezone directly against tzdata (DST-safe), without needing to change the host's system timezone — matches the "3am US Eastern" convention already established by the in-app nightly-restart feature.
- `.service`: `Type=oneshot`, `ExecStart=/usr/bin/systemctl reboot`.
- Safety basis: every container (`litehouse-server`, `caddy`, and every app) already runs with `--restart unless-stopped`/`always`, so a full reboot is self-healing — this is the exact recovery path already validated live during the incident (`systemctl restart docker`/`containerd` brought everything back without manual container-by-container intervention).
- Rationale for weekly (not nightly): the dial-stdio leak accumulates at roughly ~25 processes/day; a 7-day window gives comfortable headroom before it could approach the ~322-process level that caused the incident, while avoiding a nightly full-outage window on top of the existing nightly app-restart.

## 4. Bootstrap wiring

- Add two new phase functions to `src/install/phases.rs` (existing phases run through `phase11_verification` — the two new ones are `phase12_dial_stdio_cleanup_timer` and `phase13_weekly_reboot_timer`), following phase 8's (`phase8_log_rotation`) existing pattern: build the unit file contents as template strings in `src/install/templates.rs`, write them via `sudo_write_file` to `/etc/systemd/system/`, then `systemctl daemon-reload` and `systemctl enable --now <name>.timer`.
- `lh upgrade` (`src/commands/upgrade.rs`) does **not** currently re-run `install.rs`'s phases — it only swaps the binary/image and restarts the `litehouse-server` container. Extend `upgrade.rs` to also idempotently re-apply these two new phases (and only these — not the full install flow) on every upgrade run, so:
  - This one upcoming `lh upgrade` run brings the already-installed production server (104.248.15.20) current, without a separate manual SSH step.
  - Any future edits to the unit files (e.g. changing the cleanup cadence) automatically propagate to production on the next upgrade, rather than silently drifting.
- Both phase functions must be idempotent (safe to re-run): writing the same unit file content again and re-running `enable --now` on an already-enabled timer is a no-op in systemd, so no special "already applied" detection is needed.

## 5. Documentation fix

`CLAUDE.md`'s description of `resolve_docker_socket_path()` (in the "Socket resolution" bullet under Container Management) currently says it "queries `docker system connection ls` for default connection... uses `docker machine inspect`" — this is stale; that logic was removed in a past dead-code sweep. Update it to describe the actual current behavior: checks `DOCKER_HOST`/other env var overrides, then falls back to checking whether `/var/run/docker.sock` exists, defaulting to that path either way.

## Testing

- Unit tests for the memory-limit resolution logic (default value used when no env var set; override value used and correctly parsed when `LITEHOUSE_MEMORY_LIMIT_MB` is set; a malformed value falls back to the default rather than panicking) — pure function, testable without Docker.
- Unit tests for the two new phase functions asserting on the generated unit-file content (matching the existing `#[cfg(test)]` convention in `templates.rs`, e.g. asserting the `OnCalendar=` lines and `ExecStart=` lines are present and correct), following the same pattern as existing template tests.
- No new Docker integration test is added for the memory cap itself (asserting an OOM-kill actually occurs under a 256MB cap would require deliberately spinning up a memory-hungry test container, which is disproportionate for this change) — the existing `restart_one_app_restarts_a_running_container` ignored integration test will continue to pass and implicitly exercises the new `HostConfig` fields via `docker::run()`.
- The two systemd timers are validated manually against the production server (SSH in, confirm `systemctl list-timers` shows both, confirm `systemctl status` for each after their first `enable --now`) as part of the deploy step in the implementation plan — not automatable in CI.

## Non-goals

- No cap on `litehouse-server`'s or `caddy`'s own containers (see §1).
- No attempt to fix the dial-stdio leak at its true source (manual SSH debugging habits) — the cleanup timer is a mitigation, not a prevention of the underlying practice.
- No configurable reboot schedule (fixed at weekly/Sunday 3am Eastern) — matches the nightly-restart feature's precedent of a fixed schedule; can be made configurable later if needed, but isn't required now.
- No monitoring/alerting on dial-stdio process count (mentioned as a nice-to-have in the original investigation) — out of scope for this spec; the hourly cleanup keeps the count bounded regardless of whether alerting exists.
