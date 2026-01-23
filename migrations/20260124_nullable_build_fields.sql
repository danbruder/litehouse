-- Make image_id, image_tag, and git_commit nullable to support in-progress builds
-- SQLite doesn't support ALTER COLUMN, so we need to recreate the table

-- Create new table with nullable columns
CREATE TABLE build_new (
    id TEXT PRIMARY KEY NOT NULL,
    app_id TEXT NOT NULL,
    image_id TEXT NULL,
    image_tag TEXT NULL,
    git_commit TEXT NULL,
    log_path TEXT NULL,
    status TEXT NOT NULL DEFAULT 'success',

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);

-- Copy existing data
INSERT INTO build_new (id, app_id, image_id, image_tag, git_commit, log_path, status, created_at, updated_at)
SELECT id, app_id, image_id, image_tag, git_commit, log_path, status, created_at, updated_at
FROM build;

-- Drop old table
DROP TABLE build;

-- Rename new table
ALTER TABLE build_new RENAME TO build;

-- Recreate index
CREATE INDEX IF NOT EXISTS idx_build ON build(app_id);
