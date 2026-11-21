# Remote (HTTP) MCP Server — Scoping

Status: **scoping, not implemented.** Follows
`2026-07-25-litehouse-mcp-server-design.md` (the stdio server, `lh mcp serve`).

## Problem

`lh mcp serve` is a local stdio process. It reads the admin token from
`~/.config/litehouse/client-config.toml` and calls the admin API through
`ApiClient`. That works for Claude Code on a laptop where `lh connect` has
run. It doesn't work for:

- **Claude mobile / claude.ai / Desktop.** These clients only take *remote*
  MCP servers (custom connectors). Calls come from Anthropic's cloud
  (`160.79.104.0/21`), so the server has to be publicly reachable.
- **Claude Code in the cloud (web/mobile sessions).** No `lh` binary is
  installed and no client config exists, unless a setup script installs one.

The goal is an MCP endpoint that a Claude client can be pointed at by URL,
plus a settings-page card that makes the setup a copy-paste job.

## What Claude clients support (checked 2026-11-21)

From claude.com/docs/connectors/building/authentication:

- **Transport:** Streamable HTTP is the recommended transport. Legacy HTTP+SSE
  still works but is being deprecated.
- **Auth for custom connectors on claude.ai, Desktop and mobile:**
  - OAuth 2.0 with Dynamic Client Registration (RFC 7591). Supported by default.
  - OAuth with a Client ID Metadata Document. Supported by default.
  - Authless. Supported by default.
  - Static request headers (a fixed bearer token). **Beta, for a limited set of
    organizations, and set by an org Owner.** We can't count on this for a
    personal Pro/Max account.
- **OAuth requirements:**
  - PKCE with S256.
  - A `401` with `WWW-Authenticate: Bearer resource_metadata="…"`.
  - RFC 9728 protected-resource metadata, where `resource` must equal the MCP
    URL exactly.
  - RFC 8414 authorization-server metadata.
  - The token endpoint must accept form encoding.
  - Refresh tokens must rotate for public clients, and refresh failures must
    return `invalid_grant`.
  - Hosted apps redirect to `https://claude.ai/api/mcp/auth_callback`. Claude
    Code uses a loopback redirect: `http://localhost/callback` or
    `http://127.0.0.1/callback`, on any port.
  - Auth spec versions 2025-03-26, 2025-06-18 and 2025-11-25.
- **Claude Code** also takes arbitrary headers:
  `claude mcp add --transport http … --header "Authorization: Bearer …"`. It
  also takes `.mcp.json` entries with `${ENV_VAR}` expansion.

**So a static admin token alone covers Claude Code, but not mobile or
claude.ai. Reaching mobile needs OAuth.** Unverified: whether a custom
connector added on claude.ai shows up inside *Claude Code* cloud sessions. The
`.mcp.json` + environment-secret path below covers those either way.

## Recommendation

### Where it lives: inside `litehouse-server`, at `/mcp` on the admin host

`https://admin.<domain>/mcp`, for example `https://admin.lh.danbruder.com/mcp`.

- The admin host is already public over HTTPS behind Caddy, and `/api` is
  already exposed there behind the admin token. `/mcp` adds no new listener,
  port, DNS name or container.
- A separate service (say, a `litehouse-mcp` container) would need its own
  credential to reach the admin API. Because the server stores only
  `sha256(admin_token)`, that credential would be a second long-lived
  admin-equivalent secret. A separate service also brings its own deploy and
  upgrade path. It's not worth it on a single-server platform.
- Mount it as a sibling of `/api` in `commands/server.rs`, before the UI
  fallback. The OAuth endpoints (`/.well-known/*`, `/oauth/*`) mount alongside
  it.

### Auth: OAuth 2.1 built into the server, plus the bearer admin token

Two ways to authenticate on `/mcp`, both checked by one middleware:

1. **`Authorization: Bearer <admin token>`.** This is the existing check
   (`auth::token_authorized`). It's for Claude Code via `--header` or
   `.mcp.json`. It's phase 1 and is useful on its own.
