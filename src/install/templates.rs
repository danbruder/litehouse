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
pub fn build_litehouse_image_script(_litehouse_uid: &str) -> String {
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

# Build the image
docker build -t litehouse:latest $BUILD_DIR

echo "Container image build completed"
"#.to_string()
}

/// Script to pull Caddy container image
pub fn pull_caddy_image_script(_litehouse_uid: &str) -> String {
    r#"#!/bin/bash
set -e

echo "Pulling Caddy container image..."

docker pull caddy:latest

echo "Caddy container image pull completed"
"#
    .to_string()
}

/// Script to start litehouse-server container
pub fn start_litehouse_container_script(_litehouse_uid: &str) -> String {
    r#"#!/bin/bash
set -e

echo "Starting litehouse-server container..."

# Stop and remove any existing container
docker stop litehouse-server 2>/dev/null || true
docker rm litehouse-server 2>/dev/null || true

# Create the litehouse network if it doesn't exist
docker network create litehouse-network 2>/dev/null || true

# Create Docker volumes if they don't exist
docker volume create litehouse_config 2>/dev/null || true
docker volume create litehouse_data 2>/dev/null || true

# Fix volume ownership for litehouse user (UID 1000)
# Docker creates volumes as root by default, but container runs as litehouse (UID 1000)
echo "Setting correct ownership on Docker volumes..."
docker run --rm \
  -v litehouse_config:/config \
  -v litehouse_data:/data \
  alpine:latest \
  sh -c 'chown -R 1000:1000 /config /data && chmod 755 /config /data'

# Copy server-config.toml from host into the Docker volume
# This ensures the container sees the production configuration
echo "Copying server-config.toml into Docker volume..."
if [ -f /opt/litehouse/config/server-config.toml ]; then
  docker run --rm \
    -v litehouse_config:/target \
    -v /opt/litehouse/config/server-config.toml:/source/server-config.toml:ro \
    alpine:latest \
    sh -c 'cp /source/server-config.toml /target/server-config.toml && chown 1000:1000 /target/server-config.toml'
  echo "Server config copied successfully"
else
  echo "Warning: /opt/litehouse/config/server-config.toml not found, container will use defaults"
fi

# Copy S3 credentials if they exist
if [ -f /opt/litehouse/config/s3-credentials.env ]; then
  echo "Copying S3 credentials into Docker volume..."
  docker run --rm \
    -v litehouse_config:/target \
    -v /opt/litehouse/config/s3-credentials.env:/source/s3-credentials.env:ro \
    alpine:latest \
    sh -c 'cp /source/s3-credentials.env /target/s3-credentials.env && chown 1000:1000 /target/s3-credentials.env && chmod 600 /target/s3-credentials.env'
  echo "S3 credentials copied successfully"
fi

# Load S3 credentials from file if it exists in the volume
S3_ENV_ARGS=""
if docker run --rm -v litehouse_config:/config alpine:latest test -f /config/s3-credentials.env 2>/dev/null; then
  echo "Loading S3 credentials from configuration file..."
  # Extract S3 vars from the file and pass them as -e flags
  S3_ENV_ARGS=$(docker run --rm -v litehouse_config:/config alpine:latest cat /config/s3-credentials.env 2>/dev/null | \
    awk 'NF && !/^#/ {print "-e " $0}' | tr '\n' ' ')
fi

# Get Docker socket group ID for permissions
DOCKER_GID=$(stat -c '%g' /var/run/docker.sock)

# Start litehouse-server container with restart policy
# Use Docker volumes instead of bind mounts to avoid permission issues
# shellcheck disable=SC2086
docker run -d \
  --name litehouse-server \
  --restart=unless-stopped \
  --network litehouse-network \
  -v litehouse_config:/opt/litehouse/config \
  -v litehouse_data:/opt/litehouse/data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  --group-add "$DOCKER_GID" \
  -e DATABASE_URL=/opt/litehouse/config/litehouse.db \
  -e LITEHOUSE_DIR=/opt/litehouse \
  -e DOCKER_HOST=unix:///var/run/docker.sock \
  -e CADDY_API_URL=http://caddy-container:2019/load \
  -e RUST_LOG=info \
  $S3_ENV_ARGS \
  litehouse:latest

echo "litehouse-server container started"
"#
    .to_string()
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
