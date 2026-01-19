-- migrations/20260119_assign_default_org.sql
-- Assign a default organization for existing apps

-- Create a default organization for existing apps
INSERT INTO organization (id, name, slug, created_at, updated_at)
SELECT
    'default-org-' || lower(hex(randomblob(16))),
    'Default Organization',
    'default',
    datetime('now'),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM organization WHERE slug = 'default');

-- Assign all existing apps without an organization to the default organization
UPDATE app
SET organization_id = (SELECT id FROM organization WHERE slug = 'default')
WHERE organization_id IS NULL;
