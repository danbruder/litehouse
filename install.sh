#!/bin/sh
set -e

# Litehouse Installation Script
# Usage: curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install.sh | sh -s -- --domain lh.example.com

VERSION="${LITEHOUSE_VERSION:-latest}"
GITHUB_REPO="danbruder/litehouse"

# Colors for output (if terminal supports it)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    NC='\033[0m' # No Color
else
    RED=''
    GREEN=''
    YELLOW=''
    NC=''
fi

info() {
    printf "${GREEN}[INFO]${NC} %s\n" "$1"
}

warn() {
    printf "${YELLOW}[WARN]${NC} %s\n" "$1"
}

error() {
    printf "${RED}[ERROR]${NC} %s\n" "$1" >&2
}

die() {
    error "$1"
    exit 1
}

# Parse arguments
DOMAIN=""
SKIP_VERIFY=""

while [ $# -gt 0 ]; do
    case "$1" in
        --domain)
            DOMAIN="$2"
            shift 2
            ;;
        --domain=*)
            DOMAIN="${1#*=}"
            shift
            ;;
        --skip-verify)
            SKIP_VERIFY="--skip-verify"
            shift
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --version=*)
            VERSION="${1#*=}"
            shift
            ;;
        --help|-h)
            cat <<EOF
Litehouse Installation Script

Usage:
  curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install.sh | sh -s -- --domain <domain>

Options:
  --domain <domain>    Base domain for wildcard routing (e.g., lh.example.com) [required]
  --skip-verify        Skip the final verification step
  --version <version>  Specific version to install (default: latest)
  --help, -h           Show this help message

Example:
  curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install.sh | sh -s -- --domain lh.example.com
EOF
            exit 0
            ;;
        *)
            die "Unknown option: $1. Use --help for usage."
            ;;
    esac
done

# Validate required arguments
if [ -z "$DOMAIN" ]; then
    die "Missing required argument: --domain. Use --help for usage."
fi

# Validate domain format
case "$DOMAIN" in
    *.*)
        # Contains at least one dot, looks like a domain
        ;;
    *)
        die "Invalid domain format: $DOMAIN. Expected format: subdomain.example.com"
        ;;
esac

info "Litehouse Installer"
info "==================="
info "Domain: $DOMAIN"
info "Version: $VERSION"
echo ""

# Check OS
OS=$(uname -s)
if [ "$OS" != "Linux" ]; then
    die "Unsupported operating system: $OS. Only Linux is supported."
fi

# Check architecture
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64)
        ARCH="x86_64"
        ;;
    aarch64|arm64)
        ARCH="aarch64"
        ;;
    *)
        die "Unsupported architecture: $ARCH. Only x86_64 and aarch64 are supported."
        ;;
esac

info "Detected: Linux $ARCH"

# Check if running as root
if [ "$(id -u)" -ne 0 ]; then
    die "This script must be run as root. Try: sudo sh -c 'curl -fsSL ... | sh -s -- --domain $DOMAIN'"
fi

# Check for required commands
for cmd in curl tar; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        die "Required command not found: $cmd. Please install it first."
    fi
done

# Determine download URL
if [ "$VERSION" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/latest/download/litehouse-linux-${ARCH}.tar.gz"
else
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/litehouse-linux-${ARCH}.tar.gz"
fi

# Create temp directory
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

info "Downloading litehouse binary from $DOWNLOAD_URL..."
if ! curl -fsSL "$DOWNLOAD_URL" -o "$TEMP_DIR/litehouse.tar.gz"; then
    die "Failed to download litehouse binary. Please check if the release exists."
fi

info "Extracting binary..."
tar -xzf "$TEMP_DIR/litehouse.tar.gz" -C "$TEMP_DIR"

# Find the binary (it might be named differently or in a subdirectory)
BINARY_PATH=$(find "$TEMP_DIR" -name "lh" -o -name "litehouse" | head -1)
if [ -z "$BINARY_PATH" ]; then
    # Try looking for any executable
    BINARY_PATH=$(find "$TEMP_DIR" -type f -executable | head -1)
fi

if [ -z "$BINARY_PATH" ]; then
    die "Could not find litehouse binary in downloaded archive."
fi

# Install binary to /usr/local/bin
info "Installing litehouse to /usr/local/bin/lh..."
cp "$BINARY_PATH" /usr/local/bin/lh
chmod +x /usr/local/bin/lh

# Verify binary works
if ! /usr/local/bin/lh --version >/dev/null 2>&1; then
    die "Failed to execute installed binary. The binary may be incompatible with this system."
fi

INSTALLED_VERSION=$(/usr/local/bin/lh --version 2>&1 || echo "unknown")
info "Installed: $INSTALLED_VERSION"
echo ""

# Run the install command
info "Running litehouse install..."
echo ""

exec /usr/local/bin/lh install --domain "$DOMAIN" $SKIP_VERIFY
