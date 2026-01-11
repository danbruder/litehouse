# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**litehouse** is a platform for deploying and running containerized applications, transitioning from a static binary deployment system to a container-based platform using Podman and Caddy. The goal is to create a self-hosted platform similar to Vercel, with focus on Next.js applications.

The system consists of:
- **CLI client** - Local tool for managing apps (create, start, stop, build, logs, etc.)
- **Server** - HTTP server with admin API and reverse proxy
- **Podman integration** - Container orchestration via Bollard (Docker/Podman API)
- **Caddy integration** - Dynamic reverse proxy configuration
- **SQLite database** - App state, builds, remotes, and environment variables

## Build & Development Commands

```bash
# Build the project
cargo build

# Run the server (starts HTTP API and reverse proxy)
cargo run -- serve

# Run tests (includes comprehensive Podman integration tests)
cargo test

# Run specific test
cargo test test_run_function_happy_path

# Build for production (Linux musl target)
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl

# Deploy to server (use deploy.sh)
./deploy.sh
```

## Architecture Overview

### Current State (V2 Refactor)

The project is **mid-refactor** from a static binary deployment model to a container-based platform. See NOTES.md for detailed refactor notes and decisions.

**Completed:**
- Create/delete apps
- Set environment variables
- Add/remove Git remotes
- Build apps from Git repos (creates Docker images via Podman)
- Start/stop containers
- View container logs
- Caddy reverse proxy management

**In Progress:**
- Automatic subdomain routing (partial - Caddy config generation exists)
- GitHub webhook integration
- Restart functionality

### Key Components

#### 1. Client-Server Model

The CLI (`src/cli.rs`) sends HTTP requests to the server API (`src/api.rs`) via `ApiClient` (`src/api_client.rs`). The server runs on a configurable host/port (default: localhost:3030).

**Client config:** `~/.config/litehouse/client-config.toml`
```toml
base_url = "http://admin-api.localhost"
```

#### 2. Database Schema

SQLite database with tables:
- `app` - Core app records (id, name, state, port)
- `build` - Build history (image_id, image_tag, git_commit)
- `remote` - Git remote configuration (name, directory, remote, branch)
- `env_var` - Environment variables per app
- `state_change` - State transition history

See `migrations/20250403_initial.sql` for full schema.

#### 3. Container Management (Podman)

**Module:** `src/podman.rs`

Uses Bollard to communicate with Podman via Unix socket. Key functions:
- `build(directory, tag)` - Builds Docker image from Dockerfile using `podman build` CLI
- `run(name, image_tag)` - Creates and starts a container (idempotent, skips if already running)
- `stop(app)` - Stops a running container
- `logs_stream(app_name, lines, follow)` - Streams container logs
- `connect()` - Establishes Docker API connection via `resolve_podman_socket_path()`

**Socket resolution:** Checks `PODMAN_SSH_SOCK`, `PODMAN_SOCK`, `CONTAINER_HOST` env vars, then queries `podman system connection ls` for default connection. On macOS with Podman Machine, uses `podman machine inspect` to find the forwarded socket.

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

**Flow:** App with remote → `git pull` → `podman build` → Store build record → Start container

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

**State tracking:** Currently stored in database but source of truth is the actual Podman container state. The system queries Podman directly for current status rather than relying on cached state.

**Port assignment:** Apps are assigned a port (stored in `app.port`). The Caddy proxy routes subdomain traffic to `0.0.0.0:{port}`.

### Data Flow Examples

**Creating and deploying an app:**
1. `lh create myapp` → Creates DB record
2. `lh remote myapp add https://github.com/user/repo` → Stores Git remote
3. `lh build myapp` → Clones repo, runs `podman build`, stores build record
4. `lh start myapp` → Starts container from built image, syncs Caddy config
5. App accessible at `myapp.localhost:9090` (local) or `myapp.s.danbruder.com` (production)

**Server startup:**
1. Server starts → Connects to SQLite and Podman
2. Ensures Caddy container is running
3. Syncs Caddy config with all apps that have ports
4. Starts HTTP server with admin API and reverse proxy routes

## Important Patterns & Decisions

### Podman vs Docker
- Uses Bollard (Docker API client) to communicate with Podman
- Some operations use direct `podman` CLI (e.g., `build`) due to API limitations
- Socket path resolution is critical on macOS with Podman Machine

### Container Identity
- Container names follow pattern: `{app-name}-container`
- Images tagged as: `{app-name}:latest`
- Always check if container exists before creating to avoid conflicts

### Error Handling
- Most functions return `Result<T>` with `anyhow::Error`
- Custom error types: `PodmanError`, `GitError`
- Extensive use of `#[instrument]` for tracing

### Database Access
- Database modules in `src/db/` (app.rs, build.rs, remote.rs, etc.)
- Uses SQLx with compile-time query verification
- Models in `src/models/` define domain types

### Testing
- Extensive integration tests in `src/podman.rs` (`#[cfg(test)] mod tests`)
- Tests create real containers using Podman
- Test helpers for container cleanup and state verification
- Tests assume Podman is installed and running

## Configuration

**Client config:** `~/.config/litehouse/client-config.toml`
- `base_url` - Server API endpoint

**Server config:** Loaded via `ServerConfig::load()` (see `src/config.rs`)
- Host and ports for proxy, Caddy HTTP/HTTPS

**Environment variables:**
- `PODMAN_SSH_SOCK`, `PODMAN_SOCK`, `CONTAINER_HOST` - Override socket path
- `LITEHOUSE_LOCAL_DEV` or `RUST_LOG` - Enable local dev mode
- `DATABASE_URL` - SQLite database path (default: `~/.local/share/litehouse/litehouse.db`)

## Known Issues & TODOs

From NOTES.md:
- Restart command not implemented (see `src/cli.rs:153`)
- Deploy endpoint receives binary but doesn't process it (`src/api.rs:243`)
- Caddy integration needs testing with actual subdomain routing
- State synchronization between DB, Podman, and in-memory is not unified
- No monitoring, backup, or litestream integration yet
- GitHub webhook support planned but not implemented

## Development Context

**Branch:** `podman` (refactoring branch, not merged to main yet)

**Target use case:** Self-hosted Next.js apps with subdomain routing, similar to Vercel's deployment model

**Original concept:** Deploy statically-linked binaries, now pivoting to container-based deployment for broader adoption (Next.js, etc.)
