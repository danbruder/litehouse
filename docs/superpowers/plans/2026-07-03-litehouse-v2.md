# Litehouse v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform litehouse into a minimal run-only platform: GitHub Actions builds → GHCR → authenticated deploy hook → server pulls & runs; daily SQLite backups to S3; single-token auth; Askama+HTMX admin UI; provable end-to-end on a fresh DigitalOcean node at `*.s.danbruder.com`.

**Architecture:** The server never builds. `lh create` commits a workflow + sets a secret in the app's GitHub repo; `git push` builds an image on GitHub, pushes to GHCR, and POSTs the server's deploy hook; the server pulls the image, recreates the container, and syncs Caddy. Backups snapshot SQLite volumes via one-shot containers into a shared volume, then upload to S3. Disaster recovery = state DB from S3 + images from GHCR + volumes from S3.

**Tech Stack:** Rust (axum 0.6, bollard 0.15, sqlx 0.7/SQLite, clap 4), Caddy container, GitHub Actions + GHCR, aws-sdk-s3, Askama + HTMX, `crypto_box` for GitHub secret sealing.

**Spec:** `docs/superpowers/specs/2026-07-03-litehouse-v2-design.md` (including addenda: single admin token, no Elm/SSE/message-bus/reconciler, agent-friendly CLI).

---

## Conventions for every task

