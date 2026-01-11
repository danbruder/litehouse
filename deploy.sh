#! /bin/bash

set -e

# echo "Building the frontend..."
# cd assets && npm run build && cd ..

echo "Building the backend..."
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl

echo "Pushing..."
scp ./target/x86_64-unknown-linux-musl/release/lh litehouse@litehouse:/opt/litehouse/lh.new

echo "Deploying..."
ssh root@litehouse -t 'systemctl stop litehouse'
ssh litehouse@litehouse -t 'mv /opt/litehouse/lh.new /opt/litehouse/lh'
ssh root@litehouse -t 'systemctl restart litehouse'

echo "Done!"
