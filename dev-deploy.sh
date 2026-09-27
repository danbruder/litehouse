#!/bin/bash
set -e

SERVER="${SERVER:-root@104.248.15.20}"

# Build for the server's CPU (x86_64 or aarch64).
ARCH="$(ssh "$SERVER" uname -m)"
case "$ARCH" in
    x86_64|amd64) ARCH=x86_64 ;;
    aarch64|arm64) ARCH=aarch64 ;;
    *) echo "Unsupported server architecture: $ARCH" >&2; exit 1 ;;
esac
TARGET="$ARCH-unknown-linux-musl"

echo "==> Building static $ARCH binary..."
TARGET_CC="$ARCH-linux-musl-gcc" cargo build --release --target "$TARGET"

echo "==> Uploading to $SERVER..."
scp "target/$TARGET/release/lh" "$SERVER":/tmp/lh

echo "==> Running upgrade on $SERVER..."
ssh "$SERVER" "sudo /tmp/lh upgrade --from-path /tmp/lh"

echo "==> Done!"
