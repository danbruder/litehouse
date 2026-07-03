-- v2: single-operator platform. Drop multi-user auth, builds, remotes, webhooks.

-- `app.organization_id` was added via `ALTER TABLE ... REFERENCES organization(id)`,
-- which bakes a real foreign-key constraint into the column even though it came
-- from an ALTER rather than the original CREATE TABLE. Since `organization` is
-- dropped below, that FK would make every INSERT/UPDATE on `app` fail with
-- "no such table: main.organization" once foreign key enforcement is on. Rebuild
-- the table so `organization_id` becomes a plain, unused free-text column (kept
-- only because dropping a column outright is needless churn for v2), and add
-- the new v2 columns in the same rebuild.
CREATE TABLE app_new (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT UNIQUE NOT NULL,
    state TEXT NOT NULL DEFAULT 'created',
    port INTEGER NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    organization_id TEXT NULL,
    repo TEXT,              -- "owner/name"
    image TEXT,             -- last deployed image ref
    exposed_port TEXT,      -- detected from image on deploy
    deploy_token_hash TEXT  -- sha256 hex of per-app deploy token
);
INSERT INTO app_new (id, name, state, port, created_at, updated_at, organization_id)
SELECT id, name, state, port, created_at, updated_at, organization_id FROM app;
DROP TABLE app;
ALTER TABLE app_new RENAME TO app;
CREATE INDEX idx_app_name ON app(name);
CREATE INDEX idx_app_org ON app(organization_id);

DROP TABLE IF EXISTS refresh_token;
DROP TABLE IF EXISTS organization_member;
DROP TABLE IF EXISTS organization;
DROP TABLE IF EXISTS "user";
DROP TABLE IF EXISTS github_connection;
DROP TABLE IF EXISTS webhook_config;
DROP TABLE IF EXISTS webhook_delivery;
DROP TABLE IF EXISTS remote;
DROP TABLE IF EXISTS build;
DROP TABLE IF EXISTS state_change;

CREATE TABLE deploy (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL REFERENCES app(id),
    image TEXT NOT NULL,
    git_sha TEXT,
    status TEXT NOT NULL DEFAULT 'in_progress',    -- in_progress | succeeded | failed
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_deploy_app ON deploy(app_id, created_at DESC);
