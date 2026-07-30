# App health checks & zero-downtime retry

## Problem

`deploy::do_deploy` (`src/deploy.rs`) stops and removes an app's existing
container, then starts the new one — there is no container overlap. During
that window the app is genuinely down: Caddy has no retry tuning and no
awareness of upstream health, so any request that lands during the gap fails
immediately instead of waiting for the new container to come up.

## Goal

Let an app owner opt in to two related Caddy behaviors by configuring a
health check path for their app:

1. **Active health checks** — Caddy polls the configured path on the app's
   upstream and tracks it as up/down based on real HTTP responses, not just
   TCP connection success.
2. **Passive retry** — Caddy holds an in-flight request open and retries the
   upstream for a bounded window instead of failing immediately, masking a
   redeploy's stop/start gap.

Apps that don't configure a health check path keep today's behavior
(unqualified `reverse_proxy`, no retry or active health-check tuning).

## Non-goals

- No per-app tuning of retry/poll timing — durations are fixed defaults for
  every app that opts in.
- No change to the deploy sequence itself (still stop-then-start, not
  blue-green). This feature only changes how Caddy behaves during the
  resulting gap.
- No new health-check UI beyond `lh` CLI commands and the existing app detail
  surfaces (admin UI wiring, if any, is out of scope for this spec).

## Design

### Schema

Add `health_check_path: Option<String>` to `App` (`src/models/app.rs`), same
shape as the existing `custom_domains` column: nullable `TEXT`, no health
check path means no health check configured. New migration
`migrations/20260730_health_check_path.sql`:

```sql
ALTER TABLE app ADD COLUMN health_check_path TEXT;
```

### Validation

Mirror `is_valid_domain`'s precedent (`src/models/app.rs`): add
`is_valid_health_check_path(path: &str) -> bool` requiring the path starts
with `/`, contains no whitespace, and has no scheme/host (i.e. reject
anything containing `://`). Validated at the command layer, same as domains.

### CLI

New `lh health-check` subcommand group in `src/cli.rs`, following the
`DomainCmd` pattern:

- `lh health-check set <app> <path>` — set/replace the health check path.
- `lh health-check unset <app>` — clear it (reverts to current behavior).
- `lh health-check show <app>` — print the configured path, or a "not
  configured" message.

### Command module

New `src/commands/health_check.rs`, mirroring `src/commands/domain.rs`:

```rust
pub enum HealthCheckError {
    AppNotFound(String),
    InvalidPath(String),
    DatabaseError(#[from] crate::db::DatabaseError),
}

pub async fn set(pool, docker, app_name, path) -> Result<(), HealthCheckError>;
pub async fn unset(pool, docker, app_name) -> Result<(), HealthCheckError>;
pub async fn get(pool, app_name) -> Result<Option<String>, HealthCheckError>;
```

`set`/`unset` validate, persist via `db::app::save`, then call
`caddy::sync_configuration` — a sync failure logs a warning but does not fail
the operation (same as `domain::add`/`domain::remove`), since the DB write
already succeeded and the next deploy will re-sync anyway.

### API

New admin routes in `src/api.rs`, alongside the existing domain routes:

- `GET /api/apps/:name/health-check`
- `POST /api/apps/:name/health-check` (body: `{ "path": "/healthz" }`)
- `DELETE /api/apps/:name/health-check`

Handlers follow `list_domains`/`add_domain`/`remove_domain`'s
match-on-error-variant structure (404 for `AppNotFound`, 400 for
`InvalidPath`, 500 otherwise).

### API client

New methods on `ApiClient` (`src/api_client.rs`): `set_health_check`,
`unset_health_check`, `get_health_check`, following `add_domain` /
`remove_domain` / `list_domains`.

### Caddy config generation (`src/caddy.rs`)

Extend the JSON config structs:

```rust
#[derive(Serialize, Deserialize)]
struct Handler {
    handler: String,
    upstreams: Vec<Upstream>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_balancing: Option<LoadBalancing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    health_checks: Option<HealthChecks>,
}

#[derive(Serialize, Deserialize)]
struct LoadBalancing {
    try_duration: String,
    try_interval: String,
}

#[derive(Serialize, Deserialize)]
struct HealthChecks {
    active: ActiveHealthCheck,
}

#[derive(Serialize, Deserialize)]
struct ActiveHealthCheck {
    uri: String,
    interval: String,
    timeout: String,
    expect_status: u16,
}
```

In `build_caddy_config`, when building an app's route: if
`app.health_check_path` is `Some(path)`, set

```rust
load_balancing: Some(LoadBalancing {
    try_duration: "10s".into(),
    try_interval: "250ms".into(),
}),
health_checks: Some(HealthChecks {
    active: ActiveHealthCheck {
        uri: path.clone(),
        interval: "10s".into(),
        timeout: "5s".into(),
        expect_status: 200,
    },
}),
```

Otherwise both fields stay `None` and are omitted from the serialized config
(`skip_serializing_if`), preserving today's output byte-for-byte for apps
without a health check path.

The admin route (`litehouse-server:3030`) is unaffected — it never sets
`health_check_path`, so it never gets these fields.

### Tests

Extend the existing `#[cfg(test)] mod tests` block in `src/caddy.rs`:

- `routes_with_health_check_path_get_load_balancing_and_health_checks` — app
  with `health_check_path = Some("/healthz")` produces JSON containing
  `"try_duration":"10s"`, `"try_interval":"250ms"`, and
  `"uri":"/healthz"`.
- `routes_without_health_check_path_omit_load_balancing` — app with
  `health_check_path = None` produces JSON with no `load_balancing` or
  `health_checks` key at all (guards against a regression that always
  includes the fields with empty/default values).

Also add unit tests for `is_valid_health_check_path` mirroring the existing
`is_valid_domain` test coverage (accepts `/healthz`, rejects empty string,
rejects a path with a scheme, rejects a path with whitespace, rejects a path
not starting with `/`).

## Rollout

No env var, no feature flag — this is inert until an app owner explicitly
runs `lh health-check set`. Existing apps are unaffected until they opt in.
