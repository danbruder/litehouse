-- OAuth 2.1 for the remote MCP endpoint (`POST /mcp`, see src/oauth.rs).
-- Litehouse is its own authorization server: clients register dynamically
-- (RFC 7591), the admin approves on a consent page, and Claude gets an
-- expiring, revocable token instead of the admin token. Only sha256 hashes
-- of codes and tokens are stored.
CREATE TABLE oauth_client (
    id TEXT PRIMARY KEY NOT NULL,
    client_name TEXT NOT NULL,
    -- JSON array of registered redirect URIs
    redirect_uris TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE oauth_code (
    code_hash TEXT PRIMARY KEY NOT NULL,
    client_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    scope TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE TABLE oauth_token (
    id TEXT PRIMARY KEY NOT NULL,
    client_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    access_hash TEXT NOT NULL UNIQUE,
    access_expires_at INTEGER NOT NULL,
    refresh_hash TEXT NOT NULL UNIQUE,
    refresh_expires_at INTEGER NOT NULL,
    -- admin token hash at grant time: rotating the admin token revokes
    -- every grant
    admin_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);
