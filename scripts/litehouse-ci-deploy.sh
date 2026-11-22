#!/bin/bash
# litehouse-ci-deploy: forced command for the CI deploy SSH key.
#
# Installed to /usr/local/bin/litehouse-ci-deploy by `lh install` and every
# `lh upgrade`. The CI key in authorized_keys is restricted to it:
#
#   command="/usr/local/bin/litehouse-ci-deploy",no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-pty ssh-ed25519 AAAA... litehouse-ci
#
# sshd puts whatever the client asked to run in $SSH_ORIGINAL_COMMAND. Two
# requests are honoured; nothing the client sends is ever executed directly:
#
#   deploy-commit <40-hex sha>  deploy ghcr.io/danbruder/litehouse:sha-<short>
#                               (binary + container) and verify it is running
#   anything else               `lh upgrade` to the latest tagged release,
#                               which is what this key always did before
#
# Both paths end by printing LITEHOUSE_DEPLOYED_IMAGE=<image of the running
# litehouse-server container>, which release.yml checks so a deploy that
# didn't land fails the workflow instead of passing green.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  exec sudo -n "$0" "$@"
fi

REPO_IMAGE="ghcr.io/danbruder/litehouse"
REQUEST="${SSH_ORIGINAL_COMMAND:-}"

running_image() {
  docker inspect litehouse-server --format '{{.Config.Image}}' 2>/dev/null || true
}

main() {
if [[ "$REQUEST" =~ deploy-commit[[:space:]]+([0-9a-f]{40}) ]]; then
  SHA="${BASH_REMATCH[1]}"
  TAG="sha-${SHA:0:7}"
  IMAGE="$REPO_IMAGE:$TAG"
  BIN="/tmp/lh-$TAG"

  echo "==> Deploying litehouse commit $SHA ($IMAGE)"
  docker pull "$IMAGE"
  CID="$(docker create "$IMAGE")"
  trap 'docker rm "$CID" >/dev/null 2>&1 || true; rm -f "$BIN"' EXIT
  docker cp "$CID:/usr/local/bin/lh" "$BIN"
  chmod +x "$BIN"
  "$BIN" upgrade --from-path "$BIN" --image-tag "$TAG"

  RUNNING="$(running_image)"
  echo "LITEHOUSE_DEPLOYED_IMAGE=$RUNNING"
  if [ "$RUNNING" != "$IMAGE" ]; then
    echo "ERROR: litehouse-server is running '$RUNNING', expected '$IMAGE'" >&2
    exit 1
  fi
  if ! docker ps --filter name=^litehouse-server$ --format '{{.Status}}' | grep -q '^Up'; then
    echo "ERROR: litehouse-server is not running after the deploy" >&2
    exit 1
  fi
  echo "==> litehouse-server is running $IMAGE"
else
  echo "==> Upgrading litehouse to the latest tagged release"
  /usr/local/bin/lh upgrade
  echo "LITEHOUSE_DEPLOYED_IMAGE=$(running_image)"
fi
}

# Parsed in full before it runs, because `lh upgrade` replaces this file
# while it is executing.
main "$@"
exit $?
