#!/bin/bash
set -e

SERVER="${SERVER:-root@104.248.15.20}"
CONFIG_PATH="/var/lib/docker/volumes/litehouse_config/_data/server-config.toml"

if [[ "$1" != "-y" && "$1" != "--yes" ]]; then
  read -p "This invalidates the current admin token on $SERVER. Continue? [y/N] " confirm
  if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
    echo "Aborted."
    exit 1
  fi
fi

echo "==> Generating new token..."
NEW_TOKEN=$(openssl rand -hex 32)
NEW_HASH=$(printf '%s' "$NEW_TOKEN" | shasum -a 256 | awk '{print $1}')

echo "==> Writing new hash to $SERVER:$CONFIG_PATH..."
ssh "$SERVER" "sed -i 's/^admin_token_hash = .*/admin_token_hash = \"$NEW_HASH\"/' $CONFIG_PATH"

echo "==> Restarting litehouse-server..."
ssh "$SERVER" "docker restart litehouse-server >/dev/null"

echo "==> Waiting for server to come back up..."
for i in $(seq 1 15); do
  CODE=$(ssh "$SERVER" "curl -s -o /dev/null -w '%{http_code}' -H 'Authorization: Bearer $NEW_TOKEN' http://localhost:3030/api/apps" || echo "000")
  if [[ "$CODE" == "200" ]]; then
    echo "==> New token verified (HTTP 200)."
    break
  fi
  if [[ "$i" == "15" ]]; then
    echo "New token did not verify after restart (last status: $CODE)." >&2
    exit 1
  fi
  sleep 1
done

DOMAIN=$(ssh "$SERVER" "grep '^domain' $CONFIG_PATH | cut -d'\"' -f2")
ADMIN_SUBDOMAIN=$(ssh "$SERVER" "grep '^admin_subdomain' $CONFIG_PATH | cut -d'\"' -f2")
ADMIN_SUBDOMAIN="${ADMIN_SUBDOMAIN:-admin}"
BASE_URL="http://$ADMIN_SUBDOMAIN.$DOMAIN"

echo "==> New admin token: $NEW_TOKEN"

if command -v lh >/dev/null 2>&1; then
  echo "==> Reconnecting local CLI to $BASE_URL..."
  lh connect "$BASE_URL" --token "$NEW_TOKEN"
else
  echo "==> lh CLI not found on PATH; connect manually with:"
  echo "    lh connect $BASE_URL --token $NEW_TOKEN"
fi

echo "==> Done! Save the token above somewhere durable — it cannot be recovered again."
