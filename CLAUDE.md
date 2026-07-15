# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**litehouse** is a self-hosted, single-server platform for SQLite apps, similar to Vercel but self-hosted: `lh create` once, `git push` to deploy forever. The server never builds anything — GitHub Actions builds images and pushes them to GHCR, a per-app deploy hook on the server pulls and runs them, and a built-in daily job backs up app data to S3.

The system consists of:
- **CLI client** (`lh`) - Local tool for managing apps (create, deploy, start, stop, logs, backup, restore, etc.)
- **litehouse-server container** - HTTP server with admin API + deploy hook + server-rendered admin UI, runs as a Docker container (NOT a systemd service)
- **Caddy container** - Reverse proxy with automatic HTTPS and subdomain routing
- **App containers** - User applications, each in their own container, images pulled from GHCR
- **Docker integration** - Container orchestration via Bollard (Docker API client)
- **SQLite database** - App state, deploy history, and environment variables
- **GitHub Actions** - Builds each app's image and pushes it to `ghcr.io/{owner}/{app}`, then calls the server's deploy hook
- **S3** - Daily backup destination (app data + the server's own state DB)

**IMPORTANT DEPLOYMENT DETAILS:**
- The litehouse server itself runs as a Docker container named `litehouse-server`
- Check server status: `docker ps | grep litehouse-server` (NOT `systemctl status litehouse`)
- Restart server: Handled by container restart policy or `docker restart litehouse-server`
- Server binary at `/usr/local/bin/lh` is only used for CLI operations and install/upgrade tasks
- After installation, the `lh serve` command runs inside the `litehouse-server` container

## Build & Development Commands

```bash
# Build the project
cargo build

# Run the server (starts HTTP API and reverse proxy)
cargo run -- serve

# Run tests (includes comprehensive Docker integration tests)
cargo test

# Run specific test
cargo test test_run_function_happy_path

# Build for production (Linux musl target)
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl

# Deploy to server (use dev-deploy.sh)
./dev-deploy.sh
```

## Architecture Overview

### Current State (v2 shipped)

The v2 refactor (external builds via GHCR, push-to-deploy, daily S3 backups, single admin token, server-rendered admin UI, disaster recovery) is **complete**. See VISION.md for the roadmap and `docs/superpowers/specs/2026-07-03-litehouse-v2-design.md` for the original design doc.

**v2 highlights:**
- `lh create <app> --repo owner/name` registers the app, mints a deploy token, commits a GitHub Actions workflow, and sets the deploy-token secret — `git push` deploys from then on
- GitHub Actions builds the image, pushes to `ghcr.io/{owner}/{app}`, then POSTs the server's deploy hook (`POST /api/hooks/deploy`, per-app bearer token)
- Server pulls the image, recreates the container, syncs Caddy — previous container stays up until the new one is healthy
- `lh deploys <app> --wait` blocks until the in-flight deploy succeeds or fails (exit 0/1/2) — the CI/agent verification primitive
- Single admin token (sha256 hash stored server-side); `lh connect <url> --token <TOKEN>` — no users/orgs/JWT
- Daily backups (VACUUM INTO snapshots, tar.gz to S3, 14-day retention); `lh backup run` / `lh backup status --json`
- Incremental blob backup: apps get `LITEHOUSE_BLOB_PATH=/data/blobs` and anything written there is backed up to its own S3 prefix (`blobs/{app_name}/...`, NOT nested under `apps/{app_name}/`) on an upload-once basis — unchanged files are never re-uploaded. Restored automatically as part of `lh restore --yes`. See `docs/superpowers/specs/2026-07-14-blob-backup-design.md`.
- Nightly app restart: every running app is restarted once a night at 3am US Eastern time (best-effort maintenance, not a redeploy — same image, just a fresh container). An app can opt out via `lh env set <app> LITEHOUSE_SKIP_NIGHTLY_RESTART true`. Apps mid-deploy or otherwise locked are skipped for that night rather than delayed. See `docs/superpowers/specs/2026-07-15-nightly-app-restart-design.md`.
- Disaster recovery: `lh install --domain ...` on a fresh node → `lh connect` → `lh restore --yes` rebuilds state, apps, and volumes from GHCR + S3
- Server-rendered admin UI (Askama + HTMX) served from the same binary, cookie-authenticated with the admin token
- `e2e/acceptance.sh` and `e2e/dr-drill.sh` automate the full push-to-deploy and disaster-recovery flows against a real droplet; `examples/hello` is the reference app used by both

### Key Components

#### 1. Client-Server Model

The CLI (`src/cli.rs`) sends HTTP requests to the server API (`src/api.rs`) via `ApiClient` (`src/api_client.rs`). The server runs on a configurable host/port (default: localhost:3030).

**Client config:** `~/.config/litehouse/client-config.toml`
```toml
base_url = "http://admin.localhost"
```

#### 2. Database Schema

SQLite database with tables:
- `app` - Core app records (id, name, state, port, repo, image, exposed_port, deploy_token_hash)
- `deploy` - Deploy history (app_id, image, git_sha, status, error, timestamps)
- `env_var` - Environment variables per app
- `system_config` - Server-wide settings (S3 credentials, GHCR read token, admin token hash)

See `migrations/20250403_initial.sql` for the original schema and `migrations/20260703_v2_simplify.sql` for the v2 rebuild (drops `build`, `remote`, `state_change`, and all multi-user auth tables; adds `deploy` and the new `app` columns).

#### 3. Container Management (Docker)

**Module:** `src/docker.rs`

Uses Bollard to communicate with Docker via Unix socket. Key functions:
- `build(directory, tag)` - Builds Docker image from Dockerfile using `docker build` CLI
- `run(name, image_tag)` - Creates and starts a container (idempotent, skips if already running)
- `stop(app)` - Stops a running container
- `logs_stream(app_name, lines, follow)` - Streams container logs
- `connect()` - Establishes Docker API connection via `resolve_docker_socket_path()`

**Socket resolution:** Checks `DOCKER_SSH_SOCK`, `DOCKER_SOCK`, `CONTAINER_HOST` env vars, then queries `docker system connection ls` for default connection. On macOS with Docker Machine, uses `docker machine inspect` to find the forwarded socket.

**Naming convention:** Container name = `{app-name}-container`

#### 4. Reverse Proxy (Caddy)

**Module:** `src/caddy.rs`

Manages a Caddy container for reverse proxying to app containers:
- `start(docker, config)` - Ensures Caddy container is running with correct ports
- `sync_configuration(docker, db_pool)` - Rebuilds Caddy config from database apps and sends to Caddy API

**Configuration generation:**
- Local dev: Routes `{app-name}.localhost` on ports 9090/9443
- Production: Routes `{app-name}.s.danbruder.com` on ports 80/443
- Updates sent to Caddy's admin API at `http://localhost:2019/load`

**Environment detection:** Checks `LITEHOUSE_LOCAL_DEV`, `RUST_LOG`, or `debug_assertions` to determine local vs production mode.

#### 5. Deploy Engine (no server-side builds)

**Module:** `src/deploy.rs` — the single code path behind both the public GitHub deploy hook (`POST /api/hooks/deploy`) and an admin-triggered redeploy.

The server never runs `docker build`. Images are built by GitHub Actions (workflow committed by `lh create`) and pushed to `ghcr.io/{owner}/{app}`. Flow:
1. Workflow calls the deploy hook with `{image, sha}` and a per-app bearer token
2. `deploy::verify_deploy_token` checks the token against `app.deploy_token_hash` (sha256, constant-time compare)
3. `deploy::deploy_app` pulls the image, recreates the container, syncs Caddy, and records the outcome in the `deploy` table
4. The previous container keeps running until the new image pulls successfully — a failed pull leaves the old deploy untouched

**GitHub integration:** `src/github/` — device-flow OAuth (client-side only; the server never talks to the GitHub API) used by `lh create` to commit `.github/workflows/litehouse-deploy.yml` and set the `LITEHOUSE_DEPLOY_TOKEN` repo secret. `src/workflow.rs` renders the workflow template.

#### 6. Command Structure

Commands in `src/commands/` follow a pattern:
```rust
pub async fn execute(pool: &Pool<Sqlite>, ...) -> Result<()>
```

Each command module corresponds to a CLI subcommand:
- `create.rs` - Register app, commit deploy workflow, set repo secret (idempotent via `--rotate-token`)
- `delete.rs` - Delete app (removes from DB and stops container)
- `start.rs` / `stop.rs` - Start/stop the app container
- `status.rs` - Show one app's or all apps' status
- `logs.rs` - Fetch container logs
- `app_env.rs` - Set/delete environment variables
- `install.rs` / `upgrade.rs` - Server install/upgrade bash stages
- `check_dns.rs` - Verify wildcard DNS for the configured domain
- `github_login.rs` - Device-flow OAuth for `lh github login`
- `server.rs` - `lh serve` (the admin API + deploy hook + UI + backup scheduler)

Deploy, backup, and restore logic lives in top-level modules (`src/deploy.rs`, `src/backup.rs`) rather than under `commands/`, since they're invoked from both the CLI and the HTTP API.

#### 7. App Lifecycle & State

**App states:** `created`, `building`, `starting`, `running`, `stopping`, `stopped`, `failed`, `crashed` (`src/models/app_state.rs`). `building` is a holdover name — nothing on the server builds; it is unused in the v2 deploy path.

**State tracking:** Currently stored in database but source of truth is the actual Docker container state. The system queries Docker directly for current status rather than relying on cached state.

**Port assignment:** Apps are assigned a port (stored in `app.port`). The Caddy proxy routes subdomain traffic to `0.0.0.0:{port}`.

### Data Flow Examples

**Onboarding and deploying an app (the "drunk-proof" path):**
1. `lh create myapp --repo owner/myapp` → registers the app, mints a deploy token, commits `.github/workflows/litehouse-deploy.yml`, sets the `LITEHOUSE_DEPLOY_TOKEN` repo secret
2. `git push` → GitHub Actions builds the image, pushes to `ghcr.io/owner/myapp`, POSTs the deploy hook
3. Server pulls the image, recreates the container, syncs Caddy config, records the deploy
4. `lh deploys myapp --wait` blocks until that deploy succeeds or fails (exit 0/1/2)
5. App accessible at `myapp.{domain}` over HTTPS

**Server startup:**
1. Server starts → Connects to SQLite and Docker
2. Ensures Caddy container is running
3. Syncs Caddy config with all apps that have ports
4. Starts HTTP server with admin API and reverse proxy routes

## Important Patterns & Decisions

### Docker vs Docker
- Uses Bollard (Docker API client) to communicate with Docker
- Some operations use direct `docker` CLI (e.g., `build`) due to API limitations
- Socket path resolution is critical on macOS with Docker Machine

### Container Identity
- Container names follow pattern: `{app-name}-container`
- Images tagged as: `{app-name}:latest`
- Always check if container exists before creating to avoid conflicts

### Error Handling
- Most functions return `Result<T>` with `anyhow::Error`
- Custom error types: `DockerError`, `GitError`
- Extensive use of `#[instrument]` for tracing

### Database Access
- Database modules in `src/db/` (app.rs, deploy.rs, env_var.rs, system_config.rs)
- Uses SQLx with compile-time query verification
- Models in `src/models/` define domain types

### Testing
- Extensive integration tests in `src/docker.rs` (`#[cfg(test)] mod tests`)
- Tests create real containers using Docker
- Test helpers for container cleanup and state verification
- Tests assume Docker is installed and running

## Configuration

**Client config:** `~/.config/litehouse/client-config.toml`
- `base_url` - Server API endpoint

**Server config:** Loaded via `ServerConfig::load()` (see `src/config.rs`)
- Host and ports for proxy, Caddy HTTP/HTTPS

**Environment variables:**
- `DOCKER_SSH_SOCK`, `DOCKER_SOCK`, `CONTAINER_HOST` - Override socket path
- `LITEHOUSE_LOCAL_DEV` or `RUST_LOG` - Enable local dev mode
- `DATABASE_URL` - SQLite database path (default: `~/.local/share/litehouse/litehouse.db`)
- `DOCKER_API_VERSION=1.42` - Needed to run the ignored Docker-integration tests locally (e.g. `test_backup_roundtrip_minio`, `test_restore_roundtrip_minio`)

**Server-side settings** (stored in DB via `lh config`, not env vars): S3 credentials/bucket/region/endpoint/path-prefix (`lh config s3 set`), GHCR read token for pulling private images (`lh config ghcr set --token`). Both are also collectible up front at `lh install --s3-* --ghcr-token`.

## Operational Notes

**Checking Server Status:**
```bash
# Check if litehouse-server container is running
docker ps | grep litehouse-server

# View litehouse-server logs
docker logs litehouse-server -f

# Check Caddy container
docker ps | grep caddy-container

# Restart litehouse-server
docker restart litehouse-server
```

**DO NOT use systemctl** - litehouse does not run as a systemd service.

**Common Troubleshooting:**
- If apps aren't accessible: Check `docker logs caddy-container` for routing issues
- If a deploy fails: Check `docker logs litehouse-server` for deploy-hook errors, then `lh deploys <app>` (or `--json`) for the recorded error message
- If GitHub Actions can't reach the deploy hook: verify the `LITEHOUSE_DEPLOY_TOKEN` repo secret is set (`lh create <app> --rotate-token` re-mints it and re-commits the workflow)
- If containers won't start: Check Docker socket is accessible at `/var/run/docker.sock`
- If backups are missing: `lh backup status --json` shows the last successful date and the last report; a backup day is only stamped when every app backs up with zero failures
- If disaster recovery fails: confirm `lh config s3 get` / `lh config ghcr get` are populated on the fresh install before running `lh restore --yes`
- If an app restarted unexpectedly overnight: check `docker logs litehouse-server` for `"nightly app restart complete"` around 3am Eastern — it logs which apps were restarted, skipped (and why), or failed. Opt an app out with `lh env set <app> LITEHOUSE_SKIP_NIGHTLY_RESTART true`.

## Known Issues & TODOs

See VISION.md for complete roadmap. Current status:
- v2 (external builds via GHCR, push-to-deploy, daily S3 backups, single-token auth, admin UI, DR) - ✅ SHIPPED
- Phase 3 (Cloudflare DNS automation, custom domains, multi-arch server image) - Next priority

## Development Context

**Target use case:** Self-hosted SQLite apps with automatic backups, similar to Vercel's deployment model but optimized for single-host, SQLite-first workloads.

**Architecture decision:** Litehouse runs as a Docker container to ensure consistent deployment, easy upgrades, and isolation from the host system. The `lh` CLI binary is installed on the host for administration, but the server itself runs containerized.

## The server 

Working with a server right now that you can access at root@104.248.15.20 (hostname `litehouse-1`); feel free to ssh into the server if required. Also no production workloads are running on it so feel free to make changes. If you are fixing an issue with the server, ALWAYS loop the learnings back into the code (install/upgrade script etc) so that future installs don't have the defect. 

It is a digital ocean server that I'm happy to wipe clean and reinstall if needed. 

Cloudflare manages the domain (danbruder.com) and it points `*.lh.danbruder.com` to the IP address above.

When re-installing, use the lh.danbruder.com domain.

