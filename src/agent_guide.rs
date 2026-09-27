//! The agent guide (`docs/agents.md`), compiled into the binary so the copy
//! an agent reads always matches the `lh` it's running. Served by
//! `lh agent-guide` and the MCP server's `agent_guide` tool; summarized in
//! the MCP `initialize` instructions.

pub const GUIDE: &str = include_str!("../docs/agents.md");

/// Short version for MCP `initialize.instructions`, which clients may put in
/// the model's context on every turn — so it points at the full guide
/// instead of inlining it.
pub const MCP_INSTRUCTIONS: &str = "litehouse deploys Docker apps from GitHub \
to the user's own server (https://<app>.<domain>, nightly S3 backups of /data). \
Before creating or deploying an app, call `agent_guide` once: it has the app \
contract (Dockerfile at the repo root, listen on 0.0.0.0 and EXPOSE the port, \
persistent data only under /data) and the exact deploy steps. After every git \
push, verify with `list_deploys` {wait: true, sha: \"HEAD\"}: without sha you \
would read the previous deploy's result. Never ask the user to paste the admin \
token into the chat. Ask before `delete_app` or rotating tokens.";