- Work on branch `agents` (current branch). One commit per task minimum; use the step-level commits below.
- After ANY migration or query change: `DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" cargo sqlx database create && cargo sqlx migrate run && cargo sqlx prepare` (create `.dev/` if missing, it's gitignored — add to `.gitignore` in Task 1). Commit the updated `.sqlx/` directory.
- Verification baseline: `cargo build 2>&1 | tail -5` must end with `Finished` (warnings OK, zero errors) and `cargo test 2>&1 | tail -5` must report `test result: ok` for every suite that runs. Docker-dependent tests require Docker running locally; if a test fails because Docker is unavailable, note it, do not delete the test.
- **Never leave a task with a broken build.** Deletion tasks compile-check between steps.
- Commit messages: conventional (`feat:`, `refactor:`, `chore:`, `test:`).
- **Testing policy:** prefer Rust unit tests for pure logic, Rust integration tests against real Docker (Rancher Desktop is running locally) for container behavior, e2e only at Task 15. NO manual verification steps anywhere — every claim of "works" must be backed by a command and its output.
- The hook handler's `internal` helper = `fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) { (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()) }` — define it next to the handler.

---

## Phase 0 — Baseline

### Task 0: Record the baseline

**Files:** none modified.

- [ ] **Step 1:** Run `cargo build 2>&1 | tail -3` — expect `Finished`. Run `cargo test 2>&1 | grep -E '^test result' ` — record pass counts in the task notes.
- [ ] **Step 2:** Run `find src -name '*.rs' | xargs wc -l | tail -1` and record total LOC (~20,200). This number should drop substantially by Phase 2.
- [ ] **Step 3:** `git checkout -b v2 || git checkout v2` — do all work on a `v2` branch off `agents`. Commit nothing yet.

---

## Phase 1 — Demolition

Order matters: each task deletes one subsystem and leaves the build green.

### Task 1: Delete Litestream

**Files:**
- Delete: `src/litestream.rs`
- Modify: `src/lib.rs` (remove `pub mod litestream;`)
- Modify: `src/reconciler.rs` (remove litestream phases — the whole module dies in Task 2, so just stub the calls out)
- Modify: `src/api.rs:1181` and `src/api.rs:1237` (remove `litestream::sync_configuration` calls after S3 config set/delete)
- Modify: `src/commands/start.rs:89,146` (remove `litestream::` calls)
- Modify: `src/commands/server.rs:38` (remove `restore_all_databases_if_needed` on boot)
- Modify: `src/commands/delete.rs:97` (remove litestream call)
- Modify: `src/install/phases.rs` + `src/install/templates.rs` (remove phase 6c "pull litestream image" and any litestream container start / config generation)
- Modify: `src/commands/install.rs` (remove phase 6c from the phase list)
- Modify: `.gitignore` (add `.dev/`)

- [ ] **Step 1:** `grep -rn "litestream" src/ install.sh | grep -v '^Binary'` — enumerate every call site (expect the list above; if more appear, handle them the same way: delete the call, keep surrounding logic).
- [ ] **Step 2:** Delete `src/litestream.rs`, remove the module from `src/lib.rs`, and remove every call site. Where a call site's *only* purpose was litestream (e.g. an `if let Some(s3_config)` block that only synced litestream), delete the whole block. Where litestream was one step among others (e.g. `commands/start.rs` starts the container then synced litestream), keep the other steps.
- [ ] **Step 3:** `cargo build 2>&1 | tail -3` — expect `Finished`.
- [ ] **Step 4:** `cargo test 2>&1 | grep -E '^test result'` — all suites `ok`.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "refactor: remove litestream — v2 uses daily S3 backups"`

### Task 2: Delete reconciler, message_bus, and sse

**Files:**
- Delete: `src/reconciler.rs`, `src/message_bus/` (whole dir), `src/sse/` (whole dir)
- Modify: `src/lib.rs` (remove the three `pub mod` lines)
- Modify: `src/commands/server.rs` (remove Reconciler init + background loop at lines ~48–64, MessageBus construction, 15s heartbeat task; remove `message_bus` and `log_streaming_tasks` from `AppState`)
- Modify: `src/api.rs` (remove routes `GET /events/stream`, `GET /github/connect/stream`; remove `message_bus` usages in handlers — the logs handler must stream directly from `docker::logs_stream` as a chunked axum body instead of via the bus)
- Modify: `src/config.rs` (remove `reconcile_interval_secs` from `ServerConfig`)
- Modify: `src/webhook/handler.rs` (drop `message_bus` parameter — this module dies in Task 3, minimal touch to keep compiling)

- [ ] **Step 1:** `grep -rn "message_bus\|MessageBus\|reconciler\|Reconciler\|sse::" src/` — enumerate call sites.
- [ ] **Step 2:** Delete the modules and fix every call site. Replace the reconciler's boot-time role with one function `commands::server::sync_on_boot(docker, pool, config)` that: (a) ensures Caddy is running (`caddy::start`), (b) calls `caddy::sync_configuration`. Container liveness is Docker's job (`--restart unless-stopped`).
- [ ] **Step 3:** For `GET /apps/:name/logs`: keep the endpoint. Non-follow mode returns the last N lines as text. Follow mode returns `axum::body::StreamBody` wrapping `docker::logs_stream`. No task registry needed.
- [ ] **Step 4:** `cargo build 2>&1 | tail -3` → `Finished`; `cargo test 2>&1 | grep -E '^test result'` → ok (message_bus/sse test modules disappear with their code).
- [ ] **Step 5:** Commit: `refactor: remove reconciler, message bus, and SSE — docker restart policies + direct log streaming`

### Task 3: Delete server-side builds, git, remotes, and the GitHub webhook path

**Files:**
- Delete: `src/git.rs`, `src/webhook/` (whole dir), `src/commands/remote/` (whole dir), `src/commands/build.rs`, `src/commands/github/` (server connect commands die; CLI github login is rebuilt client-side in Task 10)
- Modify: `src/lib.rs`, `src/commands/mod.rs` (remove modules)
- Modify: `src/cli.rs` (remove `Commands::{Build, Remote, Github}` variants, `RemoteCmd`, `GithubCmd`, and their dispatch arms; keep `Deploy` for now — it's rewired in Task 7)
- Modify: `src/api.rs` (remove routes: `POST /webhooks/github`, `POST|DELETE /apps/:name/remote`, `POST /apps/:name/build`, `GET /apps/:name/builds`, `GET /apps/:name/builds/:build_id/logs`, `GET /apps/:name/webhook`, `GET /apps/:name/webhook/deliveries`, all `/github/*` routes; remove their handlers)
- Modify: `src/api_client.rs` (remove `remote_add`, `remote_remove`, `build`, `github_*` methods and their types)
- Modify: `src/docker.rs` (remove `build`, `build_with_log`, `save_image`, `load_image` and the buildkit code path; keep `get_exposed_port`, `run`, `stop`, `remove`, `logs*`, `image_exists`, `connect`)
- Modify: `Cargo.toml` (drop the `buildkit` feature from bollard and the `default` feature list if nothing else needs it)
- Modify: `src/commands/create.rs` (remove any remote/clone logic; `create` becomes: insert app row + create volume)

- [ ] **Step 1:** `grep -rn "commands::build\|git::\|webhook::\|remote" src/ | grep -v deploy` — enumerate.
- [ ] **Step 2:** Delete modules, routes, CLI variants, and client methods listed above. `db/remote.rs`, `db/build.rs`, `db/webhook.rs`, `db/github_connection.rs` and their models keep compiling until the schema migration in Task 5 — leave them for now, just remove *callers*.
- [ ] **Step 3:** `cargo build 2>&1 | tail -3` → `Finished`.
- [ ] **Step 4:** `cargo test 2>&1 | grep -E '^test result'` → ok (tests inside deleted modules go with them; fix any orphaned test that referenced them).
- [ ] **Step 5:** Commit: `refactor: remove server-side builds, git, remotes, and webhook build path`

### Task 4: Delete the Elm SPA

**Files:**
- Delete: `assets/` (entire directory), `compile-assets.sh`
- Modify: `src/admin_spa.rs` → shrink to a single placeholder route (real UI in Task 12)
- Modify: `Cargo.toml` (remove `rust-embed`, `mime_guess`)
- Modify: `.github/workflows/release.yml` (remove Node/Elm setup and `npm` steps)
- Modify: `dev-deploy.sh` (remove `./compile-assets.sh` line)

- [ ] **Step 1:** Replace `src/admin_spa.rs` contents with:

```rust
use axum::{response::Html, routing::get, Router};

pub fn create_admin_router() -> Router {
    Router::new().route("/", get(|| async { Html("<h1>litehouse</h1><p>UI coming in v2.</p>") }))
}
```

- [ ] **Step 2:** Delete `assets/`, `compile-assets.sh`; strip release.yml and dev-deploy.sh as above; remove the two crates from Cargo.toml.
- [ ] **Step 3:** `cargo build 2>&1 | tail -3` → `Finished`. `cargo test 2>&1 | grep -E '^test result'` → ok.
- [ ] **Step 4:** Commit: `refactor: remove Elm SPA — server-rendered UI arrives with v2 admin`

### Task 5: Single-token auth + v2 schema

Replaces JWT/users/orgs with one admin token, and migrates the schema.

**Files:**
- Delete: `src/auth/` (whole dir), `src/commands/auth/` (whole dir), `src/db/{user,organization,organization_member,refresh_token,github_connection,webhook,remote,build}.rs`, `src/models/{user,organization,organization_member,auth,github_connection,webhook_config,webhook_delivery,remote,build}.rs`
- Create: `src/auth.rs`
- Create: `migrations/20260703_v2_simplify.sql`
- Create: `src/models/deploy.rs`, `src/db/deploy.rs`
- Modify: `src/lib.rs`, `src/db/mod.rs`, `src/models/mod.rs`, `src/cli.rs` (remove `Auth` command; add `Connect`), `src/api.rs`, `src/api_client.rs`, `src/config.rs`, `src/models/app.rs`, `src/db/app.rs`, `src/commands/server.rs`

- [ ] **Step 1: Migration.** Create `migrations/20260703_v2_simplify.sql`:

```sql
-- v2: single-operator platform. Drop multi-user auth, builds, remotes, webhooks.
DROP TABLE IF EXISTS refresh_token;
DROP TABLE IF EXISTS organization_member;
DROP TABLE IF EXISTS organization;
DROP TABLE IF EXISTS user;
DROP TABLE IF EXISTS github_connection;
DROP TABLE IF EXISTS webhook_config;
DROP TABLE IF EXISTS webhook_delivery;
DROP TABLE IF EXISTS remote;
DROP TABLE IF EXISTS build;
DROP TABLE IF EXISTS state_change;

ALTER TABLE app ADD COLUMN repo TEXT;              -- "owner/name"
ALTER TABLE app ADD COLUMN image TEXT;             -- last deployed image ref
ALTER TABLE app ADD COLUMN exposed_port TEXT;      -- detected from image on deploy
ALTER TABLE app ADD COLUMN deploy_token_hash TEXT; -- sha256 hex of per-app deploy token

CREATE TABLE deploy (
    id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL REFERENCES app(id),
    image TEXT NOT NULL,
    git_sha TEXT,
    status TEXT NOT NULL DEFAULT 'in_progress',    -- in_progress | succeeded | failed
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_deploy_app ON deploy(app_id, created_at DESC);
```

(Note: `app.organization_id` column stays — SQLite can't drop columns cheaply; it's simply unused. `state_change` table and model die; remove `App::started()/running()` state-change return values accordingly.)

- [ ] **Step 2: New auth module.** Create `src/auth.rs`:

```rust
use axum::{
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api::AppState;

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Accepts `Authorization: Bearer <token>` or a `litehouse_token` cookie (for the UI).
pub async fn admin_auth_middleware<B>(
    State(state): State<Arc<RwLock<AppState>>>,
    req: Request<B>,
    next: Next<B>,
) -> Result<Response, StatusCode> {
    let expected = state.read().await.admin_token_hash.clone();
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(str::to_string);
    let cookie = req
        .headers()
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .map(str::trim)
                .find_map(|kv| kv.strip_prefix("litehouse_token="))
        })
        .map(str::to_string);
    let provided = bearer.or(cookie).ok_or(StatusCode::UNAUTHORIZED)?;
    if constant_time_eq(&hash_token(&provided), &expected) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_hex_sha256() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn generated_tokens_are_unique_64_hex() {
        let (a, b) = (generate_token(), generate_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
```

- [ ] **Step 3:** Run the auth tests before wiring: `cargo test auth:: 2>&1 | grep -E '^test result'` → ok.
- [ ] **Step 4: Wire it.** In `ServerConfig` add `admin_token_hash: Option<String>`. In `commands/server.rs`: put `admin_token_hash` into `AppState` (if `None` in config, generate a token, print it once to stdout with a warning, store its hash in AppState). In `api.rs`: delete all `/auth/*` routes and the old middleware import; every protected route now layers `crate::auth::admin_auth_middleware`. Remove `jwt_secret`, `github_client_id`, `webhook_url` from `AppState`; add `admin_token_hash: String` and `server_config: ServerConfig`.
- [ ] **Step 5: Deploy + App models.** Create `src/models/deploy.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Deploy {
    pub id: String,
    pub app_id: String,
    pub image: String,
    pub git_sha: Option<String>,
    pub status: String, // "in_progress" | "succeeded" | "failed"
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

Create `src/db/deploy.rs` with `insert(pool, &Deploy)`, `set_status(pool, id, status, error)`, `latest_for_app(pool, app_id) -> Option<Deploy>`, `list_for_app(pool, app_id, limit) -> Vec<Deploy>` — mirror the query style of `src/db/app.rs` (sqlx `query_as!`). Add `repo`, `image`, `exposed_port`, `deploy_token_hash` (`Option<String>` each) to `App` in `src/models/app.rs` and to every query in `src/db/app.rs`.

- [ ] **Step 6: CLI.** In `cli.rs`: delete `Auth` command tree; add:

```rust
/// Point this CLI at a server: lh connect https://admin.s.danbruder.com --token <TOKEN>
Connect {
    base_url: String,
    #[arg(long)]
    token: String,
},
```

Dispatch: write `ClientConfig { base_url: format!("{}/api", base_url.trim_end_matches('/')), api_token: Some(token), .. }` and save. In `config.rs` rename `ClientConfig.access_token` → `api_token`, drop `refresh_token`. In `api_client.rs` send `Authorization: Bearer {api_token}` on every request; delete login/register/refresh/logout methods.

- [ ] **Step 7:** Recreate the dev DB and sqlx cache: `rm -rf .dev && mkdir .dev && DATABASE_URL="sqlite:$PWD/.dev/litehouse.db" bash -c 'cargo sqlx database create && cargo sqlx migrate run && cargo sqlx prepare'`.
- [ ] **Step 8:** `cargo build 2>&1 | tail -3` → `Finished`. `cargo test 2>&1 | grep -E '^test result'` → ok. Also `find src -name '*.rs' | xargs wc -l | tail -1` — expect roughly half of baseline; record it.
- [ ] **Step 9:** Commit: `feat: single admin-token auth + v2 schema (deploy table, app repo/image columns)`

---

## Phase 2 — Deploy pipeline

### Task 6: `docker::pull` from GHCR

**Files:**
- Modify: `src/docker.rs`
- Modify: `src/models/system_config.rs` + `src/db/system_config.rs` (add `ghcr_token` get/set alongside the existing S3 config pattern)
- Modify: `src/api.rs`, `src/api_client.rs`, `src/cli.rs` (add `lh config ghcr set --token X` / `get`, mirroring the existing `Config S3` subcommand)

- [ ] **Step 1: Failing test** (in `src/docker.rs` tests module — integration, needs Docker):

```rust
#[tokio::test]
async fn test_pull_public_image() {
    let docker = connect().await.unwrap();
    // tiny public image; no auth needed
    pull(&docker, "alpine:3.20", None).await.unwrap();
    assert!(image_exists("alpine:3.20").await.unwrap());
}
```

Run: `cargo test test_pull_public_image 2>&1 | tail -3` → FAIL (`pull` not found).

- [ ] **Step 2: Implement** in `src/docker.rs`:

```rust
use bollard::auth::DockerCredentials;
use bollard::image::CreateImageOptions;

/// Pull an image. For ghcr.io private images pass a GitHub token with read:packages.
#[instrument(skip(docker, registry_token))]
pub async fn pull(docker: &Docker, image: &str, registry_token: Option<&str>) -> Result<()> {
    let credentials = registry_token.map(|token| DockerCredentials {
        username: Some("litehouse".to_string()), // GHCR accepts any username with a PAT
        password: Some(token.to_string()),
        ..Default::default()
    });
    let options = CreateImageOptions { from_image: image, ..Default::default() };
    let mut stream = docker.create_image(Some(options), None, credentials);
    while let Some(progress) = stream.next().await {
        progress.map_err(|e| anyhow::anyhow!("pull {image} failed: {e}"))?;
    }
    Ok(())
}
```

- [ ] **Step 3:** `cargo test test_pull_public_image 2>&1 | tail -3` → PASS.
- [ ] **Step 4:** Add `ghcr_token` to system config storage + `lh config ghcr` CLI, copying the existing S3Config pattern in `models/system_config.rs`, `db/system_config.rs`, `api.rs` (`POST|GET /config/ghcr`, redact on GET), `api_client.rs`, `cli.rs`.
- [ ] **Step 5:** `cargo build && cargo test 2>&1 | grep -E '^test result'` → ok. Commit: `feat: docker pull with GHCR auth + ghcr token config`

### Task 7: Deploy engine + deploy hook

**Files:**
- Create: `src/deploy.rs`
- Modify: `src/lib.rs` (add module), `src/api.rs` (hook route + deploys routes), `src/api_client.rs`, `src/cli.rs` (rewire `lh deploy`, add `lh deploys`), delete `Deploy{image_path,..}` tarball-upload variant and `POST /apps/:name/deploy` multipart handler

- [ ] **Step 1:** Create `src/deploy.rs`:

```rust
use anyhow::{Context, Result};
use bollard::Docker;
use sqlx::{Pool, Sqlite};

use crate::config::ServerConfig;
use crate::models::deploy::Deploy;
use crate::{caddy, db, docker};

/// Pull `image`, recreate the app container, sync Caddy, record the deploy.
/// The old container keeps running until the new image is pulled successfully.
pub async fn deploy_app(
    pool: &Pool<Sqlite>,
    docker_conn: &Docker,
    config: &ServerConfig,
    app_name: &str,
    image: &str,
    git_sha: Option<&str>,
) -> Result<Deploy> {
    let app = db::app::get_by_name(pool, app_name)
        .await?
        .context(format!("unknown app '{app_name}'"))?;

    let mut record = Deploy::new(&app.id, image, git_sha);
    db::deploy::insert(pool, &record).await?;

    let result = do_deploy(pool, docker_conn, config, &app, image).await;
    match &result {
        Ok(()) => {
            db::deploy::set_status(pool, &record.id, "succeeded", None).await?;
            record.status = "succeeded".into();
        }
        Err(e) => {
            db::deploy::set_status(pool, &record.id, "failed", Some(&format!("{e:#}"))).await?;
            record.status = "failed".into();
            record.error = Some(format!("{e:#}"));
        }
    }
    result.map(|_| record)
}

async fn do_deploy(
    pool: &Pool<Sqlite>,
    docker_conn: &Docker,
    config: &ServerConfig,
    app: &crate::models::app::App,
    image: &str,
) -> Result<()> {
    let ghcr_token = db::system_config::get_ghcr_token(pool).await?;
    docker::pull(docker_conn, image, ghcr_token.as_deref()).await?;
    let exposed_port = docker::get_exposed_port(image).await?;

    // Point of no return: replace the container.
    docker::stop_and_remove_container(docker_conn, &app.name).await.ok(); // absent on first deploy
    let env_vars = db::env_var::get_all_for_app(pool, &app.id).await?;
    docker::run(docker_conn, &app.name, image, &env_vars).await?;

    db::app::set_deployed(pool, &app.id, image, &exposed_port).await?; // updates image, exposed_port, state=running
    caddy::sync_configuration(docker_conn, pool).await?;
    Ok(())
}
```

Add `Deploy::new(app_id, image, git_sha)` to `src/models/deploy.rs` (uuid v4 id, `chrono::Utc::now().to_rfc3339()` timestamps, status `in_progress`). Add `db::app::set_deployed` to `src/db/app.rs`. Adapt `docker::run`'s existing signature (check it at implementation time — it currently derives the container name and volume itself; keep that logic, add a `stop_and_remove_container(docker, app_name)` helper wrapping the existing stop + `remove_container` bollard calls).

- [ ] **Step 2: Hook route.** In `api.rs` add a **public** route `POST /hooks/deploy` (outside the admin middleware) with per-app token auth:

```rust
#[derive(serde::Deserialize)]
struct DeployHookPayload {
    app: String,
    image: String,
    sha: Option<String>,
}

async fn deploy_hook_handler(
    State(state): State<Arc<RwLock<AppState>>>,
    headers: HeaderMap,
    Json(payload): Json<DeployHookPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (pool, docker, config) = {
        let s = state.read().await;
        (s.db_pool.clone(), s.docker.clone(), s.server_config.clone())
    };
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".into()))?;
    let app = db::app::get_by_name(&pool, &payload.app)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("unknown app '{}'", payload.app)))?;
    let expected = app.deploy_token_hash.clone()
        .ok_or((StatusCode::UNAUTHORIZED, "app has no deploy token".into()))?;
    if crate::auth::hash_token(token) != expected {
        return Err((StatusCode::UNAUTHORIZED, "bad deploy token".into()));
    }
    let deploy = crate::deploy::deploy_app(&pool, &docker, &config, &payload.app, &payload.image, payload.sha.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{e:#}")))?; // error text lands in the Actions log
    Ok(Json(serde_json::json!({ "status": deploy.status, "deploy_id": deploy.id })))
}
```

Also add admin routes `GET /apps/:name/deploys` (list) and keep `POST /apps/:name/deploy` as an admin-triggered redeploy taking `{image}` JSON (replaces the multipart tarball upload — delete that handler and `ApiClient::deploy_app`'s multipart body).

- [ ] **Step 3: CLI.** `lh deploy <app> --image ghcr.io/...` → calls admin deploy route. `lh deploys <app> [--json] [--wait]`: lists deploys; `--wait` polls `GET /apps/:name/deploys` every 3s until the newest deploy is not `in_progress`, then exits 0 on `succeeded` / 1 on `failed` printing the error. This is the agent verification primitive.
- [ ] **Step 4: Tests.** Unit-test the hook's auth decision by extracting it: `fn verify_deploy_token(provided: &str, stored_hash: Option<&str>) -> bool` in `src/deploy.rs` with tests (correct token → true; wrong token → false; no hash → false). Run `cargo test verify_deploy_token 2>&1 | tail -3` → PASS after implementing.
- [ ] **Step 5:** `cargo sqlx prepare` (new queries), `cargo build`, `cargo test` → green. Commit: `feat: deploy engine + authenticated deploy hook + lh deploys --wait`

### Task 8: Caddy cleanup

**Files:**
- Modify: `src/caddy.rs`

- [ ] **Step 1:** In `build_caddy_config` (caddy.rs:642): remove the hardcoded `{app}.lh.danbruder.com` fallback (caddy.rs:721) — if `ServerConfig.domain` is `None` and not local dev, return an error (`anyhow::bail!("server domain not configured — run lh install --domain <domain>")`). Route apps from `app.exposed_port` (set by deploys) instead of the deleted `build` table; skip apps with `exposed_port = NULL`.
- [ ] **Step 2:** Existing caddy config-generation tests: update fixtures to the new source of port data; add a test that an app with `exposed_port: Some("8080")` and domain `s.danbruder.com` yields a route matching host `myapp.s.danbruder.com` → `myapp-container:8080`.
- [ ] **Step 3:** `cargo sqlx prepare && cargo build && cargo test 2>&1 | grep -E '^test result'` → ok. Commit: `refactor: caddy routes from app.exposed_port, no hardcoded domain fallback`

---

## Phase 3 — Drunk-proof `lh create`

### Task 9: GitHub contents + secrets API

**Files:**
- Create: `src/github/actions.rs`
- Modify: `src/github/mod.rs`, `Cargo.toml` (add `crypto_box = "0.9"`, `base64 = "0.22"`)

- [ ] **Step 1: Failing test** for secret sealing (pure function, no network):

```rust
#[test]
fn seal_secret_roundtrip() {
    use crypto_box::{aead::OsRng, PublicKey, SecretKey};
    let sk = SecretKey::generate(&mut OsRng);
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode(sk.public_key().as_bytes());
    let sealed = seal_secret_for_github(&pk_b64, "hunter2").unwrap();
    let sealed_bytes = base64::engine::general_purpose::STANDARD.decode(sealed).unwrap();
    let opened = crypto_box::seal_open(&sk, &sealed_bytes).unwrap();
    assert_eq!(opened, b"hunter2");
}
```

Run `cargo test seal_secret_roundtrip 2>&1 | tail -3` → FAIL (function missing).

- [ ] **Step 2: Implement** `src/github/actions.rs`:

```rust
use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::json;

const API: &str = "https://api.github.com";

fn client(token: &str) -> reqwest::Client { /* reqwest client with auth + user-agent headers, copy pattern from github/client.rs */ }

