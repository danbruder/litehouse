/// Template for server configuration file
pub fn server_config_template(domain: &str) -> String {
    format!(
        r#"host = "0.0.0.0"
proxy_host = "0.0.0.0"
proxy_port = 3030
domain = "{}"
"#,
        domain
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

# Add litehouse user to docker group
usermod -aG docker litehouse

# Start and enable Docker service
systemctl enable docker
systemctl start docker

# Wait for Docker socket to be available
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
    echo "Error: Failed to find Docker socket after ${MAX_WAIT} seconds"
    echo "Attempting to debug..."
    systemctl status docker || echo "Failed to get docker status"
    exit 1
fi

# Ensure litehouse user has access to Docker socket
chmod 666 "$SOCKET_PATH" || true

echo "Docker configuration completed successfully"
echo "UID:$LITEHOUSE_UID"
"#
}

/// Script to wait for service to be active
pub fn wait_for_service_script() -> &'static str {
    r#"#!/bin/bash

SERVICE_NAME="$1"
MAX_WAIT=30
WAITED=0

echo "Waiting for $SERVICE_NAME to become active..."

while [ $WAITED -lt $MAX_WAIT ]; do
    if systemctl is-active --quiet $SERVICE_NAME; then
        echo "$SERVICE_NAME is active"
        exit 0
    fi
    sleep 1
    WAITED=$((WAITED + 1))
done

echo "Timeout waiting for $SERVICE_NAME to become active"
systemctl status $SERVICE_NAME || true
exit 1
"#
}

/// Script to pull the litehouse server container image
pub fn pull_container_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Pulling litehouse-server container image..."

# Run as litehouse user
sudo -u litehouse bash -c "
# Pull the latest image
docker pull ghcr.io/danbruder/litehouse:latest

echo 'Image pulled successfully'
"

echo "Container image pull completed"
"#,
        litehouse_uid
    )
}

/// Script to pull Caddy container image
pub fn pull_caddy_image_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Pulling Caddy container image..."

sudo -u litehouse bash -c "
docker pull docker.io/library/caddy:latest

echo 'Caddy image pulled successfully'
"

echo "Caddy container image pull completed"
"#,
        uid = litehouse_uid
    )
}

/// Script to pull Litestream container image
pub fn pull_litestream_image_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Pulling Litestream container image..."

sudo -u litehouse bash -c "
docker pull docker.io/litestream/litestream:latest

echo 'Litestream image pulled successfully'
"

echo "Litestream container image pull completed"
"#,
        uid = litehouse_uid
    )
}

/// Script to start Caddy container
pub fn start_caddy_container_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Starting Caddy container..."

sudo -u litehouse bash -c '
# Stop and remove any existing caddy container
docker stop caddy-container 2>/dev/null || true
docker rm caddy-container 2>/dev/null || true

# Create volumes if they do not exist
docker volume create caddy_data 2>/dev/null || true
docker volume create caddy_config 2>/dev/null || true

# Start Caddy container
docker run -d \
  --name caddy-container \
  --restart=unless-stopped \
  -p 80:80 \
  -p 443:443 \
  -p 2019:2019 \
  -v caddy_data:/data \
  -v caddy_config:/config \
  -e CADDY_ADMIN=0.0.0.0:2019 \
  --add-host=host.docker.internal:host-gateway \
  docker.io/library/caddy:latest \
  caddy run --resume

echo "Caddy container started"
'
"#,
        uid = litehouse_uid
    )
}

/// Script to start Litestream container
pub fn start_litestream_container_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Starting Litestream container..."

sudo -u litehouse bash -c '
# Stop and remove any existing litestream container
docker stop litestream-container 2>/dev/null || true
docker rm litestream-container 2>/dev/null || true

# Start Litestream container
docker run -d \
  --name litestream-container \
  --restart=unless-stopped \
  -v /opt/litehouse/data:/data \
  -v /opt/litehouse/config:/config \
  -v /opt/litehouse/data/litestream.yml:/etc/litestream.yml \
  docker.io/litestream/litestream:latest \
  replicate -config /etc/litestream.yml

echo "Litestream container started"
'
"#,
        uid = litehouse_uid
    )
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
                "upstreams": [{{ "dial": "host.containers.internal:3030" }}]
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

/// Generate initial Litestream configuration YAML
pub fn initial_litestream_config() -> &'static str {
    r#"dbs:
  - path: /config/litehouse.db
    replicas:
      - path: /data/litestream-replicas/main
"#
}
