# Litehouse MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `lh mcp serve` subcommand that runs a Model Context Protocol server over stdio, exposing litehouse app management (deploy, status, logs, env, domains, backups, create) as tools AI agents can call.

**Architecture:** A new `src/mcp.rs` module speaks JSON-RPC 2.0 over newline-delimited stdio. It builds an `ApiClient` from the existing `~/.config/litehouse/client-config.toml`, so it talks to the remote admin API with the same admin token as the CLI — no changes to `litehouse-server`, no new auth surface. Each MCP tool is a thin wrapper over an existing `ApiClient` method. Shared "create app + wire GitHub workflow" logic is extracted into `src/provision.rs` so both `lh create` and the `create_app` tool use one code path.

**Tech Stack:** Rust, `serde`/`serde_json` for JSON-RPC framing (no external MCP SDK crate), `tokio` async, existing `ApiClient` (`reqwest`).

**Critical constraint — stdout is the protocol channel:** In `lh mcp serve`, stdout must carry ONLY JSON-RPC messages. Two sources currently pollute it and MUST be fixed first: (1) several `ApiClient` methods `println!` success messages, (2) `tracing`/`fmt()` logging defaults to stdout. Tasks 1–2 fix both before any MCP code depends on a clean stdout.

---

## File Structure

- **Create `src/mcp.rs`** — JSON-RPC framing, the stdio serve loop, tool definitions, and the `call_tool` dispatch. One responsibility: translate MCP messages into `ApiClient` calls and back.
- **Create `src/provision.rs`** — client-side "create app + set repo secret + commit deploy workflow" flow, shared by CLI and MCP.
- **Modify `src/api_client.rs`** — stop printing inside network methods; add `get_status_json`; add `Serialize` to `DeployResult` and `DeployListItem`.
- **Modify `src/cli.rs`** — add the `Mcp` subcommand + dispatch; move `println!`s to the `Start`/`Stop`/`Delete`/`Env`/`Status` call sites; rewrite `run_create` to call `provision::provision_app`; move `infer_repo_from_git` into `provision.rs`.
- **Modify `src/lib.rs`** — declare `pub mod mcp;` and `pub mod provision;`.
- **Modify `src/main.rs`** — send `tracing` output to stderr.
- **Create `tests/mcp_stdio.rs`** — subprocess end-to-end handshake test (spawns the built binary).
- **Modify `CLAUDE.md`** — document the `lh mcp serve` subcommand.

---

## Task 1: Route tracing logs to stderr

Logging currently writes to stdout, which would corrupt the MCP protocol stream. Send it to stderr (conventional for CLIs anyway).

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Point the tracing subscriber at stderr**

In `src/main.rs`, change the `fmt()` builder chain to add `.with_writer(std::io::stderr)`:

```rust
    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .with_level(true)
        .with_writer(std::io::stderr)
        .compact()
        .init();
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds successfully.

- [ ] **Step 3: Verify logs now go to stderr**

Run: `cargo run -- --help 2>/dev/null | head -1`
Expected: NO "Starting Litehouse v..." line on stdout (it now goes to stderr). Running without `2>/dev/null` still shows the log line.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "fix: route tracing output to stderr so stdout stays clean"
```

---

## Task 2: Stop ApiClient network methods from printing; add get_status_json

Move user-facing `println!`s out of `ApiClient` and into the CLI call sites, and add a `get_status_json` that returns the status text instead of printing it. This keeps `ApiClient` a pure client library (required for MCP, better design regardless).

**Files:**
- Modify: `src/api_client.rs` (methods `start_app` ~159, `stop_app` ~174, `delete_app` ~189, `set_env` ~260, `get_status` ~327)
- Modify: `src/cli.rs` (call sites at lines ~411, ~412, ~427, ~454, ~455)

- [ ] **Step 1: Remove the println from `start_app`, `stop_app`, `delete_app`, `set_env`**

In `src/api_client.rs`, delete the trailing `println!(...)` line from each of these four methods so each ends with `.await?;` then `Ok(())`. Example for `start_app`:

```rust
    pub async fn start_app(&self, app_name: &str) -> Result<()> {
        let url = format!("{}/apps/{}/start", self.config.base_url, app_name);

        self.execute_request_text(|client, auth_header| {
            let mut req = client.post(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await?;

        Ok(())
    }
```

Apply the same removal (drop only the `println!` line) to `stop_app`, `delete_app`, and `set_env`.

- [ ] **Step 2: Replace `get_status` with `get_status_json`**

In `src/api_client.rs`, replace the whole `get_status` method with a version that returns the text instead of printing:

