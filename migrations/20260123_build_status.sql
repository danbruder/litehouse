-- Add status column to build table for tracking build progress
ALTER TABLE build ADD COLUMN status TEXT NOT NULL DEFAULT 'success';
