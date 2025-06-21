-- migrations/20250403_initial.sql
-- Create apps table
CREATE TABLE IF NOT EXISTS app (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT UNIQUE NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    restart_policy TEXT NOT NULL DEFAULT 'on-failure';
    last_exit_code INTEGER NULL;
    last_exit_time TIMESTAMP NULL;
);

-- Create index on app name
CREATE INDEX IF NOT EXISTS idx_apps_name ON apps(name);

CREATE TABLE IF NOT EXISTS app_event (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    started_at TIMESTAMP NOT NULL,
    ended_at TIMESTAMP NULL,
    exit_code INTEGER NULL,
    exit_reason TEXT NULL,
    FOREIGN KEY (app_id) REFERENCES apps(id)
);

-- Create index on app_id for quick lookups
CREATE INDEX IF NOT EXISTS idx_process_history_app_id ON process_history(app_id);