/// GitHub sealed-box encryption for Actions secrets.
pub fn seal_secret_for_github(repo_public_key_b64: &str, secret: &str) -> Result<String> {
    use crypto_box::{aead::OsRng, PublicKey};
    let key_bytes: [u8; 32] = B64.decode(repo_public_key_b64)?.try_into()
        .map_err(|_| anyhow::anyhow!("bad public key length"))?;
    let pk = PublicKey::from(key_bytes);
    let sealed = crypto_box::seal(&mut OsRng, &pk, secret.as_bytes())
        .map_err(|e| anyhow::anyhow!("seal failed: {e}"))?;
    Ok(B64.encode(sealed))
}

/// PUT /repos/{owner}/{repo}/actions/secrets/{name}
pub async fn put_actions_secret(token: &str, owner: &str, repo: &str, name: &str, value: &str) -> Result<()> {
    let key: serde_json::Value = client(token)
        .get(format!("{API}/repos/{owner}/{repo}/actions/secrets/public-key"))
        .send().await?.error_for_status()?.json().await?;
    let sealed = seal_secret_for_github(key["key"].as_str().context("no key")?, value)?;
    client(token)
        .put(format!("{API}/repos/{owner}/{repo}/actions/secrets/{name}"))
        .json(&json!({ "encrypted_value": sealed, "key_id": key["key_id"] }))
        .send().await?.error_for_status()?;
    Ok(())
}

