-- migrations/20250403_initial.sql

-- APP
CREATE TABLE IF NOT EXISTS app (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT UNIQUE NOT NULL,
    state TEXT NOT NULL DEFAULT 'created',

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_app_name ON app(name);

-- APP BUILD
CREATE TABLE IF NOT EXISTS build (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    image_id TEXT NOT NULL,
    image_tag TEXT NOT NULL,
    git_commit TEXT NOT NULL,

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_build ON build(app_id);

-- APP REMOTE
CREATE TABLE IF NOT EXISTS remote (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    name TEXT NOT NULL,
    directory TEXT NOT NULL,
    remote TEXT NOT NULL,
    branch TEXT NOT NULL,

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_remote ON remote(app_id);

-- APP ENV VAR
CREATE TABLE IF NOT EXISTS env_var (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_env_var_app_id_key ON env_var(app_id, key);

-- APP STATE CHANGE 
CREATE TABLE IF NOT EXISTS state_change (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    state TEXT NOT NULL,
    last_state TEXT NULL,
    last_error TEXT NULL,

    created_at DATETIME NOT NULL,
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_state_change_app_id ON state_change(app_id);
