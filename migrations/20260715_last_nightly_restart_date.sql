-- Add a column to system_config for tracking the Eastern-time date
-- (YYYY-MM-DD) the nightly app-restart scheduler last completed a pass.
-- Stored under its own config_type ('nightly_restart_meta') so it can be
-- updated independently of other system_config rows (see src/restart.rs /
-- src/commands/server.rs scheduler).
ALTER TABLE system_config ADD COLUMN last_nightly_restart_date TEXT NULL;