2. **OAuth access tokens issued by litehouse itself.** This is for claude.ai,
   Desktop and mobile. Litehouse acts as its own authorization server, since
   there are no users to federate: the "identity" is "whoever holds the admin
   token". The flow:
   - **Discovery:**
     - `GET /.well-known/oauth-protected-resource` (RFC 9728) returns
       `resource = https://admin.<domain>/mcp` and
       `authorization_servers = [https://admin.<domain>]`.
     - `GET /.well-known/oauth-authorization-server` (RFC 8414) lists the
       authorize, token and register endpoints, `S256` and
       `grant_types = [authorization_code, refresh_token]`.
   - **`POST /oauth/register` (DCR).** Stores `client_id`, `client_name` and
     `redirect_uris`. Registration is open, as the spec requires, but
     `redirect_uris` must be the claude.ai callback or a loopback
     `/callback`. Anything else is rejected, which removes the open-redirect
     and phishing-client class of bugs.
   - **`GET /oauth/authorize`.** This is a server-rendered Askama consent page,
     like `/login`.
     - If the browser already has the `litehouse_token` cookie, the page shows
       "Allow *Claude* to manage litehouse?", with a scope choice:
       **full** or **read-only**.
     - Otherwise it asks for the admin token, reusing the login form.
     - The `POST` is same-origin-checked (`auth::same_origin`), the same CSRF
       guard as the admin API. Then it redirects with a short-lived,
       single-use code bound to the PKCE challenge.
   - **`POST /oauth/token`.**
     - Code + verifier → access token (1h) + refresh token (30d, rotated on
       each use).
     - Tokens are random, and only their sha256 is stored, in a new
       `oauth_client` table and an `oauth_token` table (migration
       `2026xxxx_oauth.sql`).
     - A failed refresh returns `invalid_grant`.
   - **Revocation.** The settings page lists connected clients with
     last-used times and a revoke button. **Rotating the admin token revokes
     every OAuth grant.**

This is about 400–600 lines of Rust. It's a narrow, well-specified subset of
OAuth: no users, no third-party login, one resource. Pulling in a full OAuth
server crate isn't justified, which matches the hand-rolled JSON-RPC choice in
the stdio design.

**Rejected alternatives:**

- **Authless with a secret URL** (`/mcp/<token>`). The docs say not to read
  tokens from the URL, and the URL ends up in logs and screenshots.
- **Static headers only.** Doesn't reach mobile on a personal account.
- **Federating to GitHub OAuth.** Adds an external dependency and a GitHub
  OAuth app. "Owns the GitHub account" isn't the same as "owns this server".

### Reusing the tool implementations

Today `mcp.rs` is two layers: the JSON-RPC handling (`handle_request`) and
`call_tool`, which calls `ApiClient`. The plan:

1. **Split `mcp.rs` into `mcp/protocol.rs` (transport-agnostic) and
   `mcp/stdio.rs`.**
   - `protocol.rs` holds `handle_request`, `tool_definitions` and
     `call_tool`.
   - `stdio.rs` keeps today's line loop.
   - A new `mcp/http.rs` adds the Streamable HTTP handler.
2. **Put the operations behind a trait.** `call_tool` takes a
   `&dyn LitehouseOps` trait object instead of `&ApiClient`. The trait has
   about 15 methods: `status_json`, `deploy_app`, `list_deploys`, `get_logs`,
   `set_env`, start/stop/delete, the domain methods, the backup methods and
   `get_app_repo`.
   - **stdio:** `ApiClient` implements the trait by delegating. No behaviour
     change.
   - **HTTP:** `InProcessOps` implements it by sending requests into the
     existing `/api` router in-process (`tower::ServiceExt::oneshot`). An
     `InternalCaller` request extension tells the admin middleware the caller
     is already authenticated. Every validation, app lock and side effect stays
     in the handlers. No HTTP round trip happens, and the admin token never has
     to be known in plaintext.
   - `deploy_wait::wait` is generic over the same trait, so `list_deploys`
     with `wait` works on both paths.
3. **Streamable HTTP handler** (`POST /mcp`):
   - It takes one JSON-RPC message (or a batch) and returns
     `application/json`. That's allowed by the spec, and SSE isn't needed
     because litehouse never sends server-initiated messages.
   - `GET /mcp` returns `405`.
   - It issues `Mcp-Session-Id` on `initialize` but keeps no per-session
     state.
   - It negotiates `protocolVersion`, adding `2025-06-18` and `2025-11-25`
     alongside today's `2024-11-05`.
   - It validates `Origin` when present, as the spec requires against DNS
     rebinding.

**Tool differences on the remote path:**

| Tool | Remote behaviour |
|---|---|
| `create_app` | **Omitted from `tools/list`.** It commits a workflow and sets a repo secret using a *client-side* GitHub token, and the server deliberately never talks to the GitHub API. Agents run `lh create` locally, or this can be revisited later. |
| `list_deploys` `sha` | Must be a hex sha. There's no `git rev-parse` because the server has no checkout. `BuildWatch` (failing fast on a red Actions run) is off because the server has no GitHub token. |
| `list_deploys` `wait` | The timeout is capped at about 120s, and the agent re-calls if the deploy is still in progress. Long-held HTTP requests through Anthropic's proxy are a risk. **Verify the real tool-call timeout during implementation.** |
| everything else | Same as stdio. |

Every tool gets MCP annotations: `readOnlyHint` on list, status, logs and
`backup_status`, and `destructiveHint` on `delete_app`, `stop_app`, `env_set`,
`deploy` and `remove_domain`. Claude clients use these to ask for
confirmation before acting.

## Security risks and mitigations

