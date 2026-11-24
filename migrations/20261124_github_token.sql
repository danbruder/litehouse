-- GitHub token the server uses to wire up a new app's repo when the app is
-- created from the admin UI or the remote MCP server (set the
-- LITEHOUSE_DEPLOY_TOKEN Actions secret, commit the deploy workflow).
-- `lh create` keeps using the caller's own client-side token.
ALTER TABLE system_config ADD COLUMN github_token TEXT NULL;
