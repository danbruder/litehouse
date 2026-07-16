/// Template for server configuration file. `admin_token_hash` is the
/// sha256 hex hash of the freshly generated admin token (see
/// `crate::auth::generate_token`/`hash_token`) — the plaintext token itself
/// is never written to disk, only printed once at the end of install.
pub fn server_config_template(
    domain: &str,
    admin_token_hash: &str,
    admin_subdomain: Option<&str>,
) -> String {
    let mut config = format!(
        r#"host = "0.0.0.0"
port = 3030
caddy_http_port = 80
caddy_https_port = 443
domain = "{}"
admin_token_hash = "{}"
"#,
        domain, admin_token_hash
    );

    if let Some(sub) = admin_subdomain {
        config.push_str(&format!("admin_subdomain = \"{}\"\n", sub));
    }

    config
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

/// systemd timer unit for the hourly dial-stdio cleanup. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §2.
pub fn dial_stdio_cleanup_timer() -> &'static str {
    r#"[Unit]
Description=Hourly cleanup of orphaned docker dial-stdio helper processes

[Timer]
OnCalendar=hourly
Persistent=true

[Install]
WantedBy=timers.target
"#
}

/// systemd service unit for the hourly dial-stdio cleanup. Intentionally
/// blunt: it does not distinguish orphaned dial-stdio processes from ones
/// serving an active `docker` CLI command, since litehouse's production
/// code never shells out to the `docker` CLI. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §2.
pub fn dial_stdio_cleanup_service() -> &'static str {
    r#"[Unit]
Description=Kill orphaned docker system dial-stdio helper processes

[Service]
Type=oneshot
ExecStart=/usr/bin/pkill -f 'docker system dial-stdio'
# pkill exits 1 when no matching process is found - that's the common
# case (nothing to clean up), not a failure.
SuccessExitStatus=0 1
"#
}

/// systemd timer unit for the weekly host reboot. Matches the "3am US
/// Eastern" convention already established by the nightly app-restart
/// feature; systemd resolves the trailing timezone against tzdata
/// (DST-safe) without changing the host's system timezone. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §3.
pub fn weekly_reboot_timer() -> &'static str {
    r#"[Unit]
Description=Weekly host reboot to bound dial-stdio process accumulation

[Timer]
OnCalendar=Sun *-*-* 03:00:00 America/New_York
Persistent=true

[Install]
WantedBy=timers.target
"#
}

/// systemd service unit for the weekly host reboot. Safe because every
/// container (litehouse-server, caddy, and every app) runs with
/// --restart=unless-stopped/always, so a reboot is self-healing. See
/// docs/superpowers/specs/2026-07-16-server-hardening-design.md §3.
pub fn weekly_reboot_service() -> &'static str {
    r#"[Unit]
Description=Weekly host reboot

[Service]
Type=oneshot
ExecStart=/usr/bin/systemctl reboot
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

# Small droplets (1GB) wedge hard under memory pressure without swap: the
# kernel stays up but every userspace process (sshd included) stalls. 2GB of
# swap lets the box degrade gracefully instead. Idempotent.
if ! swapon --show | grep -q /swapfile; then
    echo "Provisioning 2GB swapfile..."
    fallocate -l 2G /swapfile
    chmod 600 /swapfile
    mkswap /swapfile
    swapon /swapfile
    grep -q '/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
    # Prefer reclaiming page cache over aggressively swapping processes
    sysctl -w vm.swappiness=10
    grep -q 'vm.swappiness' /etc/sysctl.conf || echo 'vm.swappiness=10' >> /etc/sysctl.conf
else
    echo "Swapfile already active"
fi

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

# Create group if it doesn't exist. Don't demand a specific GID — stock
# cloud images (e.g. Ubuntu with a default 'ubuntu' user) already occupy 1000.
if ! getent group litehouse > /dev/null 2>&1; then
    groupadd litehouse
    echo "Created litehouse group (GID $(getent group litehouse | cut -d: -f3))"
else
    echo "Group litehouse already exists"
fi

# Create user if it doesn't exist; let the system pick a free UID.
if ! id -u litehouse > /dev/null 2>&1; then
    useradd -r -m -d /opt/litehouse -s /bin/bash -g litehouse litehouse
    echo "Created litehouse user (UID $(id -u litehouse))"
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

