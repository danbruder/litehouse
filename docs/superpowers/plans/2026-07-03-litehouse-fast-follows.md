# Litehouse fast-follows (post-v2)

Three independent, small fixes surfaced during real-world app deploys. Each is isolated to different files → parallel-safe. Bundle into release **v0.2.0-alpha.7** after all three merge.

## Task A — `lh upgrade` self-overwrite bug
**Files:** `src/commands/upgrade.rs` (+ maybe `src/install/*`).
**Symptom:** `lh upgrade` fails with `cp: cannot create regular file '/usr/local/bin/lh': Text file busy` — it tries to overwrite its own running binary in place.
**Fix:** install the new binary via a path that doesn't require overwriting the running executable: write to a temp path on the same filesystem then `rename(2)`/`mv` over the target (atomic replace; Linux allows renaming over a busy executable because the running process holds the old inode). Verify the `cp` is replaced by a stage-then-`mv` (or `install` with a temp + mv). Keep the existing `.backup` copy behavior. Also ensure the image pull + `litehouse-server` container recreation happens **regardless of** the binary-install step outcome (today the flow aborts before recreating the container if the binary step errors — reorder so the container is recreated first, or make binary-install failure non-fatal with a clear warning).
**Verify:** `cargo build` clean, `cargo test` green. Can't fully e2e without the box; unit-test any extracted path helper. Report clearly what was reordered.

## Task B — implement `lh restart <app>`
**Files:** `src/cli.rs` (the `Commands::Restart` arm — currently a `println!` stub).
**Fix:** implement restart as stop-then-start using the EXISTING api_client methods (`stop_app` then `start_app`) — no new server route needed. Match the output style of the other commands (e.g. "App '<name>' restarted"). If `start_app` fails after stop, surface the error and non-zero exit. Check `src/api_client.rs` for the exact method names/signatures before wiring.
**Verify:** `cargo build` clean, `cargo test` green, `cargo run -- restart --help` renders.

## Task C — remove dead `docker::logs`
**Files:** `src/docker.rs`.
**Symptom:** `pub async fn logs(...)` contains `todo!("implement non-streaming logs")` and is never called (the real path is `logs_stream`, used by the API handler). A `todo!()` in non-test code is a latent panic trap.
**Fix:** delete the unused `logs` fn (and any now-unused imports it alone pulled in). Confirm via `grep -rn "docker::logs\b\|::logs(" src/` that nothing calls it (distinguish from `logs_stream`). Do NOT touch `logs_stream`.
**Verify:** `cargo build` clean (no unused-import warnings introduced), `cargo test` green.

## Integration (controller, after all merge)
Merge the three worktree branches, run full suite (`DOCKER_API_VERSION=1.42 cargo test` → expect ~150 green + 2 ignored), bump to `0.2.0-alpha.7`, tag, push, publish image. The `lh upgrade` fix gets its real end-to-end test by running `lh upgrade` on the box for the alpha.7 rollout.
