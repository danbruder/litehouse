# Litehouse Deployment Guide

## Prerequisites

- Linux server (Ubuntu/Debian recommended)
- Podman or Docker installed
- Internet access for pulling container images
- Git installed
- Rust toolchain (for building from source)

## Quick Start (Production Server)

### 1. Install Dependencies

```bash
# Ubuntu/Debian
sudo apt-get update
sudo apt-get install -y podman git build-essential

# Fedora/RHEL
sudo dnf install -y podman git gcc

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### 2. Install SQLx CLI

```bash
cargo install sqlx-cli --no-default-features --features sqlite
```

### 3. Clone and Build Litehouse

```bash
git clone <your-repo-url> litehouse
cd litehouse
git checkout claude/plan-next-priority-ZI5lR

# Set up database
export DATABASE_URL=sqlite://config/litehouse.db
sqlx database create
sqlx migrate run

# Build litehouse
cargo build --release
```

### 4. Start Podman API Service

```bash
# Create socket directory
sudo mkdir -p /run/podman

# Start Podman API (if systemd available)
sudo systemctl enable --now podman.socket

# OR start manually (if no systemd)
sudo podman system service --time=0 unix:///run/podman/podman.sock &
```

### 5. Start Litehouse Server

```bash
# For production (will use ports 80/443)
sudo -E ./target/release/lh serve

# For local development (will use ports 9090/9091)
export LITEHOUSE_LOCAL_DEV=1
./target/release/lh serve
```

The server will:
1. Connect to SQLite database
2. Connect to Podman API
3. Pull and start Caddy container
4. Start admin API on port 80 (or 3030 if non-root)
5. Sync Caddy configuration with existing apps

## Deploying Your First App

### Example: Deploy a Node.js App

```bash
# 1. Create the app
./target/release/lh create myapp

# 2. Add Git remote
./target/release/lh remote myapp add https://github.com/youruser/your-nodejs-app

# 3. Build the app (requires Dockerfile in repo)
./target/release/lh build myapp

# 4. Start the app
./target/release/lh start myapp

# 5. Check status
./target/release/lh status

# Your app is now accessible at:
# Local dev: http://myapp.localhost:9090
# Production: http://myapp.s.danbruder.com
```

### Example: Deploy nginx

```bash
# Create app
./target/release/lh create nginx-test

# Add remote to a repo with nginx Dockerfile
./target/release/lh remote nginx-test add https://github.com/nginxinc/docker-nginx

# Build and start
./target/release/lh build nginx-test
./target/release/lh start nginx-test

# Test
curl http://nginx-test.localhost:9090
```

## Dockerfile Requirements

Your Git repository must contain a `Dockerfile` at the root. Example:

### Node.js Dockerfile

```dockerfile
FROM node:18-alpine

WORKDIR /app

# Copy package files
COPY package*.json ./

# Install dependencies
RUN npm ci --only=production

# Copy app code
COPY . .

# Expose port (litehouse will map this dynamically)
EXPOSE 3000

# Start app
CMD ["node", "index.js"]
```

### Python/Flask Dockerfile

```dockerfile
FROM python:3.11-slim

WORKDIR /app

# Copy requirements
COPY requirements.txt .

# Install dependencies
RUN pip install --no-cache-dir -r requirements.txt

# Copy app code
COPY . .

# Expose port
EXPOSE 5000

# Start app
CMD ["python", "app.py"]
```

### Next.js Dockerfile

```dockerfile
FROM node:18-alpine AS builder

WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM node:18-alpine
WORKDIR /app
COPY --from=builder /app/.next ./.next
COPY --from=builder /app/public ./public
COPY --from=builder /app/package*.json ./
RUN npm ci --only=production

EXPOSE 3000
CMD ["npm", "start"]
```

## Environment Variables

Set environment variables for your apps:

```bash
./target/release/lh env myapp DATABASE_URL "sqlite:///data/app.db"
./target/release/lh env myapp API_KEY "your-secret-key"
./target/release/lh env myapp NODE_ENV "production"

# Delete an env var
./target/release/lh env myapp OLD_VAR "" --delete
```

## Managing Apps

### View All Apps

```bash
./target/release/lh status
```

Output:
```
ID  Name       State     Port
1   myapp      running   8001
2   webapp     stopped   8002
3   api        running   8003
```

### View Logs

```bash
# Last 50 lines
./target/release/lh logs myapp

# Last 100 lines
./target/release/lh logs myapp --lines 100

# Follow logs (real-time)
./target/release/lh logs myapp --follow
```

### Stop an App

```bash
./target/release/lh stop myapp
```

### Restart an App

```bash
./target/release/lh stop myapp
./target/release/lh start myapp
```

### Delete an App

```bash
./target/release/lh delete myapp
# This stops the container and removes it from the database
```

## Troubleshooting

### Podman Socket Not Found

```bash
# Check if Podman service is running
sudo podman system service --time=0 unix:///run/podman/podman.sock &

