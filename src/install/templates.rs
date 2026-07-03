/// Template for server configuration file. `admin_token_hash` is the
/// sha256 hex hash of the freshly generated admin token (see
/// `crate::auth::generate_token`/`hash_token`) — the plaintext token itself
/// is never written to disk, only printed once at the end of install.
pub fn server_config_template(domain: &str, admin_token_hash: &str) -> String {
    format!(
        r#"host = "0.0.0.0"
port = 3030
caddy_http_port = 80
caddy_https_port = 443
domain = "{}"
admin_token_hash = "{}"
"#,
        domain, admin_token_hash
    )
}

/// Template for logrotate configuration
pub fn logrotate_template() -> &'static str {
    r#"/var/log/litehouse/*.log {
    daily
    rotate 14
    compress
    delaycompress
    notifempty
    create 0640 litehouse litehouse
    sharedscripts
    postrotate
        systemctl reload litehouse > /dev/null 2>&1 || true
    endscript
}
"#
}

/// Bootstrap script for system preparation
pub fn system_preparation_script() -> &'static str {
    r#"#!/bin/bash
set -e

echo "Updating package lists..."
apt-get update -y

echo "Upgrading existing packages..."
DEBIAN_FRONTEND=noninteractive apt-get upgrade -y

echo "Installing required packages..."
DEBIAN_FRONTEND=noninteractive apt-get install -y \
    docker.io \
    git \
    ufw \
    fail2ban \
    sqlite3

echo "Enabling and starting Docker..."
systemctl enable docker
systemctl start docker

echo "System preparation completed successfully"
"#
}

/// Script for security hardening
pub fn security_hardening_script() -> &'static str {
    r#"#!/bin/bash
set -e

echo "Configuring UFW firewall..."
# Allow SSH
ufw allow 22/tcp
# Allow HTTP
ufw allow 80/tcp
# Allow HTTPS
ufw allow 443/tcp
# Enable UFW (non-interactive)
echo "y" | ufw enable

echo "Configuring fail2ban..."
systemctl enable fail2ban
systemctl start fail2ban

echo "Security hardening completed successfully"
"#
}

/// Script for user and directory setup
pub fn user_setup_script() -> &'static str {
    r#"#!/bin/bash
set -e

echo "Creating litehouse group and user..."

# Create group if it doesn't exist
if ! getent group litehouse > /dev/null 2>&1; then
    groupadd -g 1000 litehouse
    echo "Created litehouse group with GID 1000"
else
    echo "Group litehouse already exists"
fi

# Create user if it doesn't exist
if ! id -u litehouse > /dev/null 2>&1; then
    useradd -r -m -d /opt/litehouse -s /bin/bash -g litehouse -u 1000 litehouse
    echo "Created litehouse user with UID 1000"
else
    echo "User litehouse already exists"
fi

# Add litehouse user to docker group
echo "Adding litehouse user to docker group..."
usermod -aG docker litehouse

echo "Creating directory structure..."
mkdir -p /opt/litehouse/config
mkdir -p /opt/litehouse/data

echo "Setting ownership and permissions..."
chown -R litehouse:litehouse /opt/litehouse
chmod 755 /opt/litehouse

echo "User and directory setup completed successfully"
"#
}

/// Script for Docker configuration
pub fn docker_setup_script() -> &'static str {
    r#"#!/bin/bash
set -e

echo "Configuring Docker for litehouse user..."

# Get litehouse user UID
LITEHOUSE_UID=$(id -u litehouse)

# Verify Docker is running
if ! systemctl is-active --quiet docker; then
    echo "Starting Docker daemon..."
    systemctl start docker
fi

# Verify Docker socket exists and is accessible
SOCKET_PATH="/var/run/docker.sock"
MAX_WAIT=10
WAITED=0

while [ $WAITED -lt $MAX_WAIT ]; do
    if [ -S "$SOCKET_PATH" ]; then
        echo "Docker socket verified at $SOCKET_PATH"
        break
    fi
    echo "Waiting for Docker socket... ($WAITED/$MAX_WAIT)"
    sleep 1
    WAITED=$((WAITED + 1))
done

if [ ! -S "$SOCKET_PATH" ]; then
    echo "Error: Docker socket not found after ${MAX_WAIT} seconds"
    systemctl status docker || echo "Failed to get Docker status"
    exit 1