/// Create or update a file via PUT /repos/{owner}/{repo}/contents/{path} (fetches existing sha if present).
pub async fn put_file(token: &str, owner: &str, repo: &str, path: &str, content: &str, message: &str) -> Result<()> {
    let existing = client(token)
        .get(format!("{API}/repos/{owner}/{repo}/contents/{path}"))
        .send().await?;
    let sha = if existing.status().is_success() {
        existing.json::<serde_json::Value>().await?["sha"].as_str().map(String::from)
    } else { None };
    let mut body = json!({ "message": message, "content": B64.encode(content) });
    if let Some(sha) = sha { body["sha"] = json!(sha); }
    client(token)
        .put(format!("{API}/repos/{owner}/{repo}/contents/{path}"))
        .json(&body)
        .send().await?.error_for_status()?;
    Ok(())
}
```

- [ ] **Step 3:** `cargo test seal_secret_roundtrip 2>&1 | tail -3` → PASS.
- [ ] **Step 4:** Commit: `feat: github actions API — sealed secrets + contents write`

### Task 10: Workflow template + `lh create`

**Files:**
- Create: `src/workflow.rs`
- Modify: `src/commands/create.rs`, `src/cli.rs`, `src/api.rs` (create endpoint returns a freshly minted deploy token), `src/api_client.rs`, `src/lib.rs`
- Create: `src/commands/github_login.rs` (client-side device flow → saves token to client config; `GITHUB_TOKEN` env and `gh auth token` take precedence)

- [ ] **Step 1: Failing golden test** in `src/workflow.rs`:

```rust
#[test]
fn workflow_renders_owner_app_and_hook() {
    let yml = render_deploy_workflow("danbruder", "hello", "https://admin.s.danbruder.com/api/hooks/deploy");
    assert!(yml.contains("ghcr.io/danbruder/hello:${{ github.sha }}"));
    assert!(yml.contains("https://admin.s.danbruder.com/api/hooks/deploy"));
    assert!(yml.contains("secrets.LITEHOUSE_DEPLOY_TOKEN"));
    assert!(!yml.contains('\t'));
}
```

- [ ] **Step 2: Implement** `render_deploy_workflow(owner, app, hook_url) -> String` returning:

```yaml
name: litehouse deploy
on:
  push:
    branches: [main, master]