```rust
    /// Fetch the status JSON for one app (`Some(name)`) or all apps
    /// (`None`). Returns the server's raw response body.
    pub async fn get_status_json(&self, app_name: Option<&str>) -> Result<String> {
        let url = match app_name {
            Some(name) => format!("{}/apps/{}", self.config.base_url, name),
            None => format!("{}/apps", self.config.base_url),
        };

        self.execute_request_text(|client, auth_header| {
            let mut req = client.get(&url);
            if let Some(header) = auth_header {
                req = req.header("Authorization", header);
            }
            req
        }).await
    }
```

- [ ] **Step 3: Update the CLI call sites to print**

In `src/cli.rs`, update these match arms (around lines 411–455) so the user-facing messages still appear:

```rust
                Commands::Start { app_name } => {
                    api_client.start_app(&app_name).await?;
                    println!("App '{}' started successfully", app_name);
                    Ok(())
                }
                Commands::Stop { app_name } => {
                    api_client.stop_app(&app_name).await?;
                    println!("App '{}' stopped successfully", app_name);
                    Ok(())
                }
```

For `Commands::Delete`:

```rust
                Commands::Delete { app_name } => {
                    api_client.delete_app(&app_name).await?;
                    println!("App '{}' deleted successfully", app_name);
                    Ok(())
                }
```

For `Commands::Env`:

```rust
                Commands::Env {
                    app_name,
                    key,
                    value,
                    delete,
                } => {
                    api_client.set_env(&app_name, &key, &value, delete).await?;
                    println!("Environment variable set for app '{}'", app_name);
                    Ok(())
                }
```

For `Commands::Status`:

```rust
                Commands::Status { app_name } => {
                    let status = api_client.get_status_json(app_name.as_deref()).await?;
                    println!("Status: {}", status);
                    Ok(())
                }
```

(The `Restart` arm at ~413 already calls `stop_app`/`start_app` then prints its own "restarted" message — leave it; it now emits a single clean message instead of three.)

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: builds successfully with no unused-import or type errors.

- [ ] **Step 5: Run the existing test suite**

Run: `cargo test --lib`
Expected: PASS (no behavior change to tested code paths).

- [ ] **Step 6: Commit**

```bash
git add src/api_client.rs src/cli.rs
git commit -m "refactor: move ApiClient success prints to CLI call sites, add get_status_json"
```

---

## Task 3: Extract provision_app into src/provision.rs

Pull the "create app + set repo secret + commit workflow" logic out of `run_create` so the MCP `create_app` tool can reuse it. `infer_repo_from_git` moves too.

**Files:**
- Create: `src/provision.rs`
- Modify: `src/lib.rs`
- Modify: `src/cli.rs` (`run_create` ~761, `infer_repo_from_git` ~879)

- [ ] **Step 1: Create `src/provision.rs`**

