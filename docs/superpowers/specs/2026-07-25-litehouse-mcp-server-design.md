# Litehouse MCP Server

## Problem

AI agents currently have no first-class way to deploy, inspect, or manage
litehouse apps — they'd have to shell out to the `lh` CLI and parse text
output. An MCP server exposes the same capabilities as structured tools an
agent can call directly.

## Approach

A new `lh mcp serve` subcommand runs an MCP server over stdio. It builds an
`ApiClient` (`src/api_client.rs`) from the same
`~/.config/litehouse/client-config.toml` the CLI already uses, so it talks to
the remote admin API with the same admin token and base URL — no new auth
surface, no changes to `litehouse-server` itself. An agent launches
`lh mcp serve` as its MCP server process wherever `lh` is already installed
and connected (`lh connect`).

The JSON-RPC 2.0 stdio framing (`initialize`, `tools/list`, `tools/call`) is
hand-rolled rather than pulled in from an external MCP SDK crate. The
protocol subset needed here is small, and litehouse already favors direct
implementations over heavy frameworks (e.g. axum/hyper used directly rather
than a higher-level web framework).

## Tool surface

Mirrors CLI capability, scoped to app lifecycle and deploy operations.
Excludes server bootstrap (`install`/`upgrade` — root-only, not applicable to
a remote agent) and credential management (`config s3/ghcr set|get` — a
separate blast radius from "deploy stuff"; can be added later if needed).

| Tool | Args | Wraps |
|---|---|---|
| `list_apps` | — | `ApiClient::get_status(None)` |
| `app_status` | `app_name` | `ApiClient::get_status(Some(name))` |
| `deploy` | `app_name, image, sha?` | `ApiClient::deploy_app` |
| `list_deploys` | `app_name, limit?, wait?, timeout?` | `ApiClient::list_deploys`, reusing the existing poll-until-not-`in_progress` loop behind `lh deploys --wait` |
| `logs` | `app_name, lines?` | `ApiClient::get_logs` (non-follow only — a single tool call can't stream) |
| `env_set` | `app_name, key, value, delete?` | `ApiClient::set_env` |
| `list_domains` | `app_name` | `ApiClient::list_domains` |
| `add_domain` | `app_name, domain` | `ApiClient::add_domain` |
| `remove_domain` | `app_name, domain` | `ApiClient::remove_domain` |
| `start_app` | `app_name` | `ApiClient::start_app` |
| `stop_app` | `app_name` | `ApiClient::stop_app` |
| `delete_app` | `app_name` | `ApiClient::delete_app` |
| `create_app` | `app_name, repo?, rotate_token?` | mirrors `lh create`; if GitHub device-flow login is needed and no cached credentials exist, returns an error telling the caller to run `lh github login` first |
| `backup_status` | — | `ApiClient::backup_status` |
| `run_backup` | — | `ApiClient::run_backup` |

`list_apps` and `app_status` pass the server's JSON response through
verbatim as the tool result text, matching what `ApiClient::get_status`
already fetches (it currently prints this JSON via `println!`; it will be
refactored to return the JSON string so both the CLI and the MCP tool can use
it).

## Error handling

Each tool call wraps its `ApiClient` result. An `Err` becomes an MCP tool
error result (`isError: true`, text = the `anyhow` message) rather than a
JSON-RPC protocol-level error, so the agent sees failures like "app not
found" as an ordinary tool failure it can reason about and retry or report,
not a broken connection.

## Testing

- Unit tests for the JSON-RPC framing: `initialize` handshake, `tools/list`
  shape, `tools/call` dispatch and error-result formatting.
- An integration test that spawns the built `lh` binary as a subprocess,
  pipes stdin/stdout, and drives a couple of real tool calls (e.g.
  `list_apps`, `deploy`) against a local test server — matching the existing
  Docker-integration-test style already used in this repo (`src/docker.rs`).

## Out of scope (v1)

- `install` / `upgrade` (root-only server bootstrap)
- `config s3/ghcr set|get` (credential management)
- Streaming log `follow` (no natural fit for a one-shot tool call)
