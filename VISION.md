# Litehouse Vision

## Core Vision

**Litehouse is a self-hosted platform for deploying SQLite apps.**

Deploy containerized apps on your own VPS with automatic backups, reverse proxy, and HTTPS. Optimized for solo developers and small teams running SQLite-backed applications.

> **"The self-hosted platform for SQLite apps"**

## Architecture

### Docker Only

Litehouse uses Docker exclusively. No Podman support — Docker permissions and socket handling are well-understood, and supporting both adds complexity without meaningful benefit.

### External Builds — SHIPPED

Builds do not happen on the server. A $5 VPS doesn't have the resources to build Docker images reliably. Instead:

1. **GitHub Actions** — `lh create <app> --repo owner/name` commits a workflow that builds the image and pushes it to `ghcr.io/{owner}/{app}` on every push, then calls the server's deploy hook
2. **Local escape hatch** — `lh deploy <app> --image <ref>` deploys a pre-built image directly, using the same code path as the hook

The server receives pre-built Docker images and runs them. This keeps the server cheap, simple, and fast.

### Daily S3 Backups — SHIPPED

A built-in daily job (no host cron) takes a consistent snapshot of every app's SQLite data via `VACUUM INTO`, plus the server's own state database, tars and uploads each to S3, and prunes anything older than 14 days. `lh backup run` triggers it on demand; `lh backup status --json` reports the last successful date and the last report. A backup day is only marked successful when every app backs up with zero failures.

S3 credentials are collected during `lh install` (or set later via `lh config s3 set`) and stored in the server's database, not a config file.

### Single Admin Token — SHIPPED

One operator, one server: no users, organizations, or JWTs. `lh install` generates a random admin token, prints it once, and stores only its SHA-256 hash. The CLI presents it as a bearer header; the browser presents it as a cookie after a login form. `lh connect <url> --token <token>` is the only auth setup a client needs.

### Server-Rendered Admin UI — SHIPPED

An Askama + HTMX UI served from the same binary (no separate build step, no JS framework, no asset compile pipeline): apps list, app detail, deploy history, and a log tail, cookie-authenticated with the admin token. The CLI remains the primary and most capable interface — the UI is for a quick glance, not for scripting.

### CLI-First

The CLI (`lh`) is the primary interface and must be as capable as any UI. Every operation is available via CLI, non-interactively (flags/env vars, `--json` on read commands, meaningful exit codes) so agents can drive it as easily as a human.

### Server Setup

`lh install --domain <domain> [--s3-*] [--ghcr-token <token>]` runs idempotent bash setup stages embedded in the binary:

- Each stage is independent and can be re-run safely
- Collects S3 credentials and the GHCR read token upfront (optional — can also be set later via `lh config s3 set` / `lh config ghcr set`)
- Pulls the `litehouse-server` image from `ghcr.io/danbruder/litehouse` — the install never builds an image on the droplet
- `lh upgrade` re-runs the same pull/restart path for the binary and container image

### Disaster Recovery — SHIPPED

Nothing on the server is precious. On a freshly installed node: `lh connect` to authenticate, then `lh restore --yes` pulls the newest state-DB snapshot from S3 (apps, deploy tokens, settings), re-pulls each app's image from GHCR, restores each app's data volume from its own S3 backup, and starts every container. `e2e/dr-drill.sh` automates and verifies the full wipe-reinstall-restore cycle against a real droplet; `e2e/acceptance.sh` does the same for first-time onboarding. `examples/hello` is the reference app both scripts deploy.

### Reverse Proxy

Caddy runs as a container alongside app containers, providing:
- Automatic HTTPS via Let's Encrypt
- Subdomain routing (`{app}.{domain}`, domain set at install time)
- Dynamic configuration updates when apps are added/removed

## Roadmap

### Phase 1: Container Platform ✅ Complete

- Docker integration via Bollard
- App lifecycle (create, start, stop, delete)
- Caddy reverse proxy with HTTPS
- End-to-end subdomain routing

### v2: Push-to-Deploy Platform ✅ SHIPPED

- External builds via GitHub Actions → GHCR, deploy hook on the server (no server-side builds, no git remotes tracked in DB)
- `lh create --repo` drunk-proof onboarding: registers app, commits workflow, sets deploy secret
- `lh deploys --wait` as the CI/agent verification primitive
- Daily S3 backups (VACUUM INTO snapshots, 14-day retention), `lh backup run` / `lh backup status --json`
- Disaster recovery: `lh install` → `lh connect` → `lh restore --yes`
- Single admin token auth (no users/orgs/JWT)
- Server-rendered Askama + HTMX admin UI
- `e2e/acceptance.sh` and `e2e/dr-drill.sh` automating both flows end to end

### Phase 3: DNS & Hardening (Next)

- Cloudflare API integration for automatic wildcard DNS record creation during install
- Custom domain support (per-app domains beyond the shared wildcard)
- Multi-arch (`arm64` + `amd64`) `litehouse-server` image on GHCR
- CSRF token hardening for the admin UI (currently an origin-header guard on state-changing routes — move to a proper per-session token)

### Phase 4: Polish

- Buildpack-style framework detection (no Dockerfile needed)
- Zero-downtime deploys with health checks
- Deployment rollback
- TUI interface

## What Litehouse Is NOT

- Not a multi-machine orchestrator
- Not a generic container platform (use Coolify for that)
- Not for apps that need PostgreSQL/MySQL
- Not trying to replace Kubernetes

## End State

A developer with a SQLite app can:

1. Spin up a $5 VPS with wildcard DNS pointed at it
2. Run `curl ... | sudo sh -s -- --domain lh.example.com`, then `lh connect` and `lh create myapp --repo owner/myapp`
3. `git push` → image builds in CI → deploys automatically
4. App data backs up to S3 daily
5. Never SSH into the server again — and if the box dies, `lh install` + `lh restore --yes` on a new one brings everything back
6. Pay $5-10/month instead of $100+/month for a PaaS

**This is the experience we're building.**
