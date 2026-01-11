/// Template for the systemd service file
pub fn systemd_service_template(litehouse_uid: &str) -> String {
    format!(
        r#"[Unit]
Description=Litehouse Application Platform
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=litehouse
Group=litehouse
WorkingDirectory=/opt/litehouse
ExecStart=/opt/litehouse/lh serve
Restart=always
RestartSec=10

# Environment
Environment="DATABASE_URL=/opt/litehouse/config/litehouse.db"
Environment="LITEHOUSE_DIR=/opt/litehouse"
Environment="PODMAN_SOCK=/run/user/{}/podman/podman.sock"
Environment="RUST_LOG=info"

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/litehouse
ReadWritePaths=/run/user/{}/podman

# Allow binding to ports 80 and 443
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
"#,
        litehouse_uid, litehouse_uid
    )
}

/// Template for server configuration file
pub fn server_config_template(domain: &str) -> String {
    format!(
        r#"host = "0.0.0.0"
proxy_host = "0.0.0.0"
proxy_port = 80
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
    groupadd litehouse
    echo "Created litehouse group"
else
    echo "Group litehouse already exists"
fi

# Create user if it doesn't exist
if ! id -u litehouse > /dev/null 2>&1; then
    useradd -r -m -d /opt/litehouse -s /bin/bash -g litehouse litehouse
    echo "Created litehouse user"
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

/// Script for Podman configuration
pub fn podman_setup_script() -> &'static str {
    r#"#!/bin/bash
set -e

echo "Configuring Podman for litehouse user..."

# Enable user lingering (allows user services to run without login)
loginctl enable-linger litehouse

# Enable and start Podman socket as litehouse user
sudo -u litehouse bash -c 'XDG_RUNTIME_DIR=/run/user/$(id -u) systemctl --user enable podman.socket'
sudo -u litehouse bash -c 'XDG_RUNTIME_DIR=/run/user/$(id -u) systemctl --user start podman.socket'

# Get litehouse user UID
LITEHOUSE_UID=$(id -u litehouse)

# Verify socket exists
SOCKET_PATH="/run/user/${LITEHOUSE_UID}/podman/podman.sock"
if [ -S "$SOCKET_PATH" ]; then
    echo "Podman socket verified at $SOCKET_PATH"
else
    echo "Warning: Podman socket not found at expected path: $SOCKET_PATH"
    echo "Attempting to restart podman socket..."
    sudo -u litehouse bash -c "XDG_RUNTIME_DIR=/run/user/${LITEHOUSE_UID} systemctl --user restart podman.socket"
    sleep 2
    if [ -S "$SOCKET_PATH" ]; then
        echo "Podman socket created successfully"
    else
        echo "Error: Failed to create Podman socket"
        exit 1
    fi
fi

echo "Podman configuration completed successfully"
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