```rust
//! Client-side "create an app and wire up its GitHub deploy workflow" flow,
//! shared by `lh create` (CLI) and the MCP `create_app` tool. Registers the
//! app on the server, sets the `LITEHOUSE_DEPLOY_TOKEN` repo secret, and
//! commits `.github/workflows/litehouse-deploy.yml`.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

use crate::api_client::ApiClient;
use crate::config::ClientConfig;

/// Result of a successful provision — a machine-readable summary suitable for
/// JSON output (CLI `--json`, MCP tool result).
#[derive(Debug, Serialize)]
pub struct ProvisionOutcome {
    pub name: String,
    pub url: String,
    pub repo: String,
    pub workflow_committed: bool,
}

/// Register `app_name` on the server (linked to `repo`), then set the deploy
/// secret and commit the deploy workflow to the repo. `repo` may be `None` to
/// infer "owner/name" from the current directory's `origin` git remote.
///
/// `allow_interactive` is passed straight to GitHub token resolution: `false`
/// (the MCP / `--json` path) never blocks on a device-flow prompt and returns
/// a descriptive error if no token is already available.
pub async fn provision_app(
    api_client: &ApiClient,
    config: &ClientConfig,
    app_name: &str,
    repo: Option<String>,
    rotate_token: bool,
    allow_interactive: bool,
) -> Result<ProvisionOutcome> {
    let repo = match repo {
        Some(r) => r,
        None => infer_repo_from_git()?,
    };

    let (owner, repo_name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("repo must be in 'owner/name' form, got '{}'", repo))?;

    let create_result = match api_client.create_app(app_name, Some(&repo), rotate_token).await {
        Ok(r) => r,
        Err(e) if !rotate_token && e.to_string().contains("already exists") => {
            return Err(anyhow!(
                "App '{}' already exists. Pass rotate_token=true to re-link it \
                 (mints a fresh deploy token and re-commits the deploy workflow).",
                app_name
            ));
        }
        Err(e) => return Err(e),
    };

    // The server's base_url already ends in /api; the deploy hook lives at
    // /api/hooks/deploy alongside the rest of the admin API.
    let hook_url = format!("{}/hooks/deploy", config.base_url.trim_end_matches('/'));

    let setup = async {
        let token =
            crate::commands::github_login::resolve_github_token(allow_interactive).await?;
        crate::github::actions::put_actions_secret(
            &token,
            owner,
            repo_name,
            "LITEHOUSE_DEPLOY_TOKEN",
            &create_result.deploy_token,
        )
        .await
        .context("setting LITEHOUSE_DEPLOY_TOKEN secret")?;

        let workflow =
            crate::workflow::render_deploy_workflow(owner, repo_name, app_name, &hook_url);
        crate::github::actions::put_file(
            &token,
            owner,
            repo_name,
            ".github/workflows/litehouse-deploy.yml",
            &workflow,
            "Add litehouse deploy workflow",
        )
        .await
        .context("committing .github/workflows/litehouse-deploy.yml")?;

        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(e) = setup {
        // The app already exists on the server at this point — say so, and
        // surface the most common cause (a token without `workflow` scope).
        return Err(anyhow!(
            "App '{}' was created on the server, but setting up the GitHub workflow for {} \
             failed: {:#}\nHint: committing workflow files needs a GitHub token with the \
             `workflow` scope (e.g. `gh auth refresh -h github.com -s workflow`).",
            app_name,
            repo,
            e
        ));
    }

    Ok(ProvisionOutcome {
        name: create_result.name,
        url: create_result.url,
        repo,
        workflow_committed: true,
    })
}

/// Infer "owner/name" from the `origin` git remote in the current directory.
/// Supports both GitHub HTTPS and SSH remote URL forms.
pub fn infer_repo_from_git() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .context("running `git remote get-url origin`")?;

    if !output.status.success() {
        return Err(anyhow!(
            "Could not find a git remote named 'origin' in the current directory. \
             Pass the repo explicitly as 'owner/name'."
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (owner, repo) = crate::github::api::parse_repo_url(&url).map_err(|_| {
        anyhow!(
            "The 'origin' remote ('{}') is not a github.com repo. Pass the repo explicitly \
             as 'owner/name'.",
            url
        )
    })?;

    Ok(format!("{}/{}", owner, repo))
}
```

- [ ] **Step 2: Declare the module in `src/lib.rs`**

Add (alphabetical placement is fine; keep it near the others):

```rust
pub mod provision;
```

- [ ] **Step 3: Rewrite `run_create` in `src/cli.rs` to call `provision_app`**

Replace the entire `run_create` function body with:

```rust
async fn run_create(
    api_client: &ApiClient,
    config: &ClientConfig,
    app_name: &str,
    repo: Option<String>,
    rotate_token: bool,
    json: bool,
) -> Result<()> {
    // --json implies non-interactive: never block on a device-flow prompt
    // when the caller is a script/agent expecting a single JSON line.
    let allow_interactive = !json;

    match crate::provision::provision_app(
        api_client,
        config,
        app_name,
        repo,
        rotate_token,
        allow_interactive,
    )
    .await
    {
        Ok(outcome) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "name": outcome.name,
                        "url": outcome.url,
                        "repo": outcome.repo,
                        "workflow_committed": outcome.workflow_committed,
                    }))?
                );
            } else {
                println!("App '{}' created", outcome.name);
                println!("  URL:  {}", outcome.url);
                println!("  Repo: {}", outcome.repo);
                println!("git push to deploy.");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("{:#}", e);
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 4: Delete the old `infer_repo_from_git` from `src/cli.rs`**

Remove the `fn infer_repo_from_git() -> Result<String> { ... }` definition near line 879 (it now lives in `provision.rs`). Confirm no remaining references in `cli.rs`:

Run: `grep -n "infer_repo_from_git" src/cli.rs`
Expected: no output (all references removed).

- [ ] **Step 5: Verify it compiles**

Run: `cargo build`
Expected: builds successfully. If `use` imports for `Context` or others in `cli.rs` are now unused, remove them until the build is warning-clean for that file.

- [ ] **Step 6: Run tests**

Run: `cargo test --lib`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/provision.rs src/lib.rs src/cli.rs
git commit -m "refactor: extract provision_app shared by lh create and MCP"
```

---

## Task 4: Add Serialize to DeployResult and DeployListItem

The MCP `deploy` and `list_deploys` tools serialize these structs to JSON output.

