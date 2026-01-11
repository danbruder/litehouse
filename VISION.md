# Product Vision & Strategy

## Core Vision

**Bindrop is a managed application runtime for self-hosted SQLite apps.**

The goal is to provide a Vercel/Netlify-like user experience for deploying SQLite-backed applications on your own infrastructure, with zero operational overhead after initial setup.

## Positioning Statement

> **"The self-hosted platform for SQLite apps"**
>
> Deploy Next.js, Rails, or Django apps with SQLite in minutes. Automatic backups to S3, zero-config DNS, and a web UI so simple you'll never SSH again.

## Target Users

- Solo developers and indie hackers
- Small teams (2-10 people)
- Side projects and MVP deployments
- Anyone using SQLite for production apps
- Developers who want self-hosted infrastructure without ops complexity

## Key Differentiators

1. **SQLite is first-class** - Not an afterthought, but the primary use case
2. **Litestream built-in** - Automatic, bulletproof backups without configuration
3. **Zero-ops after setup** - Set it once, never SSH again
4. **Web UI focused** - Not CLI-heavy like Kamal or Kubernetes
5. **Single-machine optimized** - Not trying to be a distributed orchestrator
6. **Simpler than containers** - Buildpack-style auto-detection
7. **Cheaper than PaaS** - Your own VPS, your own costs

## What Bindrop Is NOT

- Not a multi-machine orchestrator (use Kubernetes/Uncloud for that)
- Not a generic container platform (use Coolify/Dokploy for that)
- Not for PostgreSQL/MySQL apps (use managed databases)
- Not for enterprise scale (optimized for small-scale, high-value apps)

## Core User Experience

### Initial Setup (One Time)

```bash
# On your VPS
curl -fsS https://get.bindrop.dev/install.sh | sh
bindrop server init \
  --s3-access-key=... \
  --s3-secret-key=... \
  --s3-bucket=my-backups \
  --cloudflare-token=...
```

This command automatically:
- Sets up bindrop's own SQLite database with Litestream
- Installs and configures Caddy for reverse proxy
- Configures firewall (ports 22, 80, 443 only)
- Enables automatic security updates
- Stores S3/Cloudflare credentials securely
- Starts the web UI at `https://bindrop.yourdomain.com`

### Deploy an App (Every Time)

**Via Web UI:**
1. Click "New App"
2. Enter GitHub repo URL and app name
3. Bindrop automatically:
   - Detects framework (Next.js, Rails, Django, etc.)
   - Clones and builds the app
   - Provisions SQLite database at `/data/app.db`
   - Sets up Litestream for automatic S3 backups
   - Configures DNS via Cloudflare (`myapp.yourdomain.com`)
   - Provisions HTTPS certificate via Caddy/Let's Encrypt
   - Deploys with zero-downtime
4. App is live at `https://myapp.yourdomain.com`

**Subsequent deploys:**
- Push to GitHub → Webhook triggers build → Zero-downtime deploy
- Or click "Deploy" in web UI
- Automatic rollback if health checks fail

### Database Management

**Via Web UI:**
- View backup status (last backup time, size, health)
- Restore to point-in-time (pick from calendar)
- Download current database snapshot
- View backup history
- Health monitoring with alerts

## Architecture Principles

### 1. Web UI as Primary Interface

- Admin web UI is the canonical way to interact with bindrop
- CLI becomes a thin wrapper around the API (for power users)
- UI accessible at `https://bindrop.yourdomain.com/admin`

### 2. First-Class SQLite + Litestream Integration

**Per-app directory structure:**
```
~/.local/share/bindrop/apps/{app_id}/
  ├── data/
  │   └── app.db           # App's SQLite database
  ├── litestream/
  │   ├── config.yml       # Generated Litestream config
  │   └── replicas/        # Local replica cache
  └── builds/              # Build artifacts
```

**Automatic Litestream features:**
- Auto-generated config per app
- Sidecar container or in-container installation
- Backup path: `s3://bucket/bindrop/{app_name}/db/`
- Health monitoring and alerts
- Point-in-time restore via UI