The admin token is effectively root on the box. Anyone holding it can deploy
any image (arbitrary code on the host's Docker network), rewrite any app's
env, delete apps and trigger a restore. `/mcp` doesn't widen *who* can reach
admin operations, since `/api` is already public behind the same token. The
new risks come from *what is now driving them* and *where credentials live*.

1. **Prompt injection steering destructive tools.** This is the main new risk.
   The model reads untrusted text: app logs, deploy error messages, repo
   contents, web pages from the same conversation. Injected instructions
   could get it to call `delete_app`, `env_set` or `deploy` with an attacker's
   image. Mitigations:
   - destructive annotations, so the client asks for confirmation;
   - a **read-only scope** offered on the consent page and enforced
     server-side, where write tools are hidden from `tools/list` and rejected
     on call;
   - keeping `restore`, `config s3/ghcr` and admin-token rotation off MCP
     entirely, as in v1.
2. **Credentials stored in Anthropic's cloud.** OAuth means Claude holds a
   revocable, expiring, scoped token, never the admin token. The bearer path
   does put the admin token into Claude Code config or an environment secret.
   Document that as the user's choice and keep it out of the OAuth flow.
3. **OAuth implementation bugs.**
   - open redirect: fixed by the redirect-URI allowlist;
   - code replay: single-use codes with a 60s TTL and PKCE;
   - token theft from the DB: only hashes are stored;
   - CSRF on consent: same-origin check plus a state parameter;
   - brute-forcing the admin token on the consent page: rate-limit
     `/oauth/authorize` and `/oauth/token` per IP, which also helps `/login`.
4. **Secrets in tool output.** Logs and deploy errors may contain secrets,
   which then live in Claude transcripts. This is already true for stdio. Note
   it in `docs/agents.md`, and never return env var *values* (there is no
   `env_get` tool today; keep it that way).
5. **Optional network allowlist.** A server setting could limit OAuth-token
   access on `/mcp` to `160.79.104.0/21`. It's off by default because Claude
   Code on a laptop uses the bearer path from anywhere.

## Settings UI: "Connect Claude" card

The UI is a new card on `/settings` (`frontend/src/pages/Settings.tsx`). Its
data comes from `GET /api/server/info`, which gains an `mcp_url` field derived
from `admin_host`. The SPA **cannot** show the admin token: the cookie is
`HttpOnly` and the server keeps only its hash. So no snippet ever contains the
real token.

| Row | Copy button content | Token exposure |
|---|---|---|
| Connector URL (claude.ai → Settings → Connectors → Add custom connector; then usable on mobile) | `https://admin.lh.danbruder.com/mcp` | none, OAuth handles it |
| Claude Code, OAuth | `claude mcp add --transport http litehouse https://admin.lh.danbruder.com/mcp` (then `/mcp` → authenticate) | none |
| Claude Code, bearer | `claude mcp add --transport http litehouse https://admin…/mcp --header "Authorization: Bearer $LITEHOUSE_TOKEN"` | placeholder only |
| Claude Code cloud sessions | `.mcp.json` with `"headers": {"Authorization": "Bearer ${LITEHOUSE_TOKEN}"}`, plus a note to add `LITEHOUSE_TOKEN` as an environment secret | placeholder only |
| Local stdio (today) | `claude mcp add litehouse -- lh mcp serve` | none |

Below the snippets, the card lists **connected OAuth clients** (name, scope,
created, last used) with a revoke button, backed by new
`GET/DELETE /api/oauth/clients` endpoints. It uses a copy-to-clipboard button
component (there isn't one yet).

Per the repo rules, changing `frontend/src` means running `npm run build` and
committing `src/ui/spa/`.

## Effort estimate

| Phase | Scope | Estimate |
|---|---|---|
| 1 | Split `mcp.rs`, the `LitehouseOps` trait with `InProcessOps`, `POST /mcp` Streamable HTTP, bearer admin token auth, tool annotations, remote tool differences, tests (protocol over HTTP plus in-process dispatch) | ~1–1.5 days |
| 2 | OAuth: metadata endpoints, DCR, consent page, token/refresh, migration, scopes, rate limit, revoke, tests that walk the full PKCE flow | ~2–3 days |
| 3 | Settings card, `server/info.mcp_url`, OAuth client list and revoke, SPA rebuild | ~0.5–1 day |
| 4 | Real-client check: add the connector on claude.ai, call it from mobile, run Claude Code with both auth modes against `lh.danbruder.com`; update `docs/agents.md`, `CLAUDE.md`, `site/llms.txt` | ~0.5 day |

**Total: about 4–6 days of focused work.** Phase 1 makes Claude Code (local and
cloud) work over HTTP by itself. **Mobile works only after phase 2.**

## Out of scope

- `create_app` over remote MCP (needs server-side GitHub credentials)
- Streaming logs (`follow`) and SSE responses
- Multi-user accounts or per-app tokens (litehouse stays single-admin)
- Listing in Anthropic's connector directory
