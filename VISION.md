# Litehouse Vision

## Core Vision

**Litehouse is a self-hosted platform for deploying SQLite apps.**

Deploy containerized apps on your own VPS with automatic backups, reverse proxy, and HTTPS. Optimized for solo developers and small teams running SQLite-backed applications.

> **"The self-hosted platform for SQLite apps"**

## Architecture

### Docker Only

Litehouse uses Docker exclusively. No Podman support — Docker permissions and socket handling are well-understood, and supporting both adds complexity without meaningful benefit.

### External Builds

Builds do not happen on the server. A $5 VPS doesn't have the resources to build Docker images reliably. Instead:

1. **GitHub Actions** — Litehouse provides workflow templates that build images and push them to the server
2. **Local builds** — `lh build` builds locally and syncs the image to the server

The server receives pre-built Docker images and runs them. This keeps the server cheap, simple, and fast.

### Restic Backups to S3

Hourly backups via Restic to S3-compatible storage. Restic backs up:

- **App data volumes** — SQLite databases and any other persistent data
- **Docker images** — So apps can be fully restored on a fresh server without rebuilding

No Litestream. Restic is simpler, handles both data and images in one tool, and hourly granularity is sufficient for the target use case.

S3 credentials are collected during `lh server init` and stored in the server's config.

### CLI-First

The CLI (`lh`) is the primary interface and must be as capable as any UI. Every operation is available via CLI. A TUI is aspirational but not a priority.

### Server Setup

`lh server init` runs idempotent bash setup stages:

- Bash scripts are embedded in the Rust binary
- Each stage is independent and can be re-run safely
- Stages can run in parallel where possible
- Collects S3 credentials, domain config upfront

### Reverse Proxy

Caddy runs as a container alongside app containers, providing:
- Automatic HTTPS via Let's Encrypt
- Subdomain routing (`{app}.litehouse.run`)
- Dynamic configuration updates when apps are added/removed

## Roadmap

### Phase 1: Container Platform ✅ Complete

- Docker integration via Bollard
- App lifecycle (create, build, start, stop, delete)
- Git remote management
- Caddy reverse proxy with HTTPS
- End-to-end subdomain routing

### Phase 2: External Builds & Backups (Current)

- GitHub Actions workflow templates for building images
- `lh build` for local builds with image sync to server
- Restic integration for hourly S3 backups (data + images)
- `lh server init` with S3 credential collection
- Idempotent server setup scripts embedded in binary

### Phase 3: DNS & Security Automation

- Cloudflare API integration for automatic DNS records
- Firewall configuration during server init
- Automatic security updates
- Custom domain support

### Phase 4: Polish

- Buildpack-style framework detection (no Dockerfile needed)
- Zero-downtime deploys with health checks
- Deployment history and rollback
- TUI interface

## What Litehouse Is NOT

- Not a multi-machine orchestrator
- Not a generic container platform (use Coolify for that)
- Not for apps that need PostgreSQL/MySQL
- Not trying to replace Kubernetes

## End State

A developer with a SQLite app can:

1. Spin up a $5 VPS
2. Run `curl ... | sh && lh server init`
3. Push to GitHub → image builds in CI → deploys automatically
4. App data backs up to S3 hourly via Restic
5. Never SSH into the server again
6. Pay $5-10/month instead of $100+/month for a PaaS

**This is the experience we're building.**
