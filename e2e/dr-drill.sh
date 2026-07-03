#!/usr/bin/env bash
# Disaster-recovery drill: wipe the node, reinstall fresh, restore from S3 backup,
# and confirm the previously-deployed app comes back up on its subdomain.
# No GitHub interaction — this assumes an app (e.g. hello.${DOMAIN}) was already
# deployed and backed up to S3 before this drill runs.
set -euo pipefail
: "${SERVER_IP:?e.g. 104.248.15.20}"
: "${DOMAIN:?e.g. s.danbruder.com}"
S3_ARGS="${S3_ARGS:-}"          # e.g. --s3-access-key .. --s3-secret-key .. --s3-bucket .. --s3-region ..
GHCR_ARGS="${GHCR_ARGS:-}"      # e.g. --ghcr-token ghp_...
LH="${LH:-cargo run --quiet --}" # local lh invocation

echo "==> 1/5 build release binary (musl)"
[ -n "${PREBUILT_LH:-}" ] || TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl
BIN="${PREBUILT_LH:-target/x86_64-unknown-linux-musl/release/lh}"

echo "==> 2/5 wipe node"
ssh "root@${SERVER_IP}" 'docker ps -aq | xargs -r docker rm -f; docker volume ls -q | xargs -r docker volume rm 2>/dev/null || true; rm -rf /opt/litehouse'

echo "==> 3/5 reinstall"
scp "$BIN" "root@${SERVER_IP}:/usr/local/bin/lh"
INSTALL_OUT=$(ssh "root@${SERVER_IP}" "lh install --domain ${DOMAIN} ${S3_ARGS} ${GHCR_ARGS}")
echo "$INSTALL_OUT"
TOKEN=$(echo "$INSTALL_OUT" | grep -oE -- '--token [a-f0-9]{64}' | awk '{print $2}' | tail -1)
[ -n "$TOKEN" ] || { echo "FATAL: no admin token in install output"; exit 1; }

echo "==> 4/5 connect CLI + restore from newest S3 backup"
$LH connect "https://admin.${DOMAIN}" --token "$TOKEN"
$LH restore --yes

echo "==> 5/5 poll until the restored app is reachable again"
for i in $(seq 1 30); do
  if curl -fsS --max-time 10 "https://hello.${DOMAIN}" | grep -q "hello from litehouse"; then
    echo "DR DRILL PASSED: https://hello.${DOMAIN} is live"; exit 0
  fi
  echo "  ... not live yet ($i/30)"; sleep 10
done
echo "FATAL: app never became reachable after restore"; exit 1