# Or check systemd
sudo systemctl status podman.socket
sudo systemctl start podman.socket
```

### Caddy Won't Start

```bash
# Check Podman containers
podman ps -a

# Check Caddy logs
podman logs caddy-container

# Manually pull Caddy image
podman pull caddy:latest

# Remove and recreate Caddy
podman rm -f caddy-container
./target/release/lh serve
```

### App Won't Build

```bash
# Check build logs in app directory
ls /opt/litehouse/data/apps/{app-name}/build

# Verify Dockerfile exists in repo
cd /opt/litehouse/data/apps/{app-name}/build
ls -la

# Try building manually
cd /opt/litehouse/data/apps/{app-name}/build
podman build -t {app-name}:latest .
```

### Can't Access App at Subdomain

```bash
# Check if container is running
podman ps

# Check if Caddy is running
podman ps | grep caddy

# Check Caddy configuration
curl http://localhost:2019/config/apps/http/servers/litehouse/routes | jq

# Test direct container access
curl http://localhost:{app-port}

# Test Caddy routing
curl -H "Host: {app-name}.localhost" http://localhost:9090
```

### Database Locked Error

```bash
# Stop litehouse server
pkill lh

# Check for stale connections
lsof /opt/litehouse/config/litehouse.db

# Restart server
./target/release/lh serve
```

## Production Considerations

### 1. Run as Systemd Service

Create `/etc/systemd/system/litehouse.service`:

```ini
[Unit]
Description=Litehouse Application Platform
After=network.target podman.socket

[Service]
Type=simple
User=root
WorkingDirectory=/opt/litehouse
Environment="DATABASE_URL=sqlite:///opt/litehouse/config/litehouse.db"
ExecStart=/opt/litehous./target/release/lh serve
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable litehouse
sudo systemctl start litehouse
sudo systemctl status litehouse
```

### 2. Set Up Firewall

```bash
# Allow HTTP and HTTPS
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow 22/tcp  # SSH
sudo ufw enable
```

### 3. Configure DNS

For production, point your domain's A record to your server:

```
*.s.yourdomain.com  →  YOUR_SERVER_IP
```

Then update Caddy configuration in `src/caddy.rs` to use your domain instead of `s.danbruder.com`.

### 4. Enable HTTPS

Caddy automatically provisions Let's Encrypt certificates for HTTPS. No configuration needed!

### 5. Backup Strategy

```bash
# Backup script
#!/bin/bash
DATE=$(date +%Y%m%d_%H%M%S)
tar -czf /backups/litehouse-$DATE.tar.gz \
  /opt/litehouse/config/litehouse.db \
  /opt/litehouse/data/

# Keep last 7 days
find /backups -name "litehouse-*.tar.gz" -mtime +7 -delete
```

Add to cron:

```bash
# Run daily at 2 AM
0 2 * * * /opt/litehouse/backup.sh
```

## Configuration Files

### Server Config

Location: `/opt/litehouse/config/server-config.toml`

```toml
host = "0.0.0.0"
proxy_host = "0.0.0.0"
proxy_port = 80
caddy_http_port = 9090  # Use 80 in production
caddy_https_port = 9091 # Use 443 in production
```

### Client Config

Location: `/opt/litehouse/config/client-config.toml`

```toml
base_url = "http://admin-api.localhost"  # Update for production
```

## Monitoring

### Check Server Health

```bash
# API health check
curl http://localhost/apps

# Caddy health check
curl http://localhost:2019/config/

# Podman status
podman ps
```

### View Server Logs

```bash
# If running as systemd service
sudo journalctl -u litehouse -f

# If running manually
tail -f /tmp/litehouse-server.log
```

## Limits and Scaling

Current limitations (single-machine setup):
- **Apps:** Limited by available ports (starting at 8000)
- **Resources:** Limited by single server CPU/RAM/disk
- **Concurrency:** All apps run on same machine

Future scaling options:
- Add multi-machine support (Phase 5+)
- Implement resource limits per app
- Add monitoring and auto-scaling

## Security Checklist

- [ ] Firewall configured (only ports 22, 80, 443 open)
- [ ] SSH key authentication enabled (password auth disabled)
- [ ] Regular system updates enabled
- [ ] Backups configured and tested
- [ ] Non-root user for litehouse (recommended)
- [ ] Secrets stored as env vars (not in code)
- [ ] HTTPS enabled via Caddy
- [ ] Rate limiting configured (future)

## Support

For issues or questions:
- Check `ROUTING_STATUS.md` for implementation details
- Review `VISION.md` for product roadmap
- Check GitHub issues
- Review logs: `journalctl -u litehouse -f`

## Next Steps

After basic deployment works:
1. Add webhook support for auto-deploy on Git push (Phase 2)
2. Build web UI for app management (Phase 2)
3. Add SQLite + Litestream integration (Phase 3)
4. Implement Cloudflare DNS automation (Phase 4)
