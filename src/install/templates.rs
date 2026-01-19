/// Template for server configuration file
pub fn server_config_template(domain: &str) -> String {
    format!(
        r#"host = "0.0.0.0"
port = 3030
caddy_http_port = 80
caddy_https_port = 443
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
    podman \
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

echo "Configuring subuid/subgid for rootless Podman..."
# Check if litehouse already has subuid/subgid mappings
if ! grep -q "^litehouse:" /etc/subuid; then
    echo "litehouse:100000:65536" >> /etc/subuid
    echo "Added subuid mapping for litehouse"
fi

if ! grep -q "^litehouse:" /etc/subgid; then
    echo "litehouse:100000:65536" >> /etc/subgid
    echo "Added subgid mapping for litehouse"
fi

echo "User and directory setup completed successfully"
"#
}

/// Script for Podman configuration
pub fn podman_setup_script() -> &'static str {
    r#"#!/bin/bash
set -e

echo "Configuring Podman for litehouse user..."

# Get litehouse user UID
LITEHOUSE_UID=$(id -u litehouse)

# Configure system to allow binding to privileged ports (< 1024) for rootless containers
echo "Configuring unprivileged port access for rootless containers..."
if ! grep -q "net.ipv4.ip_unprivileged_port_start" /etc/sysctl.conf; then
    echo "net.ipv4.ip_unprivileged_port_start=80" >> /etc/sysctl.conf
    sysctl -p /etc/sysctl.conf
    echo "Configured unprivileged port start to 80"
else
    echo "Unprivileged port configuration already exists"
fi

# Enable user lingering (allows user services to run without login)
loginctl enable-linger litehouse

# Wait a moment for the runtime directory to be created
sleep 2

# Verify runtime directory exists
RUNTIME_DIR="/run/user/${LITEHOUSE_UID}"
if [ ! -d "$RUNTIME_DIR" ]; then
    echo "Creating runtime directory: $RUNTIME_DIR"
    mkdir -p "$RUNTIME_DIR"
    chown litehouse:litehouse "$RUNTIME_DIR"
    chmod 700 "$RUNTIME_DIR"
fi

# Set proper environment and enable Podman socket as litehouse user
cd /tmp && sudo -u litehouse bash -c "
export XDG_RUNTIME_DIR=/run/user/${LITEHOUSE_UID}
export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${LITEHOUSE_UID}/bus

# Try to start systemd user session if not running
systemctl --user status > /dev/null 2>&1 || {
    echo 'Starting systemd user session...'
}

# Enable and start podman socket
systemctl --user enable podman.socket
systemctl --user start podman.socket
"

# Verify socket exists
SOCKET_PATH="/run/user/${LITEHOUSE_UID}/podman/podman.sock"
MAX_WAIT=10
WAITED=0

while [ $WAITED -lt $MAX_WAIT ]; do
    if [ -S "$SOCKET_PATH" ]; then
        echo "Podman socket verified at $SOCKET_PATH"
        break
    fi
    echo "Waiting for Podman socket... ($WAITED/$MAX_WAIT)"
    sleep 1
    WAITED=$((WAITED + 1))
done

if [ ! -S "$SOCKET_PATH" ]; then
    echo "Error: Failed to create Podman socket after ${MAX_WAIT} seconds"
    echo "Attempting to debug..."
    ls -la "$RUNTIME_DIR" || echo "Runtime dir doesn't exist"
    cd /tmp && sudo -u litehouse bash -c "XDG_RUNTIME_DIR=/run/user/${LITEHOUSE_UID} systemctl --user status podman.socket" || echo "Failed to get status"
    exit 1
fi

echo "Podman configuration completed successfully"
echo "UID:$LITEHOUSE_UID"
"#
}

/// Dockerfile template for building litehouse image locally
pub fn local_dockerfile_template() -> &'static str {
    r#"FROM alpine:latest
RUN apk add --no-cache ca-certificates git
RUN addgroup -g 1000 litehouse && adduser -D -u 1000 -G litehouse litehouse
RUN mkdir -p /opt/litehouse/config /opt/litehouse/data && chown -R litehouse:litehouse /opt/litehouse
COPY lh /usr/local/bin/lh
RUN chmod +x /usr/local/bin/lh
WORKDIR /opt/litehouse
USER litehouse
EXPOSE 3030
CMD ["lh", "serve"]
"#
}

/// Script to build the litehouse server container image locally
pub fn build_litehouse_image_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Building litehouse-server container image locally..."

# Create build context directory with world-readable permissions
BUILD_DIR=$(mktemp -d)
chmod 755 "$BUILD_DIR"
trap "rm -rf $BUILD_DIR" EXIT

