-- Add a nullable column on app for an opt-in HTTP health check path.
-- When set, Caddy actively polls this path on the app's upstream and
-- retries in-flight requests for a bounded window during a deploy's
-- stop/start gap instead of failing immediately. NULL means no health
-- check configured -- routing falls back to today's plain reverse_proxy.
ALTER TABLE app ADD COLUMN health_check_path TEXT NULL;