### 3. Zero-Config DNS with Cloudflare

**Automatic DNS management:**
- User sets Cloudflare API token once during setup
- On app creation: auto-create A record `{app}.yourdomain.com`
- Enable Cloudflare proxy (DDoS protection)
- Configure SSL/TLS to "Full (strict)"
- Delete DNS records on app deletion
- Support custom domains (user adds CNAME, bindrop verifies)

### 4. Security Automation

**Configured once during `bindrop server init`:**
- Firewall: Allow 22 (SSH), 80 (HTTP), 443 (HTTPS) only
- Automatic security updates (security repo only)
- Optional: fail2ban for SSH protection
- Email notifications for critical events

### 5. Buildpack-Style Framework Detection

**Auto-detect and configure:**
- Next.js (with Prisma/Drizzle detection)
- SvelteKit
- Remix
- Rails
- Django
- Laravel
- Generic Node.js/Python/Ruby

**Auto-inject:**
- `DATABASE_URL=file:/data/app.db`
- Litestream wrapper/sidecar
- Health check endpoint detection
- Appropriate runtime dependencies

### 6. Zero-Downtime Deployment

**Blue-green deployment strategy:**
1. Build new image: `{app-name}:new`
2. Start new container: `{app-name}-container-new`
3. Wait for health check to pass (default: `GET /` returns 200)
4. Update Caddy config to point to new container
5. Gracefully stop old container (30s drain)
6. Tag new image as `:latest`
7. Remove old container

**Automatic rollback:**
- If health checks fail after 60s, rollback to previous version
- User can manually rollback from deployment history

### 7. Monitoring & Observability

**Web UI dashboard shows:**
- All apps with status indicators
- Recent deployments (success/failure with logs)
- Backup health (last backup time, size trend)
- Disk usage warnings
- Build logs (streaming, real-time)
- Container logs (streaming, searchable, filterable)

**Alerting (optional, later):**
- Email/webhook when backup fails
- Email when deployment fails
- Disk space warnings (>80% full)

## Development Roadmap

### Phase 1: Finish V2 Foundation ✓ (Current)
- [x] Podman integration
- [x] Basic app lifecycle (create, build, start, stop)
- [x] Git remote management
- [ ] Complete Caddy integration with HTTPS
- [ ] Implement restart command
- [ ] Implement delete command
- [ ] End-to-end subdomain routing test

### Phase 2: Web UI & Core UX (Next Priority)

**Goals:**
- Make bindrop usable without CLI
- GitHub webhook integration
- One-time server setup automation

**Tasks:**
1. Build admin web UI (start minimal)
   - App list page with status indicators
   - Create app form (GitHub URL, name, env vars)
   - App detail page (logs, deployments, settings)
   - Environment variables management UI
   - Live build logs viewer
   - Live container logs viewer
2. GitHub webhook integration
   - Webhook endpoint: `POST /webhooks/github`
   - Validate webhook signatures
   - Trigger builds on push to main/specified branch
   - Show webhook setup instructions in UI
3. Server initialization command
   - `bindrop server init` wizard
   - Collect S3 credentials
   - Collect Cloudflare token
   - Detect server public IP
   - Store credentials securely (encrypted in SQLite)
4. Extend database schema
   - `deployments` table (track deployment history)
   - `system_config` table (S3, Cloudflare credentials)
   - `backup_history` table (track Litestream backups)

**Tech decisions:**
- Web UI: Htmx + Tailwind (stay in Rust ecosystem) OR SvelteKit/React
- Authentication: Simple admin password (stored hashed)
- Real-time logs: Server-Sent Events (SSE)

### Phase 3: SQLite Magic (Killer Feature)

**Goals:**
- Make SQLite + Litestream completely automatic
- Best-in-class database backup/restore UX

**Tasks:**
1. Auto-provision SQLite database per app
   - Create `/data` volume mount
   - Set `DATABASE_URL` env var automatically
   - Initialize database on first run
