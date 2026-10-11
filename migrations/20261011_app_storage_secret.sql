-- Add a column to system_config for the server-wide secret that per-app
-- storage tokens (LITEHOUSE_STORAGE_TOKEN) are HMAC-derived from. Stored under
-- its own config_type ('app_storage') and generated lazily on first use (see
-- src/storage.rs).
ALTER TABLE system_config ADD COLUMN app_storage_secret TEXT NULL;
