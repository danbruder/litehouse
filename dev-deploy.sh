#!/bin/bash
set -e

SERVER="${SERVER:-root@104.248.15.20}"

echo "==> Building static binary..."
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl

echo "==> Uploading to $SERVER..."
scp target/x86_64-unknown-linux-musl/release/lh "$SERVER":/tmp/lh

echo "==> Running upgrade on $SERVER..."
ssh "$SERVER" "sudo /tmp/lh upgrade --from-path /tmp/lh"

echo "==> Done!"