jobs:
  deploy:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/build-push-action@v6
        with:
          context: .
          push: true
          tags: |
            ghcr.io/{owner}/{app}:latest
            ghcr.io/{owner}/{app}:${{ github.sha }}
      - name: notify litehouse
        run: |
          curl -fsS -X POST "{hook_url}" \
            -H "Authorization: Bearer ${{ secrets.LITEHOUSE_DEPLOY_TOKEN }}" \
            -H "Content-Type: application/json" \
            -d "{\"app\":\"{app}\",\"image\":\"ghcr.io/{owner}/{app}:${{ github.sha }}\",\"sha\":\"${{ github.sha }}\"}"
```

(`{owner}`/`{app}`/`{hook_url}` are format substitutions; the `${{ }}` stay literal.) Run test → PASS.

- [ ] **Step 3: Server side.** `POST /apps` (create) gains optional `repo` field; response gains `deploy_token` (plaintext, returned exactly once) and `url`. Server mints token via `auth::generate_token()`, stores `hash_token(&token)` on the app row. If the app exists and the caller passes `--rotate-token`, mint a new one (idempotent create).
- [ ] **Step 4: Client side.** `lh create <app> [--repo owner/name] [--json]`:
  1. Resolve GitHub token: `$GITHUB_TOKEN` → `gh auth token` (shell out, ignore failure) → device flow via `commands/github_login.rs` → error listing the three options (non-interactive first — agent-friendly).
  2. If `--repo` omitted and CWD is a git repo with a GitHub origin, infer `owner/name` from `git remote get-url origin`.
  3. Call create API → get `deploy_token` + `url` + hook URL (`{base_url}/hooks/deploy`).
  4. `github::actions::put_actions_secret(token, owner, name, "LITEHOUSE_DEPLOY_TOKEN", &deploy_token)`.
  5. `github::actions::put_file(token, owner, name, ".github/workflows/litehouse-deploy.yml", &render_deploy_workflow(...), "Add litehouse deploy workflow")`.
  6. Print (or `--json` emit): app name, URL, repo, "push to main to deploy".
- [ ] **Step 5:** `cargo sqlx prepare && cargo build && cargo test` → green. Commit: `feat: drunk-proof lh create — workflow + secret + token in one command`

---

## Phase 4 — Backups & restore

### Task 11: Backup engine

**Files:**
- Create: `src/backup.rs`
- Modify: `Cargo.toml` (add `aws-sdk-s3 = "1"`, `aws-config = { version = "1", features = ["behavior-version-latest"] }`, `aws-credential-types = "1"`)
- Modify: `src/lib.rs`, `src/volume.rs` (reuse `get_app_volume_name`), `src/install/templates.rs` (litehouse-server container gains `-v litehouse_backups:/opt/litehouse/backups`; also pull `keinos/sqlite3:latest` during install — Task 13 covers install changes, just note it there)

Design: for each app, run a one-shot `keinos/sqlite3` container with binds `[{app_volume}:/data:ro, litehouse_backups:/backup]` executing:

```sh
set -e
rm -rf "/backup/{app}" && mkdir -p "/backup/{app}/dbs"
cd /data
find . -name '*.db' -o -name '*.sqlite' -o -name '*.sqlite3' | while read -r f; do
  mkdir -p "/backup/{app}/dbs/$(dirname "$f")"
  sqlite3 "file:$f?mode=ro" "VACUUM INTO '/backup/{app}/dbs/$f'"
