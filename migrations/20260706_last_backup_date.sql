-- Add a column to system_config for tracking the UTC date (YYYY-MM-DD) the
-- daily backup scheduler last successfully completed a run. Stored under its
-- own config_type ('backup_meta') so it can be updated independently of the
-- backup_report row (see src/backup.rs / src/commands/server.rs scheduler).
ALTER TABLE system_config ADD COLUMN last_backup_date TEXT NULL;