# --- Firewall guardrail for published container ports ------------------------
# Docker publishes container ports through its own iptables DOCKER chain, which
# is evaluated BEFORE ufw's INPUT rules. That means a container port published
# to 0.0.0.0 is reachable from the internet regardless of the ufw rules set in
# the security-hardening phase (which only allow 22/80/443). In the litehouse
# model the ONLY port that must be public is Caddy's 80/443; the Caddy admin
# API (2019) and all app containers are reached over the internal bridge only.
#
# Defense-in-depth: explicitly drop WAN access to the Caddy admin API at the
# DOCKER-USER hook, so even a stray/legacy port publish can never expose it.
# (Kept targeted rather than a blanket default-drop because apps may legitimately
# publish their own ports — e.g. a WebRTC UDP media range — which a blanket rule
# would silently break.)
WAN_IF=$(ip route get 1.1.1.1 2>/dev/null | sed -n 's/.* dev \([^ ]*\).*/\1/p' | head -1)
WAN_IF=${WAN_IF:-eth0}
{
  iptables -nL DOCKER-USER >/dev/null 2>&1 || iptables -N DOCKER-USER
  iptables -C DOCKER-USER -i "$WAN_IF" -p tcp --dport 2019 -j DROP 2>/dev/null \
    || iptables -I DOCKER-USER 1 -i "$WAN_IF" -p tcp --dport 2019 -j DROP
  # Persist across reboots.
  if command -v netfilter-persistent >/dev/null 2>&1; then
    netfilter-persistent save
  else
    mkdir -p /etc/iptables && iptables-save > /etc/iptables/rules.v4
  fi
  echo "DOCKER-USER guard for Caddy admin port installed on $WAN_IF"
} || echo "Warning: could not install DOCKER-USER firewall guard (continuing)"

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
#
# Lock the top-level dir to 0700 root:root. The snapshots staged here are
# plaintext copies of the state DB (S3 creds, GHCR token, admin-token hash)
# and per-app data, so they must NOT be readable by other local host users.
# litehouse-server runs as root inside its container and reaches this dir via
# a bind mount (root bypasses the mode), and the snapshot containers get their
# per-app subdir bind-mounted directly, so 0700 on this parent blocks local
# traversal without breaking either writer.
mkdir -p /opt/litehouse/backups
chmod 700 /opt/litehouse/backups
chown root:root /opt/litehouse/backups

# Seed server-config.toml into the (empty) Docker volume on first boot only.
# This container is (re)started on every `lh upgrade`/deploy, not just on
# first install — copying unconditionally here would clobber any change made
# to the live volume config since install (e.g. an admin token rotation)
# with the stale host-side snapshot, silently reverting it on every deploy.
# Once the volume has a config file, it — not the host file — is the source
# of truth.
echo "Seeding server-config.toml into Docker volume (first boot only)..."
if [ -f /opt/litehouse/config/server-config.toml ]; then
  docker run --rm \
    -v litehouse_config:/target \
    -v /opt/litehouse/config/server-config.toml:/source/server-config.toml:ro \
    alpine:3.20 \
    sh -c 'if [ ! -f /target/server-config.toml ]; then cp /source/server-config.toml /target/server-config.toml; echo "Server config seeded from host (volume was empty)"; else echo "Server config already present in volume, leaving it in place"; fi'
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
  -p 127.0.0.1:3030:3030 \
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
pub fn initial_caddy_config(domain: &str, admin_label: &str) -> String {
    format!(
        r#"{{
  "apps": {{
    "http": {{
      "servers": {{
        "app_proxy": {{
          "listen": [":80", ":443"],
          "routes": [
            {{
              "match": [{{ "host": ["{admin_label}.{domain}"] }}],
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
        domain = domain,
        admin_label = admin_label
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
    fn litehouse_run_script_seeds_config_volume_only_if_empty() {
        // The container is (re)started on every `lh upgrade`/deploy, not just
        // on first install. If the copy runs unconditionally, it clobbers
        // any change made to the live volume config since install (e.g. an
        // admin token rotation) with the stale host-side snapshot, silently
        // invalidating the current admin token on every deploy.
        let script = start_litehouse_container_script("1000", "1.2.3");
        assert!(
            script.contains("if [ ! -f /target/server-config.toml ]"),
            "container start script must only seed the config volume when it's empty, \
             not overwrite it unconditionally on every restart/upgrade"
        );
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
        let config = server_config_template("example.com", "deadbeef", None);
        assert!(config.contains("admin_token_hash = \"deadbeef\""));
        assert!(!config.contains("plaintext"));
        assert!(!config.contains("admin_subdomain"));
    }

    #[test]
    fn server_config_template_includes_custom_admin_subdomain() {
        let config = server_config_template("example.com", "deadbeef", Some("admin2"));
        assert!(config.contains("admin_subdomain = \"admin2\""));
    }

    #[test]
    fn dial_stdio_cleanup_timer_runs_hourly() {
        let timer = dial_stdio_cleanup_timer();
        assert!(timer.contains("OnCalendar=hourly"));
        assert!(timer.contains("[Timer]"));
    }

    #[test]
    fn dial_stdio_cleanup_service_kills_orphaned_dial_stdio_processes() {
        let service = dial_stdio_cleanup_service();
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("ExecStart=/usr/bin/pkill -f 'docker system dial-stdio'"));
    }

    #[test]
    fn weekly_reboot_timer_runs_sunday_3am_eastern() {
        let timer = weekly_reboot_timer();
        assert!(timer.contains("OnCalendar=Sun *-*-* 03:00:00 America/New_York"));
        assert!(timer.contains("[Timer]"));
    }

    #[test]
    fn weekly_reboot_service_reboots_the_host() {
        let service = weekly_reboot_service();
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("ExecStart=/usr/bin/systemctl reboot"));
    }
}