done
tar czf "/backup/{app}/files.tar.gz" --exclude='*.db' --exclude='*.sqlite' --exclude='*.sqlite3' --exclude='*-wal' --exclude='*-shm' .
```

The server (which mounts `litehouse_backups` at `/opt/litehouse/backups`) then tars `/opt/litehouse/backups/{app}` → uploads `s3://{bucket}/{prefix}/apps/{app}/{YYYY-MM-DD}.tar.gz`. The server's own DB: `sqlx::query("VACUUM INTO ?1")` into the backups dir → upload `s3://{bucket}/{prefix}/litehouse/{YYYY-MM-DD}.db`. Retention: after upload, list the prefix, keep newest 14, delete the rest.

- [ ] **Step 1: Failing tests** (pure logic first) in `src/backup.rs`:

```rust
#[test]
fn s3_key_layout() {
    assert_eq!(app_backup_key(Some("prod"), "hello", "2026-07-03"), "prod/apps/hello/2026-07-03.tar.gz");
    assert_eq!(app_backup_key(None, "hello", "2026-07-03"), "apps/hello/2026-07-03.tar.gz");
    assert_eq!(state_backup_key(None, "2026-07-03"), "litehouse/2026-07-03.db");
}

#[test]
fn retention_keeps_newest_14() {
    let keys: Vec<String> = (1..=20).map(|d| format!("apps/hello/2026-06-{d:02}.tar.gz")).collect();
    let doomed = keys_to_prune(&keys, 14);
    assert_eq!(doomed.len(), 6);
    assert!(doomed.contains(&"apps/hello/2026-06-01.tar.gz".to_string()));
    assert!(!doomed.contains(&"apps/hello/2026-06-20.tar.gz".to_string()));
}
```

Run → FAIL; implement `app_backup_key`, `state_backup_key`, `keys_to_prune` (sort keys lexically — dates are ISO so lexical = chronological); run → PASS.

- [ ] **Step 2: S3 client.** `fn s3_client(cfg: &S3Config) -> aws_sdk_s3::Client`:

```rust
pub fn s3_client(cfg: &crate::models::system_config::S3Config) -> aws_sdk_s3::Client {
    let creds = aws_credential_types::Credentials::new(
        cfg.access_key_id.clone(), cfg.secret_access_key.clone(), None, None, "litehouse");
    let mut builder = aws_sdk_s3::config::Builder::new()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new(cfg.region.clone()))
        .credentials_provider(creds)
        .force_path_style(true);
    if let Some(endpoint) = &cfg.endpoint {
        builder = builder.endpoint_url(endpoint.clone());
    }
    aws_sdk_s3::Client::from_conf(builder.build())
}
```

- [ ] **Step 3: Orchestration.** `pub async fn run_backup(pool, docker, ) -> Result<BackupReport>`: loads S3Config (error if unset), snapshots litehouse.db via `VACUUM INTO`, iterates apps, runs the one-shot snapshot container per app (use bollard: create container with `HostConfig{ binds }`, start, `wait_container`, inspect exit code, remove), tars each `/opt/litehouse/backups/{app}` with the existing `tar`+`flate2` crates, uploads via `put_object`, prunes. Record per-app success/failure in `BackupReport { succeeded: Vec<String>, failed: Vec<(String, String)>, ran_at: String }`; persist the report JSON in `system_config` under key `last_backup_report`.
- [ ] **Step 4: Integration test (MinIO)** — `#[ignore]`-gated test `test_backup_roundtrip_minio`: start `minio/minio` container on port 9000 (test helper mirrors docker.rs test helpers), point S3Config at it, create a volume with a sqlite db containing one row, `run_backup`, assert object exists via `list_objects_v2`, download and verify the tarball contains `dbs/./app.db` and that db opens with the row intact. Run: `cargo test test_backup_roundtrip_minio -- --ignored` → PASS.
- [ ] **Step 5:** Commit: `feat: daily backup engine — VACUUM INTO snapshots + S3 upload + retention`

### Task 12: Backup scheduling, API, CLI, restore

**Files:**
- Modify: `src/commands/server.rs` (scheduler task), `src/api.rs` (`POST /backups/run`, `GET /backups/status`, `POST /restore`), `src/api_client.rs`, `src/cli.rs` (`lh backup run`, `lh backup status [--json]`, `lh restore`)
- Modify: `src/backup.rs` (add `restore_all`)

- [ ] **Step 1: Scheduler.** In `commands/server.rs` spawn:

```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        interval.tick().await;
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let last = db::system_config::get_value(&pool, "last_backup_date").await.ok().flatten();
        if last.as_deref() != Some(today.as_str()) {
            match backup::run_backup(&pool, &docker).await {
                Ok(report) => {
                    let _ = db::system_config::set_value(&pool, "last_backup_date", &today).await;
                    tracing::info!(?report, "daily backup complete");
                }
                Err(e) => tracing::error!("daily backup failed: {e:#}"),
            }
        }
    }
});
```