2. Litestream integration
   - Generate Litestream config per app
   - Run as sidecar container or in-container process
   - Use global S3 credentials from server config
   - Backup path convention: `s3://{bucket}/bindrop/{app_name}/db/`
3. Backup/restore UI
   - Show backup history (timestamp, size, generation)
   - Point-in-time restore (calendar picker)
   - Download current database snapshot
   - Test restore functionality
4. Database health monitoring
   - Track last backup time
   - Alert if backups stop (>24h gap)
   - Show backup size trends
   - Disk space monitoring

**Module structure:**
```
src/litestream.rs
  - configure_for_app()
  - start_sidecar()
  - restore_to_point_in_time()
  - get_backup_history()
  - health_check()
```

### Phase 4: DNS & Security Automation

**Goals:**
- Zero-config DNS via Cloudflare
- Hardened security by default
- User never touches DNS or firewall

**Tasks:**
1. Cloudflare API integration
   - Auto-create DNS A records on app creation
   - Enable Cloudflare proxy (orange cloud)
   - Configure SSL/TLS mode
   - Delete DNS records on app deletion
   - Show DNS propagation status in UI
   - Support custom domains (CNAME validation)
2. Firewall automation
   - Configure `ufw` or `nftables` during `server init`
   - Allow: 22 (SSH), 80 (HTTP), 443 (HTTPS)
   - Deny: All other inbound traffic
   - Optional: fail2ban for SSH brute-force protection
3. Automatic security updates
   - Configure `unattended-upgrades` (Debian/Ubuntu)
   - Security updates only (not all packages)
   - Email notifications for critical updates
   - Automatic container base image updates (scan daily)
4. HTTPS automation
   - Caddy handles Let's Encrypt automatically
   - Ensure certificates renew properly
   - Show certificate status in UI

**Module structure:**
```
src/dns.rs
  - sync_app_dns()
  - delete_app_dns()
  - verify_custom_domain()

src/system.rs
  - configure_firewall()
  - enable_auto_security_updates()
  - get_system_status()
```

### Phase 5: Polish & Developer Experience

**Goals:**
- Buildpack auto-detection (no Dockerfile needed)
- Zero-downtime deploys with rollback
- Production-ready stability

**Tasks:**
1. Buildpack detection system
   - Detect framework from repo contents
   - Generate appropriate Dockerfile automatically
   - Support: Next.js, SvelteKit, Remix, Rails, Django, Laravel
   - Inject SQLite + Litestream dependencies
   - Set proper environment variables
2. Zero-downtime deployment
   - Blue-green deployment strategy
   - Health check support (configurable endpoint)
   - Automatic rollback on health check failure
   - Graceful connection draining
3. Deployment history & rollback
   - Track all deployments in database
   - One-click rollback to previous version
   - Show git commit, timestamp, status for each deploy
   - Deploy from specific git commit/tag
4. Better error handling
   - Friendly error messages in UI
   - Build failure diagnostics
   - Troubleshooting guides
   - Health check debugging
5. Metrics dashboard
   - App uptime tracking
   - Request rate (via Caddy logs)
   - Disk usage trends
   - Memory usage per app
   - Backup size trends

**Module structure:**
```
src/buildpacks/
  - mod.rs (Buildpack trait)
  - nextjs.rs
  - rails.rs
  - django.rs
  - generic.rs
```

## Database Schema Extensions