**Files:**
- Modify: `src/api_client.rs` (`DeployResult` ~24, `DeployListItem` ~30)

- [ ] **Step 1: Add the derive**

In `src/api_client.rs`, change:

```rust
#[derive(Debug, Deserialize)]
pub struct DeployResult {
```
to
```rust
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct DeployResult {
```

And change:

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct DeployListItem {
```
to
```rust
#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct DeployListItem {
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds successfully.

- [ ] **Step 3: Commit**

```bash
git add src/api_client.rs
git commit -m "chore: derive Serialize on DeployResult and DeployListItem for MCP output"
```

---

## Task 5: MCP protocol framing — initialize, tools/list, dispatch skeleton

Create `src/mcp.rs` with the JSON-RPC types, the request handler for `initialize` / `tools/list` / unknown methods, and a `call_tool` stub. This task is TDD'd with pure in-process unit tests (no network).

**Files:**
- Create: `src/mcp.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/mcp.rs` with framing + handlers + a minimal tool set**

```rust
//! MCP (Model Context Protocol) server exposing litehouse app management as
//! tools for AI agents. Speaks JSON-RPC 2.0 over stdio (newline-delimited
//! messages), reusing the same `ApiClient` + client-config admin token the
//! CLI uses. Launched via `lh mcp serve`.
//!
//! stdout carries ONLY protocol messages; all logging goes to stderr (see
//! `main.rs`).

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

use crate::api_client::{ApiClient, LogStream};
use crate::config::ClientConfig;

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn method_error(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Wrap tool output as an MCP tool result. `is_error=true` marks a failed
/// tool call the agent can reason about (not a JSON-RPC protocol error).
fn tool_result(text: String, is_error: bool) -> Value {
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "litehouse", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// The tool catalog advertised via `tools/list`.
fn tool_definitions() -> Value {
    json!([
        { "name": "list_apps", "description": "List all litehouse apps and their status.",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "app_status", "description": "Get one app's status.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" } }, "required": ["app_name"] } },
        { "name": "deploy", "description": "Deploy a container image to an app (pulls, replaces the running container, syncs Caddy).",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" },
              "image": { "type": "string", "description": "e.g. ghcr.io/org/app:sha-abc123" },
              "sha": { "type": "string", "description": "git commit sha (optional)" } },
              "required": ["app_name", "image"] } },
        { "name": "list_deploys", "description": "List an app's deploy history (newest first). Set wait=true to block until the newest deploy leaves in_progress.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" },
              "limit": { "type": "integer", "default": 20 },
              "wait": { "type": "boolean", "default": false },
              "timeout": { "type": "integer", "description": "max seconds to wait", "default": 600 } },
              "required": ["app_name"] } },
        { "name": "logs", "description": "Fetch recent container logs for an app.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" },
              "lines": { "type": "integer", "default": 100 } },
              "required": ["app_name"] } },
        { "name": "env_set", "description": "Set (or, with delete=true, remove) an environment variable on an app.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" },
              "key": { "type": "string" },
              "value": { "type": "string" },
              "delete": { "type": "boolean", "default": false } },
              "required": ["app_name", "key"] } },
        { "name": "start_app", "description": "Start an app's container.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" } }, "required": ["app_name"] } },
        { "name": "stop_app", "description": "Stop an app's container.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" } }, "required": ["app_name"] } },
        { "name": "delete_app", "description": "Delete an app (removes it from the server and stops its container).",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" } }, "required": ["app_name"] } },
        { "name": "list_domains", "description": "List an app's custom top-level domains.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" } }, "required": ["app_name"] } },
        { "name": "add_domain", "description": "Route a custom top-level domain to an app.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" }, "domain": { "type": "string" } },
              "required": ["app_name", "domain"] } },
        { "name": "remove_domain", "description": "Remove a custom top-level domain from an app.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" }, "domain": { "type": "string" } },
              "required": ["app_name", "domain"] } },
        { "name": "backup_status", "description": "Show the last backup date and report.",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "run_backup", "description": "Trigger a backup run now and return the report.",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "create_app", "description": "Register a new app and wire up its GitHub deploy workflow. Requires a GitHub token already available via $GITHUB_TOKEN, the gh CLI, or a prior `lh github login` (device-flow login cannot run through MCP).",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" },
              "repo": { "type": "string", "description": "owner/name; inferred from the origin git remote if omitted" },
              "rotate_token": { "type": "boolean", "default": false } },
              "required": ["app_name"] } }
    ])
}

/// Handle one parsed request. Returns `None` for notifications (no `id`, or
/// `notifications/initialized`), otherwise `Some(response_json)`.
async fn handle_request(
    req: JsonRpcRequest,
    api: &ApiClient,
    config: &ClientConfig,
) -> Option<Value> {
    // Notifications carry no id and expect no response.
    if req.id.is_none() {
        return None;
    }
    let id = req.id.clone().unwrap();

    match req.method.as_str() {
        "initialize" => Some(success(id, initialize_result())),
        "tools/list" => Some(success(id, json!({ "tools": tool_definitions() }))),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = match call_tool(&name, &args, api, config).await {
                Ok(text) => tool_result(text, false),
                Err(e) => tool_result(format!("{:#}", e), true),
            };
            Some(success(id, result))
        }
        other => Some(method_error(
            id,
            -32601,
            format!("Method not found: {}", other),
        )),
    }
}

// ---- argument helpers ----

fn required_str(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("missing required string argument '{}'", key))
}

fn optional_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn optional_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

fn optional_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

/// Dispatch a tool call to the matching `ApiClient` operation. Filled in by
/// later tasks; for now only unknown-tool handling exists.
async fn call_tool(
    name: &str,
    _args: &Value,
    _api: &ApiClient,
    _config: &ClientConfig,
) -> Result<String> {
    match name {
        other => Err(anyhow!("unknown tool '{}'", other)),
    }
}

/// The stdio serve loop, generic over reader/writer so tests can drive it
/// with in-memory buffers.
async fn run<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    api: &ApiClient,
    config: &ClientConfig,
) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break; // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = method_error(Value::Null, -32700, format!("Parse error: {}", e));
                writeln!(writer, "{}", serde_json::to_string(&resp)?)?;
                writer.flush()?;
                continue;
            }
        };
        if let Some(resp) = handle_request(req, api, config).await {
            writeln!(writer, "{}", serde_json::to_string(&resp)?)?;
            writer.flush()?;
        }
    }
    Ok(())
}

/// Entry point for `lh mcp serve`: load client config, build the API client,
/// and run the stdio loop against real stdin/stdout.
pub async fn serve() -> Result<()> {
    let config = ClientConfig::load()?;
    let api = ApiClient::new(config.clone());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run(stdin.lock(), stdout.lock(), &api, &config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClientConfig;

    fn dummy_ctx() -> (ApiClient, ClientConfig) {
        let config = ClientConfig::default();
        (ApiClient::new(config.clone()), config)
    }

    fn request(id: i64, method: &str, params: Value) -> JsonRpcRequest {
        serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn initialize_reports_server_info() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(request(1, "initialize", json!({})), &api, &config)
            .await
            .expect("initialize returns a response");
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "litehouse");
        assert_eq!(resp["result"]["capabilities"]["tools"], json!({}));
    }

    #[tokio::test]
    async fn tools_list_includes_deploy_and_create() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(request(2, "tools/list", json!({})), &api, &config)
            .await
            .unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"deploy"));
        assert!(names.contains(&"create_app"));
        assert!(names.contains(&"list_apps"));
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        let (api, config) = dummy_ctx();
        let notif: JsonRpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }))
        .unwrap();
        assert!(handle_request(notif, &api, &config).await.is_none());
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(request(3, "bogus/method", json!({})), &api, &config)
            .await
            .unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn unknown_tool_returns_error_result() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(
            request(4, "tools/call", json!({ "name": "nope", "arguments": {} })),
            &api,
            &config,
        )
        .await
        .unwrap();
        // A failed tool call is a successful JSON-RPC response with isError.
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }

    #[test]
    fn required_str_missing_errors() {
        assert!(required_str(&json!({}), "app_name").is_err());
        assert_eq!(
            required_str(&json!({ "app_name": "x" }), "app_name").unwrap(),
            "x"
        );
    }
}
```

- [ ] **Step 2: Declare the module in `src/lib.rs`**

```rust
pub mod mcp;
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test --lib mcp::`
Expected: all `mcp::tests::*` PASS (`initialize_reports_server_info`, `tools_list_includes_deploy_and_create`, `notifications_get_no_response`, `unknown_method_is_method_not_found`, `unknown_tool_returns_error_result`, `required_str_missing_errors`).

- [ ] **Step 4: Commit**

```bash
git add src/mcp.rs src/lib.rs
git commit -m "feat: MCP protocol framing (initialize, tools/list, dispatch skeleton)"
```

---

## Task 6: Wire the `lh mcp serve` subcommand

Add the CLI subcommand and route it to `mcp::serve()`.

**Files:**
- Modify: `src/cli.rs` (Commands enum ~15, top-level dispatch ~357)

- [ ] **Step 1: Add the `Mcp` command and its subcommand enum**

In `src/cli.rs`, add a variant to the `Commands` enum (place it near `Serve`):

```rust
    /// Run an MCP (Model Context Protocol) server over stdio so AI agents can
    /// manage litehouse apps. Reuses the client config + admin token.
    Mcp {
        #[command(subcommand)]
        command: McpCmd,
    },
```

And add the subcommand enum near the other `*Cmd` enums (e.g. after `GhcrCmd`):

```rust
#[derive(Subcommand)]
enum McpCmd {
    /// Start the MCP server on stdio (JSON-RPC 2.0).
    Serve,
}
```

- [ ] **Step 2: Dispatch it in the top-level match**

In `run()`, add an arm alongside `Commands::Serve` (before the `_ =>` catch-all), since MCP loads its own config internally:

```rust
        Commands::Mcp { command } => match command {
            McpCmd::Serve => crate::mcp::serve().await,
        },
```

- [ ] **Step 3: Verify it parses and builds**

Run: `cargo build`
Expected: builds successfully.

- [ ] **Step 4: Smoke-test the handshake manually**

Run:
```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | cargo run -- mcp serve 2>/dev/null
```
Expected: a single JSON line on stdout containing `"serverInfo":{"name":"litehouse"...}` and NO log lines mixed in.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "feat: add lh mcp serve subcommand"
```

---

## Task 7: Implement read-only + mutating tools

Fill in `call_tool` for every tool that maps directly to an `ApiClient` method. (`create_app` and `list_deploys --wait` come in Tasks 8–9.)

**Files:**
- Modify: `src/mcp.rs` (`call_tool`)

- [ ] **Step 1: Replace the `call_tool` stub with the full dispatch (minus create_app/wait)**

```rust
async fn call_tool(
    name: &str,
    args: &Value,
    api: &ApiClient,
    _config: &ClientConfig,
) -> Result<String> {
    match name {
        "list_apps" => api.get_status_json(None).await,
        "app_status" => {
            let app = required_str(args, "app_name")?;
            api.get_status_json(Some(&app)).await
        }
        "deploy" => {
            let app = required_str(args, "app_name")?;
            let image = required_str(args, "image")?;
            let sha = optional_str(args, "sha");
            let result = api.deploy_app(&app, &image, sha.as_deref()).await?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        "list_deploys" => {
            let app = required_str(args, "app_name")?;
            let limit = optional_u64(args, "limit").unwrap_or(20) as u32;
            let deploys = api.list_deploys(&app, limit).await?;
            Ok(serde_json::to_string_pretty(&deploys)?)
        }
        "logs" => {
            let app = required_str(args, "app_name")?;
            let lines = optional_u64(args, "lines").unwrap_or(100) as usize;
            match api.get_logs(&app, lines, false).await? {
                LogStream::Full(logs) => Ok(logs),
                // follow=false always yields Full; treat Lines as empty.
                LogStream::Lines(_) => Ok(String::new()),
            }
        }
        "env_set" => {
            let app = required_str(args, "app_name")?;
            let key = required_str(args, "key")?;
            let value = optional_str(args, "value").unwrap_or_default();
            let delete = optional_bool(args, "delete").unwrap_or(false);
            api.set_env(&app, &key, &value, delete).await?;
            Ok(format!(
                "env var '{}' {} for app '{}'",
                key,
                if delete { "deleted" } else { "set" },
                app
            ))
        }
        "start_app" => {
            let app = required_str(args, "app_name")?;
            api.start_app(&app).await?;
            Ok(format!("app '{}' started", app))
        }
        "stop_app" => {
            let app = required_str(args, "app_name")?;
            api.stop_app(&app).await?;
            Ok(format!("app '{}' stopped", app))
        }
        "delete_app" => {
            let app = required_str(args, "app_name")?;
            api.delete_app(&app).await?;
            Ok(format!("app '{}' deleted", app))
        }
        "list_domains" => {
            let app = required_str(args, "app_name")?;
            let domains = api.list_domains(&app).await?;
            Ok(serde_json::to_string_pretty(&domains)?)
        }
        "add_domain" => {
            let app = required_str(args, "app_name")?;
            let domain = required_str(args, "domain")?;
            api.add_domain(&app, &domain).await?;
            Ok(format!("domain '{}' routed to app '{}'", domain, app))
        }
        "remove_domain" => {
            let app = required_str(args, "app_name")?;
            let domain = required_str(args, "domain")?;
            api.remove_domain(&app, &domain).await?;
            Ok(format!("domain '{}' removed from app '{}'", domain, app))
        }
        "backup_status" => {
            let status = api.backup_status().await?;
            Ok(serde_json::to_string_pretty(&json!({
                "last_backup_date": status.last_backup_date,
                "last_backup_report": status.last_backup_report,
            }))?)
        }
        "run_backup" => {
            let report = api.run_backup().await?;
            Ok(serde_json::to_string_pretty(&report)?)
        }
        other => Err(anyhow!("unknown tool '{}'", other)),
    }
}
```

Note: `_config` stays unused until Task 9 adds `create_app`; the leading underscore keeps the build warning-free.

- [ ] **Step 2: Add unit tests for argument validation (no network)**

Add to the `tests` module in `src/mcp.rs`:

```rust
    #[tokio::test]
    async fn deploy_missing_image_is_error_result() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(
            request(
                5,
                "tools/call",
                json!({ "name": "deploy", "arguments": { "app_name": "x" } }),
            ),
            &api,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("image"));
    }
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib mcp::`
Expected: all mcp tests PASS, including `deploy_missing_image_is_error_result`.

- [ ] **Step 4: Verify it builds warning-clean**

Run: `cargo build 2>&1 | grep -i warning || echo "no warnings"`
Expected: `no warnings` (or only pre-existing unrelated warnings).

- [ ] **Step 5: Commit**

```bash
git add src/mcp.rs
git commit -m "feat: implement MCP read-only and mutating tools"
```

---

## Task 8: list_deploys wait/timeout polling

Add blocking-until-settled behavior to the `list_deploys` tool, mirroring `lh deploys --wait`.

**Files:**
- Modify: `src/mcp.rs`

- [ ] **Step 1: Add a `wait_for_deploy` helper**

Add above `call_tool` in `src/mcp.rs`:

```rust
/// Poll an app's deploys until the newest one leaves `in_progress` (or a
/// deploy first appears), or until `timeout_secs` elapses — the MCP analogue
/// of `lh deploys <app> --wait`. Returns the final deploy list either way.
async fn wait_for_deploy(
    api: &ApiClient,
    app: &str,
    limit: u32,
    timeout_secs: u64,
) -> Result<Vec<crate::api_client::DeployListItem>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let deploys = api.list_deploys(app, limit).await?;
        let settled = match deploys.first() {
            Some(d) => d.status != "in_progress",
            None => false, // no deploy registered yet — keep waiting
        };
        if settled || std::time::Instant::now() >= deadline {
            return Ok(deploys);
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}
```

- [ ] **Step 2: Use it in the `list_deploys` arm**

Replace the `"list_deploys"` arm in `call_tool` with:

```rust
        "list_deploys" => {
            let app = required_str(args, "app_name")?;
            let limit = optional_u64(args, "limit").unwrap_or(20) as u32;
            let wait = optional_bool(args, "wait").unwrap_or(false);
            let timeout = optional_u64(args, "timeout").unwrap_or(600);
            let deploys = if wait {
                wait_for_deploy(api, &app, limit, timeout).await?
            } else {
                api.list_deploys(&app, limit).await?
            };
            Ok(serde_json::to_string_pretty(&deploys)?)
        }
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build`
Expected: builds successfully.

- [ ] **Step 4: Run tests**

Run: `cargo test --lib mcp::`
Expected: PASS (existing mcp tests still green; no new unit test — wait logic needs a live server, covered by manual/e2e).

- [ ] **Step 5: Commit**

```bash
git add src/mcp.rs
git commit -m "feat: support wait/timeout in MCP list_deploys tool"
```

---

## Task 9: create_app tool

Wire the `create_app` tool to the shared `provision_app` (non-interactive: agents can't complete GitHub device flow).

**Files:**
- Modify: `src/mcp.rs`

- [ ] **Step 1: Add the `create_app` arm to `call_tool`**

Change the `call_tool` signature's `_config` back to `config` (it's now used):

```rust
async fn call_tool(
    name: &str,
    args: &Value,
    api: &ApiClient,
    config: &ClientConfig,
) -> Result<String> {
```

Add this arm just before the `other =>` catch-all:

```rust
        "create_app" => {
            let app = required_str(args, "app_name")?;
            let repo = optional_str(args, "repo");
            let rotate = optional_bool(args, "rotate_token").unwrap_or(false);
            // allow_interactive = false: an agent cannot complete GitHub
            // device-flow login, so provision_app returns a descriptive error
            // if no token is already available.
            let outcome =
                crate::provision::provision_app(api, config, &app, repo, rotate, false).await?;
            Ok(serde_json::to_string_pretty(&outcome)?)
        }
```

- [ ] **Step 2: Verify it builds warning-clean**

Run: `cargo build 2>&1 | grep -i "warning: unused" || echo "no unused warnings"`
Expected: `no unused warnings` (the `config` param is now used).

- [ ] **Step 3: Add a unit test for missing app_name**

Add to the `tests` module:

```rust
    #[tokio::test]
    async fn create_app_missing_name_is_error_result() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(
            request(6, "tools/call", json!({ "name": "create_app", "arguments": {} })),
            &api,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], true);
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("app_name"));
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib mcp::`
Expected: all mcp tests PASS including `create_app_missing_name_is_error_result`.

- [ ] **Step 5: Commit**

```bash
git add src/mcp.rs
git commit -m "feat: add MCP create_app tool via shared provision_app"
```

---

## Task 10: End-to-end stdio handshake test (subprocess)

Prove the shipped binary wires up correctly: spawn `lh mcp serve`, do the `initialize` → `tools/list` handshake over its stdio, assert clean JSON responses. Uses no network (both methods are local), so it runs in normal `cargo test`.

**Files:**
- Create: `tests/mcp_stdio.rs`

- [ ] **Step 1: Create the integration test**

```rust
//! End-to-end test of `lh mcp serve`: spawn the built binary, speak JSON-RPC
//! over its stdio, and verify the initialize/tools-list handshake. No network
//! is required (both methods are handled locally).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn mcp_serve_initialize_and_list_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lh"))
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn lh mcp serve");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // initialize
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .unwrap();
    stdin.flush().unwrap();

    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON response");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "litehouse");

    // tools/list
    line.clear();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n")
        .unwrap();
    stdin.flush().unwrap();
    stdout.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"deploy".to_string()));
    assert!(names.contains(&"list_apps".to_string()));

    // Closing stdin ends the serve loop (EOF).
    drop(stdin);
    let _ = child.wait();
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test --test mcp_stdio`
Expected: `mcp_serve_initialize_and_list_tools` PASS. (Cargo builds the `lh` binary first; `CARGO_BIN_EXE_lh` resolves to it.)

- [ ] **Step 3: Commit**

```bash
git add tests/mcp_stdio.rs
git commit -m "test: end-to-end MCP stdio handshake against the built binary"
```

---

## Task 11: Document the subcommand

**Files:**
- Modify: `CLAUDE.md` (Command Structure section)

- [ ] **Step 1: Add an `mcp.rs` entry to the command list**

In `CLAUDE.md`, under "#### 6. Command Structure", add a bullet after the `server.rs` line:

```markdown
- `mcp.rs` (top-level `src/mcp.rs`) - `lh mcp serve`: MCP server over stdio exposing app management as tools for AI agents (deploy, status, logs, env, domains, backups, create). Reuses `ApiClient` + the client-config admin token; no server-side changes.
```

- [ ] **Step 2: Add a short usage note near the data-flow examples**

After the "Onboarding and deploying an app" example in `CLAUDE.md`, add:

```markdown
**AI-agent access (MCP):** `lh mcp serve` runs a Model Context Protocol server on stdio. Point an MCP-capable agent at the command `lh mcp serve` (on a host where `lh connect` has already stored the admin token). Tools mirror the CLI: `deploy`, `list_apps`, `app_status`, `list_deploys` (with `wait`), `logs`, `env_set`, `start_app`/`stop_app`/`delete_app`, `add_domain`/`remove_domain`/`list_domains`, `backup_status`/`run_backup`, and `create_app`. `create_app` needs a GitHub token already available ($GITHUB_TOKEN, `gh`, or a prior `lh github login`) since device-flow login can't run through MCP.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document lh mcp serve subcommand"
```

---

## Self-Review Notes

- **Spec coverage:** Every tool in the design's table has an implementation task — read-only + mutating (Task 7), `list_deploys` wait (Task 8), `create_app` (Task 9). Error-handling-as-`isError` is in Task 5's `handle_request`. Testing (unit framing + subprocess e2e) is Tasks 5/7/9/10. Excluded-scope items (install/upgrade, config s3/ghcr, log follow) are correctly absent.
- **Stdout-cleanliness** (implicit spec requirement for a stdio protocol server) is handled explicitly by Tasks 1–2 before any tool depends on it.
- **Type consistency:** `provision_app` / `ProvisionOutcome` (Task 3) are used identically in `run_create` (Task 3) and the `create_app` tool (Task 9). `get_status_json` (Task 2) is consumed by `list_apps`/`app_status` (Task 7). `wait_for_deploy` returns `Vec<DeployListItem>` (Task 8), which requires the `Serialize` derive added in Task 4. `call_tool`'s `config` param is introduced unused (`_config`) in Task 5/7 and activated in Task 9 — noted at each step to keep the build warning-free.
- **No placeholders:** every code step contains complete, compilable code.
```