A failed backup retries next hour (date key only set on success). A missed backup never blocks deploys.

- [ ] **Step 2: Restore.** `backup::restore_all(pool, docker, config)`:
  1. Load S3Config; list `{prefix}/litehouse/`, download newest `.db` snapshot to a temp file.
  2. Open it read-only via a second sqlx pool; copy `app`, `env_var`, and `system_config` rows (excluding s3/ghcr keys already set, and `last_backup_*`) into the live DB with `INSERT OR IGNORE`.
  3. For each restored app with an `image`: `docker::pull`, `volume::create_app_volume`, download newest `{prefix}/apps/{app}/*.tar.gz`, extract into the volume via one-shot `alpine:3.20` container with binds `[{vol}:/data, litehouse_backups:/backup]` running `sh -c "cd /data && tar xzf /backup/restore/{app}/files.tar.gz && cp -r /backup/restore/{app}/dbs/. /data/"`, then `docker::run` and `caddy::sync_configuration`.
  4. Return a report (restored apps, skipped apps with reasons).
- [ ] **Step 3: API + CLI.** Routes as listed; `lh restore` prints the report; `lh backup status --json` emits `last_backup_report`. Unit-test the row-copy SQL against two in-memory pools (source with 2 apps → target empty → copy → target has 2 apps; run twice → still 2, idempotent).
- [ ] **Step 4:** `cargo sqlx prepare && cargo build && cargo test` → green. Commit: `feat: backup scheduler, restore-from-S3, backup/restore CLI`

---

## Phase 5 — Install & release

### Task 13: Server image from GHCR + install rework