```sql
-- Extend existing app table
ALTER TABLE app ADD COLUMN database_path TEXT;
ALTER TABLE app ADD COLUMN litestream_enabled BOOLEAN DEFAULT 1;
ALTER TABLE app ADD COLUMN health_check_path TEXT DEFAULT '/';
ALTER TABLE app ADD COLUMN custom_domain TEXT;
ALTER TABLE app ADD COLUMN buildpack TEXT; -- detected framework

-- Track deployment history
CREATE TABLE deployments (
    id INTEGER PRIMARY KEY,
    app_id INTEGER NOT NULL,
    git_commit TEXT,
    git_branch TEXT,
    image_tag TEXT,
    status TEXT,  -- building, success, failed, rolled_back
    started_at DATETIME NOT NULL,
    completed_at DATETIME,
    error_message TEXT,
    deployed_by TEXT, -- 'webhook', 'manual', 'cli'
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);

-- Track Litestream backups
CREATE TABLE backup_history (
    id INTEGER PRIMARY KEY,
    app_id INTEGER NOT NULL,
    timestamp DATETIME NOT NULL,
    size_bytes INTEGER,
    generation TEXT,  -- Litestream generation ID
    index_offset INTEGER, -- Litestream index
    status TEXT,  -- success, failed
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);

-- System-wide configuration
CREATE TABLE system_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    encrypted BOOLEAN DEFAULT 0,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
-- Stores: s3_access_key, s3_secret_key, s3_bucket,
--         cloudflare_token, admin_password_hash, server_public_ip

-- Webhook configurations
CREATE TABLE webhooks (
    id INTEGER PRIMARY KEY,
    app_id INTEGER NOT NULL,
    provider TEXT NOT NULL, -- 'github', 'gitlab', 'bitbucket'
    secret TEXT NOT NULL,
    branch TEXT DEFAULT 'main',
    enabled BOOLEAN DEFAULT 1,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (app_id) REFERENCES app(id) ON DELETE CASCADE
);
```

## Tech Stack

**Current (Keep):**
- Rust + Axum (web framework) + SQLx (database)
- Bollard (Docker/Podman API client)
- Caddy (reverse proxy, automatic HTTPS)
- SQLite (application database)
- Podman (container runtime)

**Add:**
- **Web UI**: Htmx + Tailwind (server-rendered) OR SvelteKit/React (static)
- **Litestream**: Binary installation, managed via systemd or container
- **Cloudflare SDK**: `cloudflare` crate or direct HTTP API
- **System automation**: `systemd`, `ufw`/`nftables`
- **Encryption**: `ring` or `age` for encrypting secrets in database

## Learning From Other Projects

### What to Adopt

**From Uncloud:**
- Imperative operations philosophy (direct feedback, easier debugging)
- Machine provisioning automation (adapt for single-machine setup)
- Clean CLI output and progress indicators
- SSH-based initial setup

**From Fly.io:**
- Health checks before routing traffic
- LiteFS/Litestream approach for SQLite
- Regions concept (not applicable now, but good for future)

**From Vercel/Netlify:**
- Git-push-to-deploy workflow (via webhooks)
- Environment variable management UX
- Automatic HTTPS with zero config
- **One-click rollback** from deployment history
- Build logs streaming in real-time
- Preview deployments (future: deploy PR branches)

**From Heroku:**
- Buildpack concept (auto-detect, no Dockerfile needed)
- Add-ons concept (future: Redis, PostgreSQL, etc.)
- Config vars (environment variables)

**From Coolify/Dokploy:**
- Web UI patterns and organization
- Settings page design
- Docker Compose support (optional, later)

### What NOT to Copy

