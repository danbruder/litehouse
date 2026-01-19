-- migrations/20260119_github_oauth.sql
-- GitHub OAuth integration

CREATE TABLE IF NOT EXISTS github_connection (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL UNIQUE,
    github_user_id INTEGER NOT NULL,
    github_username TEXT NOT NULL,
    github_email TEXT,
    access_token TEXT NOT NULL,
    scopes TEXT NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_github_conn_user ON github_connection(user_id);
CREATE INDEX IF NOT EXISTS idx_github_conn_github_user ON github_connection(github_user_id);
