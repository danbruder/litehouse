-- migrations/20250112_system_config.sql

-- SYSTEM CONFIG
-- Stores global system configuration including S3 backup settings
CREATE TABLE IF NOT EXISTS system_config (
    id TEXT PRIMARY KEY NOT NULL,
    config_type TEXT NOT NULL UNIQUE, -- e.g., 's3_backup'

    -- S3 Configuration fields (NULL when not S3 type)
    s3_access_key_id TEXT NULL,
    s3_secret_access_key TEXT NULL,
    s3_bucket TEXT NULL,
    s3_region TEXT NULL,
    s3_endpoint TEXT NULL, -- Optional for S3-compatible services
    s3_path_prefix TEXT NULL, -- Optional path prefix, defaults to 'litehouse'

    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_system_config_type ON system_config(config_type);
