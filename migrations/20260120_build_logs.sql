-- Add log_path column to build table for storing build logs
ALTER TABLE build ADD COLUMN log_path TEXT;
