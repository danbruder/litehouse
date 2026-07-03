-- Add a column to system_config for storing the JSON-serialized report of
-- the most recent backup run (see src/backup.rs::BackupReport).
ALTER TABLE system_config ADD COLUMN last_backup_report TEXT NULL;
