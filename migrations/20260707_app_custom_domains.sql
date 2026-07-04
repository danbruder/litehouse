-- Add a nullable column on app for per-app custom top-level domains.
-- Holds a JSON array of hostnames (e.g. ["familyquotes.app", "www.familyquotes.app"]).
-- NULL/empty means no custom domains -- routing falls back to the derived
-- {name}.{server_domain} host only.
ALTER TABLE app ADD COLUMN custom_domains TEXT NULL;
