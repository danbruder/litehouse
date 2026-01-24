-- migrations/20260125_remove_port_requirement.sql
-- Add exposed_port column to build table for Docker network routing

-- Add exposed_port column to build table
-- Stores the port exposed by the Docker image (from Dockerfile EXPOSE directive)
-- Used by Caddy to route traffic to containers on litehouse-network
ALTER TABLE build ADD COLUMN exposed_port TEXT NULL;

-- Keep app.port column for backwards compatibility
-- Set to NULL for all new apps
-- Can be removed in future migration after verification