fi

# Test Docker access as litehouse user
echo "Testing Docker access..."
if ! sudo -u litehouse docker info > /dev/null 2>&1; then
    echo "Error: litehouse user cannot access Docker"
    echo "Make sure litehouse is in the docker group and re-login"
    exit 1
fi

echo "Docker configuration completed successfully"
echo "UID:$LITEHOUSE_UID"
"#
}

/// Script to pull the litehouse-server image from GHCR for the given
/// version. Falls back to `:latest` (logging the fallback) if the exact
/// version tag isn't available yet (e.g. the GHCR publish step hasn't
/// finished, or this is a dev build with no matching release).
///
/// Prints a final `PULLED_TAG:<tag>` line so the calling phase can find out
/// which tag actually landed (`<version>` or `latest`) and use the same
/// image reference when starting the container.
pub fn pull_litehouse_image_script(version: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

VERSION_TAG="ghcr.io/danbruder/litehouse:{version}"
LATEST_TAG="ghcr.io/danbruder/litehouse:latest"

echo "Pulling litehouse-server image ($VERSION_TAG)..."
if docker pull "$VERSION_TAG"; then
  echo "PULLED_TAG:{version}"
else
  echo "Warning: failed to pull $VERSION_TAG, falling back to $LATEST_TAG"
  docker pull "$LATEST_TAG"
  echo "PULLED_TAG:latest"
fi

echo "litehouse-server image pull completed"
"#,
        version = version
    )
}

/// Script to pull the container images needed to run litehouse: Caddy (the
/// reverse proxy) and the sqlite3/alpine helper images used by the
/// backup/restore one-shot containers.
pub fn pull_images_script(_litehouse_uid: &str) -> String {
    r#"#!/bin/bash
set -e

echo "Pulling Caddy container image..."
docker pull caddy:latest

echo "Pulling sqlite3 helper image..."
docker pull keinos/sqlite3:latest

echo "Pulling alpine helper image..."
docker pull alpine:3.20

echo "Image pull completed"
"#
    .to_string()
}

/// Script to start litehouse-server container. `image_tag` is the tag that
/// was actually pulled by `pull_litehouse_image_script` (either the running
/// binary's version, or `latest` if that exact version wasn't published
/// yet), so the two scripts always agree on what to run.
pub fn start_litehouse_container_script(_litehouse_uid: &str, image_tag: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Starting litehouse-server container..."

IMAGE="ghcr.io/danbruder/litehouse:{image_tag}"

# Stop and remove any existing container
docker stop litehouse-server 2>/dev/null || true
docker rm litehouse-server 2>/dev/null || true

# Create the litehouse network if it doesn't exist
docker network create litehouse-network 2>/dev/null || true

# Create Docker volumes if they don't exist
docker volume create litehouse_config 2>/dev/null || true
docker volume create litehouse_data 2>/dev/null || true

# Backups staging area is a HOST directory (not a named volume) so the server
# container and the one-shot snapshot containers it spawns can share it by
# absolute path: the server passes this same path as a bind to siblings.
mkdir -p /opt/litehouse/backups
chmod 777 /opt/litehouse/backups

# Fix volume ownership for litehouse user (UID 1000)
# Docker creates volumes as root by default, but container runs as litehouse (UID 1000)
echo "Setting correct ownership on Docker volumes..."
docker run --rm \
  -v litehouse_config:/config \
  -v litehouse_data:/data \
  alpine:3.20 \
  sh -c 'chown -R 1000:1000 /config /data && chmod 755 /config /data'

# Copy server-config.toml from host into the Docker volume
# This ensures the container sees the production configuration
echo "Copying server-config.toml into Docker volume..."
if [ -f /opt/litehouse/config/server-config.toml ]; then
  docker run --rm \
    -v litehouse_config:/target \
    -v /opt/litehouse/config/server-config.toml:/source/server-config.toml:ro \
    alpine:3.20 \
    sh -c 'cp /source/server-config.toml /target/server-config.toml && chown 1000:1000 /target/server-config.toml'
  echo "Server config copied successfully"
else
  echo "Warning: /opt/litehouse/config/server-config.toml not found, container will use defaults"
fi

# Get Docker socket group ID for permissions
DOCKER_GID=$(stat -c '%g' /var/run/docker.sock)

# Start litehouse-server container with restart policy
# Use Docker volumes instead of bind mounts to avoid permission issues
docker run -d \
  --name litehouse-server \
  --restart=unless-stopped \
  --network litehouse-network \
  -v litehouse_config:/opt/litehouse/config \
  -v litehouse_data:/opt/litehouse/data \
  -v /opt/litehouse/backups:/opt/litehouse/backups \
  -v /var/run/docker.sock:/var/run/docker.sock \
  --group-add "$DOCKER_GID" \
  -e DATABASE_URL=/opt/litehouse/config/litehouse.db \
  -e LITEHOUSE_DIR=/opt/litehouse \
  -e LITEHOUSE_BACKUPS_DIR=/opt/litehouse/backups \
  -e DOCKER_HOST=unix:///var/run/docker.sock \
  -e CADDY_API_URL=http://caddy-container:2019/load \
  -e RUST_LOG=info \
  "$IMAGE"

echo "litehouse-server container started"
"#,
        image_tag = image_tag
    )
}

