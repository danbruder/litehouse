//! MCP (Model Context Protocol) server exposing litehouse app management as
//! tools for AI agents. Speaks JSON-RPC 2.0 over stdio (newline-delimited
//! messages), reusing the same `ApiClient` + client-config admin token the
//! CLI uses. Launched via `lh mcp serve`.
//!
//! stdout carries ONLY protocol messages; all logging goes to stderr (see
//! `main.rs`).
//!
//! The same request handling also backs the remote Streamable HTTP endpoint
//! (`POST /mcp` on the admin host, see `crate::mcp_http`), which passes a
//! [`Mode`] so tools can behave as fits the caller (`create_app` uses the
//! server's own GitHub token remotely) and honour a read-only OAuth grant.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

use crate::api_client::{ApiClient, LogStream};
use crate::config::ClientConfig;

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Protocol revisions this server can speak. `initialize` echoes the
/// client's requested version when it is one of these, else the newest.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// Who is calling and what they may do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    /// Served over HTTP from inside `litehouse-server` rather than as a local
    /// `lh mcp serve` process: no git checkout, no GitHub token.
    pub remote: bool,
    /// Only tools that change nothing (an OAuth grant with the `mcp:read`
    /// scope).
    pub read_only: bool,
}

impl Mode {
    pub const LOCAL: Mode = Mode { remote: false, read_only: false };
}

/// Tools that never change anything.
const READ_ONLY_TOOLS: &[&str] = &[
    "agent_guide",
    "list_apps",
    "app_status",
    "list_deploys",
    "logs",
    "list_domains",
    "backup_status",
    "server_health",
];

/// Write tools a client should confirm before running (MCP `destructiveHint`).
const DESTRUCTIVE_TOOLS: &[&str] = &[
    "deploy",
    "env_set",
    "stop_app",
    "delete_app",
    "remove_domain",
    "create_app",
];

/// `create_app` over remote MCP: the server does the GitHub side itself.
const REMOTE_CREATE_APP_DESCRIPTION: &str = "Create a new app from an existing GitHub repo. \
The server uses its own GitHub token (set in the admin UI's Settings) to check the repo, \
set the LITEHOUSE_DEPLOY_TOKEN Actions secret and commit .github/workflows/litehouse-deploy.yml \
to the default branch. That commit starts the first build and deploy: wait on it with \
list_deploys {wait: true, sha: <commit_sha from the result>}. The repo needs a Dockerfile at \
its root (see agent_guide). Read `warnings` in the result.";

/// Longest `list_deploys` wait over remote HTTP. A tool call held open for
/// ten minutes through a hosted client is likely to be cut off; the agent
/// calls again if the deploy is still in progress.
const REMOTE_MAX_WAIT_SECS: u64 = 120;

fn tool_allowed(name: &str, mode: Mode) -> bool {
    !mode.read_only || READ_ONLY_TOOLS.contains(&name)
}

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

fn initialize_result(params: &Value, mode: Mode) -> Value {
    let requested = params.get("protocolVersion").and_then(|v| v.as_str());
    let version = match requested {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v,
        // A stdio client that sends nothing keeps the version it always got.
        None if !mode.remote => PROTOCOL_VERSION,
        _ => SUPPORTED_PROTOCOL_VERSIONS[SUPPORTED_PROTOCOL_VERSIONS.len() - 1],
    };
    let instructions = if mode.remote {
        format!(
            "{} {}",
            crate::agent_guide::MCP_INSTRUCTIONS,
            crate::agent_guide::MCP_REMOTE_INSTRUCTIONS
        )
    } else {
        crate::agent_guide::MCP_INSTRUCTIONS.to_string()
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "litehouse", "version": env!("CARGO_PKG_VERSION") },
        "instructions": instructions
    })
}

/// The catalog for `mode`: unavailable tools removed, MCP annotations added
/// so clients can ask before running destructive ones.
fn tools_for(mode: Mode) -> Value {
    let Value::Array(tools) = tool_definitions() else {
        unreachable!("tool_definitions is an array")
    };
    let tools: Vec<Value> = tools
        .into_iter()
        .filter(|t| tool_allowed(t["name"].as_str().unwrap_or(""), mode))
        .map(|mut t| {
            let name = t["name"].as_str().unwrap_or("").to_string();
            if mode.remote && name == "create_app" {
                t["description"] = json!(REMOTE_CREATE_APP_DESCRIPTION);
                t["inputSchema"]["properties"]["repo"]["description"] =
                    json!("The GitHub repo: owner/name or https://github.com/owner/name");
                t["inputSchema"]["required"] = json!(["app_name", "repo"]);
            }
            let read_only = READ_ONLY_TOOLS.contains(&name.as_str());
            t["annotations"] = json!({
                "readOnlyHint": read_only,
                "destructiveHint": DESTRUCTIVE_TOOLS.contains(&name.as_str()),
                "openWorldHint": false,
            });
            t
        })
        .collect();
    Value::Array(tools)
}

