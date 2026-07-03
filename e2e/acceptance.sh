#!/usr/bin/env bash
# End-to-end acceptance: fresh DO node -> installed server -> app live on a subdomain.
# Prereqs: wildcard DNS *.${DOMAIN} -> ${SERVER_IP}; gh CLI authed; local musl toolchain OR PREBUILT_LH pointing at a linux binary.
set -euo pipefail
: "${SERVER_IP:?e.g. 104.248.15.20}"
: "${DOMAIN:?e.g. s.danbruder.com}"
: "${HELLO_REPO:?e.g. danbruder/litehouse-hello — created if missing}"
S3_ARGS="${S3_ARGS:-}"          # e.g. --s3-access-key .. --s3-secret-key .. --s3-bucket .. --s3-region ..
GHCR_ARGS="${GHCR_ARGS:-}"      # e.g. --ghcr-token ghp_...
LH="${LH:-cargo run --quiet --}" # local lh invocation

echo "==> 1/7 build release binary (musl)"
[ -n "${PREBUILT_LH:-}" ] || TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl
BIN="${PREBUILT_LH:-target/x86_64-unknown-linux-musl/release/lh}"

echo "==> 2/7 wipe node"
ssh "root@${SERVER_IP}" 'docker ps -aq | xargs -r docker rm -f; docker volume ls -q | xargs -r docker volume rm 2>/dev/null || true; rm -rf /opt/litehouse'

echo "==> 3/7 install"
scp "$BIN" "root@${SERVER_IP}:/usr/local/bin/lh"
INSTALL_OUT=$(ssh "root@${SERVER_IP}" "lh install --domain ${DOMAIN} ${S3_ARGS} ${GHCR_ARGS}")
echo "$INSTALL_OUT"
TOKEN=$(echo "$INSTALL_OUT" | grep -oE -- '--token [a-f0-9]{64}' | awk '{print $2}' | tail -1)
[ -n "$TOKEN" ] || { echo "FATAL: no admin token in install output"; exit 1; }

echo "==> 4/7 connect CLI + smoke"
$LH connect "https://admin.${DOMAIN}" --token "$TOKEN"
$LH status

echo "==> 5/7 ensure hello repo exists and is current"
if ! gh repo view "$HELLO_REPO" >/dev/null 2>&1; then gh repo create "$HELLO_REPO" --private; fi
TMP=$(mktemp -d)
cp -r examples/hello/. "$TMP"
(cd "$TMP" && git init -qb main && git add -A && git commit -qm "hello" \
  && git remote add origin "https://github.com/${HELLO_REPO}.git" && git push -qf origin main)

echo "==> 6/7 create app (drunk-proof moment)"
$LH create hello --repo "$HELLO_REPO"

echo "==> 7/7 trigger deploy and wait"
# `lh create` above pushed a workflow commit straight to the repo, so pull
# before pushing again to avoid a non-fast-forward rejection.
(cd "$TMP" && git pull --rebase origin main && git commit -qm "deploy $(date +%s)" --allow-empty && git push -q origin main)
$LH deploys hello --wait --timeout 600
for i in $(seq 1 30); do
  if curl -fsS --max-time 10 "https://hello.${DOMAIN}" | grep -q "hello from litehouse"; then
    echo "ACCEPTANCE PASSED: https://hello.${DOMAIN} is live"; exit 0
  fi
  echo "  ... not live yet ($i/30)"; sleep 10
done
echo "FATAL: app never became reachable"; exit 1
