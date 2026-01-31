# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**litehouse** is a container-based platform for deploying and running containerized applications on a single host, similar to Vercel but self-hosted. The focus is on SQLite-backed applications with automatic backups.

The system consists of:
- **CLI client** (`lh`) - Local tool for managing apps (create, start, stop, build, logs, etc.)
- **litehouse-server container** - HTTP server with admin API, runs as a Docker container (NOT a systemd service)
- **Caddy container** - Reverse proxy with automatic HTTPS and subdomain routing
- **App containers** - User applications, each in their own container
- **Docker integration** - Container orchestration via Bollard (Docker API client)
- **SQLite database** - App state, builds, remotes, and environment variables

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

### Current State (Phase 1 Complete)

The V2 refactor to a container-based platform is **complete**. See VISION.md for the roadmap.

**Phase 1 Complete:**
- Create/delete apps
- Set environment variables
- Add/remove Git remotes
- Build apps from Git repos (creates Docker images)
- Start/stop containers
- View container logs
- Caddy reverse proxy with dynamic subdomain routing
- End-to-end subdomain routing working

**Next Priorities (Phase 2):**
- Web admin UI (Htmx + Tailwind)
- GitHub webhook integration
- Server initialization wizard (`lh server init`)

See VISION.md and NEXT_PRIORITY.md for complete roadmap.

### Key Components

#### 1. Client-Server Model

The CLI (`src/cli.rs`) sends HTTP requests to the server API (`src/api.rs`) via `ApiClient` (`src/api_client.rs`). The server runs on a configurable host/port (default: localhost:3030).

**Client config:** `~/.config/litehouse/client-config.toml`
```toml
base_url = "http://admin.localhost"
```

#### 2. Database Schema

SQLite database with tables:
- `app` - Core app records (id, name, state, port)
- `build` - Build history (image_id, image_tag, git_commit)
- `remote` - Git remote configuration (name, directory, remote, branch)
- `env_var` - Environment variables per app
- `state_change` - State transition history

See `migrations/20250403_initial.sql` for full schema.

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

#### 5. Build Process

**Flow:** App with remote → `git pull` → `docker build` → Store build record → Start container

**Module:** `src/commands/build.rs`

1. Fetch app and remote from database
2. Clone or pull Git repo into build directory (`{data_dir}/builds/{app_id}`)
3. Get Git commit hash
4. Build Docker image with tag `{app-name}:latest`
5. Store build record (app_id, image_id, image_tag, git_commit)

**Git module:** `src/git.rs` - Handles `git clone` and `git pull` operations

#### 6. Command Structure

Commands in `src/commands/` follow a pattern:
```rust
pub async fn execute(pool: &Pool<Sqlite>, ...) -> Result<()>
```

Each command module corresponds to a CLI subcommand:
- `create.rs` - Create new app
- `delete.rs` - Delete app (removes from DB and stops container)
- `start.rs` - Start container from latest build
- `stop.rs` - Stop running container
- `build.rs` - Build app from Git remote
- `logs.rs` - Fetch container logs
- `app_env.rs` - Set/delete environment variables
- `remote/` - Add/remove Git remotes

#### 7. App Lifecycle & State

**App states:** `created`, `building`, `running`, `stopped`, `error`

**State tracking:** Currently stored in database but source of truth is the actual Docker container state. The system queries Docker directly for current status rather than relying on cached state.

**Port assignment:** Apps are assigned a port (stored in `app.port`). The Caddy proxy routes subdomain traffic to `0.0.0.0:{port}`.

### Data Flow Examples

**Creating and deploying an app:**
1. `lh create myapp` → Creates DB record
2. `lh remote myapp add https://github.com/user/repo` → Stores Git remote
3. `lh build myapp` → Clones repo, runs `docker build`, stores build record
4. `lh start myapp` → Starts container from built image, syncs Caddy config
5. App accessible at `myapp.localhost:9090` (local) or `myapp.s.danbruder.com` (production)

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
- Database modules in `src/db/` (app.rs, build.rs, remote.rs, etc.)
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
- If builds fail: Check `docker logs litehouse-server` for API errors
- If containers won't start: Check Docker socket is accessible at `/var/run/docker.sock`

## Known Issues & TODOs

See VISION.md for complete roadmap. Current status:
- Phase 1 (Container platform) - ✅ COMPLETE
- Phase 2 (Web UI + webhooks) - Next priority
- Phase 3 (SQLite + Litestream) - Planned
- Phase 4 (DNS automation) - Planned

## Development Context

**Target use case:** Self-hosted SQLite apps with automatic backups, similar to Vercel's deployment model but optimized for single-host, SQLite-first workloads.

**Architecture decision:** Litehouse runs as a Docker container to ensure consistent deployment, easy upgrades, and isolation from the host system. The `lh` CLI binary is installed on the host for administration, but the server itself runs containerized.

## The server 

Working with a server right now that you can access at root@104.248.15.20; feel free to ssh into the server if required. Also no production workloads are running on it so feel free to make changes. If you are fixing an issue with the server, ALWAYS loop the learnings back into the code (install/upgrade script etc) so that future installs don't have the defect. 

It is a digital ocean server that I'm happy to wipe clean and reinstall if needed. 

Cloudflare manages the domain (litehouse.run) and it points `*.litehouse.run` to the IP address above.

When re-installing, use the litehouse.run domain.


## Beads Task Tracking

Use `bd` (beads) for all task/issue tracking instead of markdown plans.

### Quick Reference
```bash
bd init                    # Initialize in project (once)
bd ready --json            # Find work with no blockers
bd create "Title" -p 1 -t bug  # Create issue (priority 0-4, type: bug|feature|task|epic)
bd update <id> --status in_progress
bd close <id> --reason "Done"
bd dep add <child> <parent> --type discovered-from  # Link discovered work
bd list --status open --json
bd show <id>
bd sync                    # Sync with git
```

### Workflow
1. **Start session**: Run `bd ready --json` to find unblocked work
2. **During work**: File issues for bugs/tasks discovered with `bd create`, link via `bd dep add`
3. **End session**: Update statuses, close completed work, run `bd sync`

### Key Points
- Issues use hash IDs (e.g., `bd-a1b2`)
- Four dependency types: blocks, related, parent-child, discovered-from
- Data stored in `.beads/` (JSONL synced via git)
- Use `--json` flag for programmatic output

