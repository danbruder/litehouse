# Custom top-level domains per app

Route a self-owned domain (e.g. `familyquotes.app`, plus `www.familyquotes.app`) to
an existing app container, in addition to the derived `{name}.{server_domain}` host.

## Design (settled)

- **Option A**: append custom domains to the app's existing Caddy route `host` matcher.
  Same route, same upstream container. Caddy automatic HTTPS provisions the
  Let's Encrypt cert (HTTP-01) with no extra cert config.
- Storage: nullable `custom_domains TEXT` column on `app`, holding a JSON array of
  hostnames. Empty/NULL = no custom domains (today's behavior).
- DNS is the operator's job: A record for the apex → server IP. Assume Cloudflare
  **DNS-only** (grey cloud) so HTTP-01 succeeds. Document the proxied caveat; do not
  build the DNS-challenge plugin path now.
- Skip on-demand TLS / ask-endpoint (the "customers bring any domain" model) — out of scope.

## Changes

1. **Migration** `migrations/<date>_app_custom_domains.sql`: `ALTER TABLE app ADD COLUMN custom_domains TEXT;`
2. **Model** `src/models/app.rs`: add `custom_domains: Option<String>` to `App`. Add a
   helper `custom_domains_list(&self) -> Vec<String>` that parses the JSON array (empty
   on NULL/parse error). Validate each hostname (has a `.`, lowercase, no scheme/path).
3. **DB** `src/db/app.rs`: add the column to both `save` (upsert) and `insert_or_ignore`
   column lists + bindings. `get_*` use `SELECT *` / `query_as!` so the struct must match.
4. **Caddy** `src/caddy.rs` `build_caddy_config` (~line 696-709): after computing the
   derived `host`, build `hosts = vec![host]` then extend with `app.custom_domains_list()`.
   Use that vec in the route's `HostMatcher`. Certs auto-follow. Add/extend a test asserting
   a custom domain appears in the route hosts.
5. **CLI** new `src/commands/domain.rs` + wire into `src/cli.rs`:
   - `lh domain add <app> <domain>` — validates, appends to the app's list, saves, resyncs Caddy.
   - `lh domain rm <app> <domain>` — removes, saves, resyncs.
   - `lh domain list <app>` (or fold into `lh status`).
   After DB change, call the same Caddy `sync_configuration` path the deploy flow uses.
6. **UX/docs**: on `domain add`, print the required DNS record ("Create an A record:
   `<domain>` → <server IP>; if on Cloudflare, set it DNS-only / grey cloud"). Update
   CLAUDE.md operational notes.

## Verify

- `cargo build` + `cargo test` (caddy config test).
- Manually: add a domain, dump generated Caddy JSON, confirm both hosts on one route → one upstream.