**Files:**
- Modify: `.github/workflows/release.yml` (add job: build + push `ghcr.io/danbruder/litehouse:{version,latest}` using the musl binary; reuse the existing `Dockerfile`, updating it to `FROM alpine:3.20`, `COPY lh /usr/local/bin/lh`, `EXPOSE 3030`, `CMD ["lh","serve"]`)
- Modify: `src/install/phases.rs`, `src/install/templates.rs`: delete phase 6a (on-box image build); replace with `docker pull ghcr.io/danbruder/litehouse:latest` (tag pinned to the running binary's version when available); phase 6b keeps caddy pull; add pull of `keinos/sqlite3:latest` and `alpine:3.20`
- Modify: `src/install/templates.rs` litehouse-server run script: image `ghcr.io/danbruder/litehouse:{version}`, add `-v litehouse_backups:/opt/litehouse/backups`, remove litestream env plumbing
- Modify: `src/commands/install.rs` + `src/cli.rs`: add `--ghcr-token` flag (stored into system_config after boot, used for private app images; optional — public images need none); phase 7 generates the admin token, writes `admin_token_hash` into `server-config.toml`, and prints at the end:

```
✓ litehouse installed
  admin UI:  https://admin.{domain}
  connect:   lh connect https://admin.{domain} --token {token}
```

- Modify: `install.sh` (drop buildx setup; pass through `--ghcr-token`)
- Modify: `src/config.rs:454` (remove the hardcoded macOS `/Users/dan/Desktop/litehouse-data`; use `$LITEHOUSE_DIR` else `dirs`-style `~/.local/share/litehouse` on macOS, `/opt/litehouse` on Linux)

- [ ] **Step 1:** Update `Dockerfile`, release.yml (docker/login-action + docker/build-push-action, `permissions: packages: write`), install phases/templates/flags as above.
- [ ] **Step 2:** Unit-test the generated install scripts where the codebase already tests them (`install/executor.rs` has tests — extend the pattern): assert litehouse-server script contains `ghcr.io/danbruder/litehouse`, `litehouse_backups:/opt/litehouse/backups`, and does NOT contain `litestream` or `docker build`.
- [ ] **Step 3:** `cargo build && cargo test` → green. Commit: `feat: install pulls server image from GHCR, generates admin token, mounts backups volume`
- [ ] **Step 4:** Push branch, tag a prerelease (`v0.2.0-alpha.1` via `./release.sh` adapted or manual tag), confirm the GitHub Actions run publishes `ghcr.io/danbruder/litehouse:0.2.0-alpha.1`. Run: `docker pull ghcr.io/danbruder/litehouse:0.2.0-alpha.1` locally → succeeds. (Requires the package to be public or the token configured — make the GHCR package public in repo settings.)

---

## Phase 6 — Admin UI

### Task 14: Askama + HTMX admin

**Files:**
- Modify: `Cargo.toml` (add `askama = "0.12"`, `askama_axum = "0.4"`)
- Create: `templates/base.html`, `templates/login.html`, `templates/apps.html`, `templates/app_detail.html`
- Create: `src/ui.rs`
- Modify: `src/lib.rs`, `src/commands/server.rs` (mount UI router at `/` replacing the Task 4 placeholder; delete `src/admin_spa.rs`)

Scope (keep it small — the CLI is the primary interface):
- `GET /login` — token form; POST sets `litehouse_token` cookie (HttpOnly, Secure, SameSite=Lax) and redirects to `/`.
- `GET /` — table of apps: name, state (live from Docker), URL link, last deploy (status + sha + age), last backup status. HTMX `hx-post` buttons for start/stop/redeploy hitting the existing JSON API (cookie auth already works via Task 5 middleware).
- `GET /apps/:name` — deploy history (last 20), env var list (names only + values behind a reveal), log tail (`<pre>` refreshed by `hx-get` every 5s from the logs endpoint).
- All UI routes except `/login` behind the same `admin_auth_middleware` (redirect to `/login` on 401 for HTML requests — add an `Accept: text/html` check to the middleware error path).

- [ ] **Step 1:** Templates + `src/ui.rs` router. HTMX vendored: download `htmx.min.js` into `templates/` is wrong — serve it from a `const HTMX_JS: &str = include_str!(...)`, checked into `src/ui/htmx.min.js` (single file, ~48KB, version 1.9.x). Tailwind via CDN is forbidden offline — use a small hand-written `styles.css` served the same way (`include_str!`), ~100 lines; do not add a Node toolchain back.
- [ ] **Step 2:** Test: askama templates compile at build time (that IS the test); plus one router test using `tower::ServiceExt::oneshot` asserting `GET /` without cookie → 303 to `/login`, and with a valid cookie → 200 containing `<table`.
- [ ] **Step 3:** Automated UI smoke: extend the Step 2 router tests to cover login POST (correct token → 303 + `set-cookie: litehouse_token=`; wrong token → 200 login page with error text) and `GET /apps/:name` for a seeded app → 200 containing the app name. No manual browser check.
- [ ] **Step 4:** `cargo build && cargo test` → green. Commit: `feat: askama+htmx admin UI — login, app list, app detail`

---

## Phase 7 — End-to-end acceptance

### Task 15: Example app + acceptance script

**Files:**
- Create: `examples/hello/Dockerfile`, `examples/hello/index.html`, `examples/hello/README.md`
- Create: `e2e/acceptance.sh`

- [ ] **Step 1:** Example app — `examples/hello/Dockerfile`:

```dockerfile
FROM busybox:1.36
WORKDIR /www
COPY index.html .
EXPOSE 8080
CMD ["httpd", "-f", "-p", "8080", "-h", "/www"]
```

`index.html`: `<h1>hello from litehouse</h1>`. README: "push this directory to a GitHub repo, then `lh create hello --repo you/repo`."

- [ ] **Step 2:** `e2e/acceptance.sh` (idempotent, verbose, exits nonzero on any failure):

```bash
#!/usr/bin/env bash
# End-to-end acceptance: fresh DO node -> installed server -> app live on a subdomain.
# Prereqs: wildcard DNS *.${DOMAIN} -> ${SERVER_IP}; gh CLI authed; docker + musl toolchain locally.
set -euo pipefail
: "${SERVER_IP:?e.g. 104.248.15.20}"
: "${DOMAIN:?e.g. s.danbruder.com}"
: "${HELLO_REPO:?e.g. danbruder/litehouse-hello — will be created if missing}"
S3_ARGS="${S3_ARGS:-}"   # e.g. --s3-access-key .. --s3-secret-key .. --s3-bucket .. --s3-region ..

echo "==> 1/7 build release binary"
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl

echo "==> 2/7 wipe node (containers + data)"
ssh "root@${SERVER_IP}" 'docker ps -aq | xargs -r docker rm -f; docker volume ls -q | xargs -r docker volume rm; rm -rf /opt/litehouse'

echo "==> 3/7 install"
scp target/x86_64-unknown-linux-musl/release/lh "root@${SERVER_IP}:/usr/local/bin/lh"
INSTALL_OUT=$(ssh "root@${SERVER_IP}" "lh install --domain ${DOMAIN} ${S3_ARGS}")
echo "$INSTALL_OUT"
TOKEN=$(echo "$INSTALL_OUT" | grep -oE -- '--token [a-f0-9]{64}' | awk '{print $2}')
[ -n "$TOKEN" ] || { echo "FATAL: no admin token in install output"; exit 1; }

echo "==> 4/7 connect CLI"
cargo run --quiet -- connect "https://admin.${DOMAIN}" --token "$TOKEN"
cargo run --quiet -- status   # smoke: authed API round-trip

echo "==> 5/7 ensure hello repo exists and is current"
if ! gh repo view "$HELLO_REPO" >/dev/null 2>&1; then gh repo create "$HELLO_REPO" --private -y; fi
TMP=$(mktemp -d); cp -r examples/hello/* "$TMP"; (cd "$TMP" && git init -qb main && git add -A \
  && git commit -qm "hello" && git remote add origin "https://github.com/${HELLO_REPO}.git" && git push -qf origin main)

echo "==> 6/7 create app (drunk-proof moment)"
cargo run --quiet -- create hello --repo "$HELLO_REPO"

echo "==> 7/7 trigger deploy and wait"
(cd "$TMP" && git commit -qm "deploy $(date +%s)" --allow-empty && git push -q origin main)
cargo run --quiet -- deploys hello --wait --timeout 600
for i in $(seq 1 30); do
  if curl -fsS "https://hello.${DOMAIN}" | grep -q "hello from litehouse"; then
    echo "ACCEPTANCE PASSED: https://hello.${DOMAIN} is live"; exit 0
  fi
  sleep 10
done
echo "FATAL: app never became reachable"; exit 1
```

(Note: `lh create` inside the script relies on `gh auth token` for GitHub — no device flow needed. Add `--timeout` support to `lh deploys --wait` if not present from Task 7.)

- [ ] **Step 3:** Commit: `feat: example app + e2e acceptance script`
- [ ] **Step 4: RUN IT.** `SERVER_IP=104.248.15.20 DOMAIN=s.danbruder.com HELLO_REPO=danbruder/litehouse-hello S3_ARGS="--s3-access-key … --s3-bucket …" ./e2e/acceptance.sh` — expect `ACCEPTANCE PASSED`. This step requires Dan's S3 credentials and confirms the wildcard DNS `*.s.danbruder.com` → node. Any failure here is a bug: use superpowers:systematic-debugging, fix, loop the fix back into the install code (per CLAUDE.md), and re-run until green.
- [ ] **Step 5: Backup + DR drill (the real acceptance of Phase 4).** After PASSED: `lh backup run`, verify `lh backup status --json` shows `hello` succeeded. Then re-run steps 2–4 of the script but instead of `lh create`, run `lh restore` after connecting — expect `hello` back online at `https://hello.${DOMAIN}` *without* touching GitHub. Document the drill result in the task notes.

### Task 16: Docs sweep

**Files:**
- Modify: `CLAUDE.md`, `README.md`, `VISION.md`

- [ ] **Step 1:** Rewrite the build/deploy/backup/architecture sections of all three docs to match v2 (no litestream, no server builds, no JWT auth, GHCR flow, backup design, `lh connect`, acceptance script usage). Delete stale sections (remotes, webhooks, Elm).
- [ ] **Step 2:** Commit: `docs: v2 architecture`

---

## Task dependency order

Strictly sequential except: Task 9 (GitHub API) can run in parallel with Tasks 6–8; Task 14 (UI) can run in parallel with Tasks 11–13. Everything else depends on its predecessor. Task 15 requires ALL prior tasks.

## Verification summary (what "done" means)

1. `cargo build` + `cargo test` green after every task.
2. LOC meaningfully down from ~20k baseline (expect ~10–12k after Phase 1).
3. `ghcr.io/danbruder/litehouse` image publishes from a tag push (Task 13 step 4).
4. `./e2e/acceptance.sh` prints `ACCEPTANCE PASSED` against the real DO node with `*.s.danbruder.com`.
5. DR drill: wiped node restored from S3 + GHCR with `lh restore`, app reachable again.
