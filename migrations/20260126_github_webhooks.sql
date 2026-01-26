-- GitHub webhook configuration and delivery tracking
-- This migration adds support for automatic GitHub webhook setup and monitoring

CREATE TABLE IF NOT EXISTS webhook_config (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL UNIQUE,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    secret TEXT NOT NULL,
    auto_deploy BOOLEAN NOT NULL DEFAULT 1,
    github_webhook_id INTEGER,  -- GitHub's webhook ID (null if creation failed)
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, active, failed
    error_message TEXT,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_webhook_config_app ON webhook_config(app_id);

CREATE TABLE IF NOT EXISTS webhook_delivery (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT,
    github_delivery_id TEXT,
    github_event TEXT NOT NULL,
    repository_url TEXT NOT NULL,
    ref TEXT,
    commit_sha TEXT,
    status TEXT NOT NULL,  -- matched, signature_invalid, app_not_found, build_triggered, build_failed, ignored_event
    error_message TEXT,
    build_id TEXT,
    payload_snippet TEXT,
    created_at DATETIME NOT NULL,
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE SET NULL,
    FOREIGN KEY (build_id) REFERENCES build(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_delivery_app ON webhook_delivery(app_id);
CREATE INDEX IF NOT EXISTS idx_webhook_delivery_created ON webhook_delivery(created_at);
