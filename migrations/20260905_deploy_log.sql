-- The deploy detail page showed the app's live container logs (whatever the
-- app happens to be printing right now) labeled as if they were the deploy's
-- own logs. Give each deploy a real log of what the deploy engine did --
-- pulling the image, replacing the container, syncing Caddy -- so the page
-- can show that instead.
ALTER TABLE deploy ADD COLUMN log TEXT NOT NULL DEFAULT '';
