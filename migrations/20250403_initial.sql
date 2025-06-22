-- migrations/20250403_initial.sql
-- Create apps table
CREATE TABLE IF NOT EXISTS app (
  -- Config
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT UNIQUE NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    restart_policy TEXT NOT NULL DEFAULT 'on-failure';

    -- Current state
    state TEXT,
    image_id TEXT,
    last_built_at TEXT,
);

-- Create index on app name
CREATE INDEX IF NOT EXISTS idx_apps_name ON apps(name);

CREATE TABLE IF NOT EXISTS app_environment_var (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    FOREIGN KEY (app_id) REFERENCES apps(id) ON DELETE CASCADE
);

-- Create index on app_id and key for quick lookups
CREATE INDEX IF NOT EXISTS idx_app_env_var_app_id_key ON app_environment_var(app_id, key);

CREATE TABLE IF NOT EXISTS app_state_change (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    state TEXT NOT NULL,
    last_state TEXT NULL,
    last_error TEXT NULL,
    FOREIGN KEY (app_id) REFERENCES apps(id) ON DELETE CASCADE
);

-- Create index on app_id for quick lookups
CREATE INDEX IF NOT EXISTS idx_app_state_change_app_id ON app_state_change(app_id);