# Copy binary to build context
cp /usr/local/bin/lh "$BUILD_DIR/lh"
chmod 755 "$BUILD_DIR/lh"

# Write Dockerfile
cat > "$BUILD_DIR/Dockerfile" << 'DOCKERFILE'
FROM alpine:latest
RUN apk add --no-cache ca-certificates git
RUN addgroup -g 1000 litehouse && adduser -D -u 1000 -G litehouse litehouse
RUN mkdir -p /opt/litehouse/config /opt/litehouse/data && chown -R litehouse:litehouse /opt/litehouse
COPY lh /usr/local/bin/lh
RUN chmod +x /usr/local/bin/lh
WORKDIR /opt/litehouse
USER litehouse
EXPOSE 3030
CMD ["lh", "serve"]
DOCKERFILE
chmod 644 "$BUILD_DIR/Dockerfile"

# Run as litehouse user with proper environment
cd /tmp && sudo -u litehouse bash -c "
export XDG_RUNTIME_DIR=/run/user/{uid}
export PODMAN_SOCK=/run/user/{uid}/podman/podman.sock

# Build the image
podman build -t litehouse:latest $BUILD_DIR

echo 'Image built successfully'
"

echo "Container image build completed"
"#,
        uid = litehouse_uid
    )
}

/// Script to pull Caddy container image
pub fn pull_caddy_image_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Pulling Caddy container image..."

cd /tmp && sudo -u litehouse bash -c "
export XDG_RUNTIME_DIR=/run/user/{uid}
export PODMAN_SOCK=/run/user/{uid}/podman/podman.sock

podman pull docker.io/library/caddy:latest

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

cd /tmp && sudo -u litehouse bash -c "
export XDG_RUNTIME_DIR=/run/user/{uid}
export PODMAN_SOCK=/run/user/{uid}/podman/podman.sock

podman pull docker.io/litestream/litestream:latest

echo 'Litestream image pulled successfully'
"

echo "Litestream container image pull completed"
"#,
        uid = litehouse_uid
    )
}

/// Script to start litehouse-server container
pub fn start_litehouse_container_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Starting litehouse-server container..."

cd /tmp && sudo -u litehouse bash -c '
export XDG_RUNTIME_DIR=/run/user/{uid}
export PODMAN_SOCK=/run/user/{uid}/podman/podman.sock

# Stop and remove any existing container
podman stop --time 0 -i litehouse-server 2>/dev/null || true
podman rm -f litehouse-server 2>/dev/null || true

# Start litehouse-server container with restart policy
podman run -d \
  --name litehouse-server \
  --restart=unless-stopped \
  --replace \
  --userns=keep-id \
  -p 3030:3030 \
  -v /opt/litehouse/config:/opt/litehouse/config \
  -v /opt/litehouse/data:/opt/litehouse/data \
  -v /run/user/{uid}/podman/podman.sock:/run/podman/podman.sock \
  -e DATABASE_URL=/opt/litehouse/config/litehouse.db \
  -e LITEHOUSE_DIR=/opt/litehouse \
  -e PODMAN_SOCK=/run/podman/podman.sock \
  -e RUST_LOG=info \
  localhost/litehouse:latest

echo "litehouse-server container started"
'
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

cd /tmp && sudo -u litehouse bash -c '
export XDG_RUNTIME_DIR=/run/user/{uid}
export PODMAN_SOCK=/run/user/{uid}/podman/podman.sock

# Stop and remove any existing caddy container
podman stop -i caddy-container 2>/dev/null || true
podman rm -i caddy-container 2>/dev/null || true

# Create volumes if they do not exist
podman volume create caddy_data 2>/dev/null || true
podman volume create caddy_config 2>/dev/null || true

# Start Caddy container
podman run -d \
  --name caddy-container \
  --restart=unless-stopped \
  --replace \
  -p 80:80 \
  -p 443:443 \
  -p 2019:2019 \
  -v caddy_data:/data \
  -v caddy_config:/config \
  -e CADDY_ADMIN=0.0.0.0:2019 \
  --add-host=host.containers.internal:host-gateway \
  docker.io/library/caddy:latest \
  caddy run --resume

echo "Caddy container started"
'
"#,
        uid = litehouse_uid
    )
}

/// Script to enable podman-restart service
pub fn enable_podman_restart_script(litehouse_uid: &str) -> String {
    format!(
        r#"#!/bin/bash
set -e

echo "Enabling podman-restart.service..."

cd /tmp && sudo -u litehouse bash -c '
export XDG_RUNTIME_DIR=/run/user/{uid}
systemctl --user enable podman-restart.service
'

echo "podman-restart.service enabled"
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
