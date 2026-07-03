-- Add a column to system_config for storing a GitHub token (read:packages scope)
-- used to authenticate `docker pull` against private ghcr.io images.
ALTER TABLE system_config ADD COLUMN ghcr_token TEXT NULL;
