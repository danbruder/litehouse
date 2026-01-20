# Litehouse

A self-hosted platform for deploying and running containerized applications with automatic subdomain routing. Similar to Vercel, but on your own infrastructure.

## Features

- Deploy containerized applications from Git repositories
- Automatic subdomain routing (e.g., `myapp.yourdomain.com`)
- Built-in reverse proxy with automatic HTTPS via Caddy
- Environment variable management
- Container lifecycle management (start, stop, logs)
- Web-based admin interface

## Installation

### Quick Install (Linux)

Run the install script with your domain:

```bash
curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install.sh | sudo sh -s -- --domain lh.example.com
```

This will:
1. Download the latest release binary
2. Install it to `/usr/local/bin/lh`
3. Set up Docker and Caddy containers
4. Configure wildcard subdomain routing

### Manual Installation

Download the latest release:

```bash
# Download and extract
curl -fsSL https://github.com/danbruder/litehouse/releases/latest/download/litehouse-linux-x86_64.tar.gz | tar xz

# Install binary
sudo mv lh /usr/local/bin/
sudo chmod +x /usr/local/bin/lh

# Run install wizard
sudo lh install --domain lh.example.com
```

### Requirements

- Linux (x86_64 or aarch64)
- Docker
- A domain with wildcard DNS configured (e.g., `*.lh.example.com` pointing to your server)

## Usage

### Create and deploy an app

```bash
# Create a new app
lh create myapp

# Add a Git remote
lh remote myapp add https://github.com/user/repo

# Build the app (creates a Docker image)
lh build myapp

# Start the app
lh start myapp
```

Your app is now accessible at `myapp.yourdomain.com`.

### Managing apps

```bash
# List all apps
lh list

# View logs
lh logs myapp

# Stop an app
lh stop myapp

# Set environment variables
lh env myapp set DATABASE_URL=postgres://...

# Delete an app
lh delete myapp
```

### Start the server

```bash
# Start the litehouse server (API + reverse proxy)
lh serve
```

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Build frontend assets (requires Node.js and Elm)
cd assets && npm install && npm run build

# Build release binary (Linux musl)
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl
```

## License

MIT