/// Script to start Caddy container
pub fn start_caddy_container_script(_litehouse_uid: &str) -> String {
    r#"#!/bin/bash
set -e

echo "Starting Caddy container..."

# Stop and remove any existing caddy container
docker stop caddy-container 2>/dev/null || true
docker rm caddy-container 2>/dev/null || true

# Create volumes if they do not exist
docker volume create caddy_data 2>/dev/null || true
docker volume create caddy_config 2>/dev/null || true

# Create the litehouse network if it doesn't exist
docker network create litehouse-network 2>/dev/null || true

# Start Caddy container on the litehouse network
docker run -d \
  --name caddy-container \
  --restart=unless-stopped \
  --network litehouse-network \
  -p 80:80 \
  -p 443:443 \
  -v caddy_data:/data \
  -v caddy_config:/config \
  -e CADDY_ADMIN=0.0.0.0:2019 \
  caddy:latest \
  caddy run --resume

echo "Caddy container started"
"#
    .to_string()
}

/// Generate initial Caddy configuration JSON
pub fn initial_caddy_config(domain: &str) -> String {
    format!(
        r#"{{
  "apps": {{
    "http": {{
      "servers": {{
        "app_proxy": {{
          "listen": [":80", ":443"],
          "routes": [
            {{
              "match": [{{ "host": ["admin.{domain}"] }}],
              "handle": [{{
                "handler": "reverse_proxy",
                "upstreams": [{{ "dial": "litehouse-server:3030" }}]
              }}]
            }}
          ]
        }}
      }}
    }}
  }}
}}"#,
        domain = domain
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn litehouse_run_script_uses_ghcr_image_and_backups_volume() {
        let script = start_litehouse_container_script("1000", "1.2.3");
        assert!(script.contains("ghcr.io/danbruder/litehouse"));
        assert!(script.contains("-v /opt/litehouse/backups:/opt/litehouse/backups"));
        assert!(script.contains("LITEHOUSE_BACKUPS_DIR=/opt/litehouse/backups"));
        assert!(!script.to_lowercase().contains("litestream"));
        assert!(!script.contains("docker build"));
    }

    #[test]
    fn litehouse_run_script_does_not_leak_s3_env_plumbing() {
        let script = start_litehouse_container_script("1000", "1.2.3");
        assert!(!script.contains("S3_ENV_ARGS"));
        assert!(!script.contains("s3-credentials.env"));
    }

    #[test]
    fn pull_litehouse_image_script_targets_requested_version() {
        let script = pull_litehouse_image_script("1.2.3");
        assert!(script.contains("ghcr.io/danbruder/litehouse:1.2.3"));
        assert!(script.contains("ghcr.io/danbruder/litehouse:latest"));
        assert!(!script.contains("docker build"));
    }

    #[test]
    fn pull_images_script_includes_backup_helper_images() {
        let script = pull_images_script("1000");
        assert!(script.contains("keinos/sqlite3"));
        assert!(script.contains("alpine:3.20"));
        assert!(script.contains("caddy:latest"));
    }

    #[test]
    fn server_config_template_persists_only_the_token_hash() {
        let config = server_config_template("example.com", "deadbeef");
        assert!(config.contains("admin_token_hash = \"deadbeef\""));
        assert!(!config.contains("plaintext"));
    }
}