/// The tool catalog advertised via `tools/list`.
fn tool_definitions() -> Value {
    json!([
        { "name": "agent_guide", "description": "The full guide to deploying with litehouse: app requirements (Dockerfile, port, /data), first-deploy and redeploy steps, how to read failures. Read it before creating or deploying an app.",
          "inputSchema": { "type": "object", "properties": {} } },
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
        { "name": "list_deploys", "description": "List an app's deploy history (newest first). Set wait=true to block until a deploy settles. After a git push ALWAYS pass sha (the pushed commit, or a ref like HEAD) — without it you wait on the newest deploy, which is still the previous one. With sha, a failed GitHub Actions build is reported immediately. Returns an error result unless the deploy succeeded.",
          "inputSchema": { "type": "object", "properties": {
              "app_name": { "type": "string" },
              "limit": { "type": "integer", "default": 20 },
              "wait": { "type": "boolean", "default": false },
              "sha": { "type": "string", "description": "with wait: the commit to wait for (sha or git ref such as HEAD, resolved in the MCP server's working directory)" },
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
        { "name": "server_health", "description": "Server health & security checklist: firewall, open ports, Caddy admin API exposure, SSH password/root login, fail2ban, automatic security updates, pending security updates, reboot pending, disk, memory, clock, TLS certificate expiry, backups, apps. Each check is ok/warn/fail/unknown with a fix.",
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
    handle_request_with(req, api, config, Mode::LOCAL).await
}

/// Handle one raw JSON-RPC message (already parsed as JSON) for the remote
/// endpoint. Returns `None` for notifications.
pub async fn handle_message(
    message: Value,
    api: &ApiClient,
    config: &ClientConfig,
    mode: Mode,
) -> Option<Value> {
    match serde_json::from_value::<JsonRpcRequest>(message) {
        Ok(req) => handle_request_with(req, api, config, mode).await,
        Err(e) => Some(method_error(
            Value::Null,
            -32600,
            format!("Invalid request: {}", e),
        )),
    }
}

async fn handle_request_with(
    req: JsonRpcRequest,
    api: &ApiClient,
    config: &ClientConfig,
    mode: Mode,
) -> Option<Value> {
    // Notifications carry no id and expect no response.
    req.id.as_ref()?;
    let id = req.id.clone().unwrap();

    match req.method.as_str() {
        "initialize" => Some(success(id, initialize_result(&req.params, mode))),
        "ping" => Some(success(id, json!({}))),
        "tools/list" => Some(success(id, json!({ "tools": tools_for(mode) }))),
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
            if !tool_allowed(&name, mode) {
                let why =
                    format!("'{}' isn't allowed: this connection was granted read-only access", name);
                return Some(success(id, tool_result(why, true)));
            }
            let result = match call_tool(&name, &args, api, config, mode).await {
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

/// Dispatch a tool call to the matching `ApiClient` operation.
async fn call_tool(
    name: &str,
    args: &Value,
    api: &ApiClient,
    config: &ClientConfig,
    mode: Mode,
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
        "agent_guide" => Ok(crate::agent_guide::GUIDE.to_string()),
        "list_deploys" => {
            let app = required_str(args, "app_name")?;
            let limit = optional_u64(args, "limit").unwrap_or(20) as u32;
            let wait = optional_bool(args, "wait").unwrap_or(false);
            let mut timeout = optional_u64(args, "timeout").unwrap_or(600);
            if mode.remote {
                timeout = timeout.min(REMOTE_MAX_WAIT_SECS);
            }
            if !wait {
                let deploys = api.list_deploys(&app, limit).await?;
                return Ok(serde_json::to_string_pretty(&deploys)?);
            }
            let sha = match optional_str(args, "sha") {
                // The server has no git checkout to resolve refs against.
                Some(s) if mode.remote => Some(crate::deploy_wait::literal_sha(&s)?),
                Some(s) => Some(crate::deploy_wait::resolve_sha(&s)?),
                None => None,
            };
            // Watching the Actions run needs a GitHub token, which the
            // server deliberately never holds.
            let build = match &sha {
                Some(_) if !mode.remote => {
                    crate::deploy_wait::BuildWatch::discover(api, &app).await
                }
                _ => None,
            };
            let (outcome, deploys) = crate::deploy_wait::wait(
                api,
                &app,
                sha.as_deref(),
                limit,
                timeout,
                build.as_ref(),
            )
            .await?;
            let body = serde_json::to_string_pretty(&json!({
                "outcome": outcome.label(),
                "message": outcome.message(&app, sha.as_deref(), timeout),
                "deploys": deploys,
            }))?;
            match outcome {
                crate::deploy_wait::Outcome::Succeeded(_) => Ok(body),
                _ => Err(anyhow!(body)),
            }
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
        "server_health" => {
            let report = api.server_health().await?;
            Ok(serde_json::to_string_pretty(&report)?)
        }
        "run_backup" => {
            let report = api.run_backup().await?;
            Ok(serde_json::to_string_pretty(&report)?)
        }
        "create_app" if mode.remote => {
            let app = required_str(args, "app_name")?;
            let repo = required_str(args, "repo")?;
            let rotate = optional_bool(args, "rotate_token").unwrap_or(false);
            let mut outcome = api.launch_app(&app, &repo, rotate).await?;
            remote_create_summary(&mut outcome);
            Ok(serde_json::to_string_pretty(&outcome)?)
        }
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
        other => Err(anyhow!("unknown tool '{}'", other)),
    }
}

/// Shape `POST /api/apps/launch`'s result for an agent: never hand it the
/// app's deploy token (it would land in a hosted chat transcript), and say
/// what to do next.
fn remote_create_summary(outcome: &mut Value) {
    let Some(obj) = outcome.as_object_mut() else { return };
    let had_token = obj.remove("deploy_token").is_some();
    let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let next = if obj.get("workflow_committed").and_then(|v| v.as_bool()) == Some(true) {
        match obj.get("commit_sha").and_then(|v| v.as_str()) {
            Some(sha) => format!(
                "GitHub Actions is building the first deploy. Wait with list_deploys \
                 {{app_name: \"{name}\", wait: true, sha: \"{sha}\"}}."
            ),
            None => format!(
                "GitHub Actions is building the first deploy. Wait with list_deploys \
                 {{app_name: \"{name}\", wait: true}}."
            ),
        }
    } else {
        // The workflow text is fine to keep; the token isn't.
        obj.remove("workflow");
        let token_note = if had_token { " The deploy token was withheld from this chat." } else { "" };
        format!(
            "The app exists but its repo isn't wired up.{token_note} Fix the GitHub token \
             (admin UI Settings), then call create_app again with rotate_token: true."
        )
    };
    obj.insert("next".to_string(), json!(next));
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
    let config = ClientConfig::load_effective()?;
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
    async fn initialize_carries_agent_instructions() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(request(1, "initialize", json!({})), &api, &config)
            .await
            .unwrap();
        let instructions = resp["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("agent_guide"));
        assert!(instructions.contains("sha"));
    }

    #[tokio::test]
    async fn agent_guide_tool_returns_the_guide() {
        let (api, config) = dummy_ctx();
        let resp = handle_request(
            request(2, "tools/call", json!({ "name": "agent_guide", "arguments": {} })),
            &api,
            &config,
        )
        .await
        .unwrap();
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("lh deploys <app> --wait --sha HEAD"));
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

    #[test]
    fn remote_create_app_requires_repo_and_is_hidden_read_only() {
        let remote = Mode { remote: true, read_only: false };
        let tools = tools_for(remote);
        let create = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "create_app")
            .expect("create_app over remote MCP");
        assert_eq!(create["inputSchema"]["required"], json!(["app_name", "repo"]));
        assert!(create["description"].as_str().unwrap().contains("server uses its own GitHub token"));

        let ro = tools_for(Mode { remote: true, read_only: true });
        assert!(!ro.as_array().unwrap().iter().any(|t| t["name"] == "create_app"));
    }

    #[test]
    fn remote_create_summary_never_returns_the_deploy_token() {
        let mut ok = json!({ "name": "blog", "workflow_committed": true, "commit_sha": "abc123" });
        remote_create_summary(&mut ok);
        assert!(ok["next"].as_str().unwrap().contains("sha: \"abc123\""));

        let mut failed = json!({
            "name": "blog", "workflow_committed": false, "github_error": "403",
            "deploy_token": "secret", "workflow": "name: litehouse deploy"
        });
        remote_create_summary(&mut failed);
        assert!(failed.get("deploy_token").is_none());
        assert!(failed.get("workflow").is_none());
        assert!(failed["next"].as_str().unwrap().contains("rotate_token"));
        assert!(!failed.to_string().contains("secret"));
    }

}
