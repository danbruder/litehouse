# App Object Storage — Design

**Date:** 2026-10-11
**Status:** Proposed

## Goal

Let apps deployed on litehouse store objects (uploads, exports, generated
files) in the S3 bucket litehouse already has for backups, using litehouse's
S3 credentials, in a namespace that is separate from the backups.

## Shape

The server never hands S3 credentials to apps. Instead every app container
gets:

- `LITEHOUSE_STORAGE_URL` = `http://litehouse-server:3030/api/storage`
  (reachable over `litehouse-network`, which every app container joins)
- `LITEHOUSE_STORAGE_TOKEN` = a per-app bearer token

and calls a small API on the server, which signs or proxies the S3 request
with its own credentials and confines it to the app's namespace:

| Route | Does |
|---|---|
| `PUT /api/storage/objects/{key}` | upload body (proxied) |
| `GET /api/storage/objects/{key}` | download (proxied), 404 if missing |
| `DELETE /api/storage/objects/{key}` | delete, idempotent |
| `GET /api/storage/objects?prefix=` | list, app-relative keys |
| `POST /api/storage/presign` | presigned GET/PUT/DELETE URL straight to S3 |

The proxied routes are the easy path for small objects; presigned URLs keep
large bodies and browser uploads off the server.

## Namespace

Objects live at `{path_prefix}/storage/{app_name}/{key}`. The existing roots
are `apps/` (dated tarballs), `blobs/` (incremental blob backup) and
`litehouse/` (state DB). `storage/` is a sibling of all three, not nested in
any of them:

- An app can never address a backup key: the server prepends the namespace,
  and keys with `..`, `.`, empty segments, a leading `/`, backslashes or
  control characters are rejected.
- `prune_old_backups` / `newest_key` list `apps/{app}/` and assume every key
  there is a dated tarball (see the blob backup design for why that matters),
  so app objects must not live there either.

## Tokens

`lhs_{app_id}_{hex(HMAC-SHA256(secret, app_id))}`. The secret is generated on
first use and stored in `system_config` (`config_type = 'app_storage'`,
column `app_storage_secret`).

- Nothing per-app is stored, and verification is one HMAC plus an app lookup.
- The token is stable across restarts and redeploys. `lh restore` never
  copies `system_config`, so a rebuilt server mints a new secret; restore
  starts every container fresh, so each app simply gets its new token.
- Keyed on the app id rather than the name: deleting an app and creating a new
  one with the same name yields a different token, and a deleted app's token
  stops authenticating.
- An explicit `lh env set` of either variable wins over the injected value,
  same as `LITEHOUSE_BLOB_PATH`.

Existing containers pick the variables up on their next start (deploy,
restart, or the nightly restart).

## Non-goals

- **Backup / restore of app objects.** They are already in S3; `lh restore`
  doesn't touch `storage/`, and deleting an app leaves its objects in place.
- **Quotas, per-app buckets or scoped IAM credentials.** Isolation is enforced
  by the server, which is the only holder of the credentials.
- **CLI / UI surface.** Apps use the HTTP API directly.
- **Local dev without the server in Docker.** `litehouse-server` only resolves
  on `litehouse-network`; set `LITEHOUSE_STORAGE_URL` explicitly otherwise.