**From Uncloud:**
- ❌ Decentralized architecture (adds complexity, not needed)
- ❌ CRDT-based state (eventual consistency edge cases)
- ❌ Custom mesh networking (heavy lift, single-machine doesn't need)
- ❌ Managed DNS service (operational overhead)

**From Kubernetes:**
- ❌ YAML configuration hell
- ❌ Control plane complexity
- ❌ Declarative-only approach

**From Docker Swarm:**
- ❌ Multi-machine orchestration (out of scope)
- ❌ Service mesh complexity

## Naming Considerations (Future)

Current name "bindrop" doesn't clearly convey the SQLite focus. Consider:

**Potential directions:**
- Emphasize SQLite: "LiteHost", "SQLiteShip", "LiteOps"
- Emphasize simplicity: "SimpleHost", "ZeroOps", "TinyCloud"
- Emphasize self-hosting: "SelfKit", "OwnHost", "HomeBase"
- Keep bindrop and position with tagline

**Decision: Revisit after Phase 2-3 when product has proven SQLite PMF**

## Success Metrics

**Phase 2 (Web UI) Success:**
- Can deploy a Next.js + Prisma app without touching CLI
- GitHub webhook triggers deploys automatically
- Can view live logs in browser

**Phase 3 (SQLite) Success:**
- Litestream backups happen automatically every 10 seconds
- Can restore database to any point in last 30 days
- User never thinks about database backups

**Phase 4 (DNS) Success:**
- User configures Cloudflare token once
- New apps get HTTPS domains within 2 minutes
- Never touch Cloudflare dashboard again

**Phase 5 (Polish) Success:**
- Can deploy without Dockerfile (buildpack auto-detect)
- Zero-downtime deploys with health checks
- One-click rollback works reliably

**Overall Product Success:**
- User sets up bindrop in <30 minutes
- Deploys first app in <5 minutes
- Never SSHs into server again
- Runs in production for months with zero intervention

## Competitive Positioning

| Platform | Focus | Pros | Cons | Bindrop Advantage |
|----------|-------|------|------|-------------------|
| **Vercel** | Next.js, Edge | Best DX, fast | Expensive, vendor lock-in | Own your infra, 10x cheaper |
| **Netlify** | Jamstack | Easy, generous free tier | Not for dynamic apps | SQLite = dynamic without DB costs |
| **Heroku** | General PaaS | Simple, addons | Expensive, slow | Faster, cheaper, modern stack |
| **Coolify** | Self-hosted PaaS | Feature-rich | Complex, generic | SQLite-first, simpler |
| **Dokploy** | Self-hosted PaaS | Simple setup | Generic, no SQLite focus | Built-in Litestream, zero-config |
| **Kamal** | Container deploy | Simple, proven | CLI-heavy, manual | Web UI, automatic backups |
| **Fly.io** | Global edge | Great DX, LiteFS | Expensive at scale, complex | Single-machine simplicity |

**Bindrop's unique position:** The only platform optimized for self-hosted SQLite apps with automatic backups, zero-config DNS, and a beautiful web UI.

## Open Questions (To Decide)

1. **Multi-tenancy:** Should bindrop support multiple users/teams on one server?
   - Decision: Not initially. Single admin user. Revisit in Phase 5+.

2. **Container runtime:** Podman vs Docker?
   - Decision: Support both, detect which is installed. Prefer Podman (daemonless).

3. **Custom domains:** Free-form vs wildcard subdomain?
   - Decision: Start with wildcard subdomain (`*.yourdomain.com`), add custom later.

4. **Monitoring:** Self-hosted vs external service?
   - Decision: Self-hosted metrics only. Users can add external if needed.

5. **Log retention:** How long to keep logs?
   - Decision: 7 days by default, configurable up to 30 days.

6. **Pricing/Licensing:** Open source vs paid?
   - Decision: Open source (MIT/Apache), monetize via managed hosting later.

## Marketing Taglines (Brainstorm)

- "Vercel for SQLite apps"
- "Deploy SQLite apps with zero ops"
- "Self-hosted platform for indie hackers"
- "The SQLite deployment platform"
- "Heroku simplicity, your infrastructure"
- "One server, infinite apps, zero headaches"

## End State Vision (12-18 Months)

A developer with a Next.js + Prisma + SQLite app can:

1. Spin up a $5-10 VPS
2. Run one command: `curl ... | sh && bindrop server init`
3. Open web UI, enter GitHub repo URL
4. Get a live HTTPS app in <5 minutes
5. Push to GitHub → auto-deploy
6. Database backs up to S3 every 10 seconds
7. Never SSH into the server again
8. Pay $5-10/month instead of $100+/month for Vercel
9. Own their data and infrastructure
10. Sleep well knowing backups are bulletproof

**This is the experience we're building.**
