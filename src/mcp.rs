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

/// Dispatch a tool call to the matching `ApiClient` operation.
async fn call_tool(
    name: &str,
    args: &Value,
    api: &ApiClient,
    config: &ClientConfig,
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
            let wait = optional_bool(args, "wait").unwrap_or(false);
            let timeout = optional_u64(args, "timeout").unwrap_or(600);
            let deploys = if wait {
                wait_for_deploy(api, &app, limit, timeout).await?
            } else {
                api.list_deploys(&app, limit).await?
            };
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
}
