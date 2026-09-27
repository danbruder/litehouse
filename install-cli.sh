#!/bin/sh
# Install the litehouse CLI (`lh`) on a laptop, CI runner, or agent sandbox.
# Client only: no root, no Docker, no server setup (that's install.sh).
#
#   curl -fsSL https://raw.githubusercontent.com/danbruder/litehouse/main/install-cli.sh | sh
#
# Env:
#   LITEHOUSE_VERSION  release tag to install, e.g. v0.3.0 (default: latest)
#   LH_INSTALL_DIR     where to put `lh` (default: ~/.local/bin)
set -eu

REPO="danbruder/litehouse"
VERSION="${LITEHOUSE_VERSION:-latest}"
INSTALL_DIR="${LH_INSTALL_DIR:-$HOME/.local/bin}"

die() {
    printf 'install-cli: %s\n' "$1" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) OS=linux ;;
    Darwin) OS=darwin ;;
    *) die "unsupported OS $(uname -s): litehouse ships lh for Linux and macOS" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) die "unsupported CPU $(uname -m): litehouse ships lh for x86_64 and aarch64" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"

ASSET="litehouse-${OS}-${ARCH}.tar.gz"
if [ "$VERSION" = "latest" ]; then
    BASE="https://github.com/${REPO}/releases/latest/download"
else
    BASE="https://github.com/${REPO}/releases/download/${VERSION}"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

printf 'Downloading %s (%s)...\n' "$ASSET" "$VERSION"
curl -fsSL "$BASE/$ASSET" -o "$TMP/$ASSET" \
    || die "download failed: $BASE/$ASSET (does that release include $OS/$ARCH?)"

# Verify the checksum when the release publishes one and a tool is around.
if curl -fsSL "$BASE/$ASSET.sha256" -o "$TMP/$ASSET.sha256" 2>/dev/null; then
    expected="$(awk '{print $1}' "$TMP/$ASSET.sha256")"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$TMP/$ASSET" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$TMP/$ASSET" | awk '{print $1}')"
    else
        actual="$expected"
    fi
    [ "$expected" = "$actual" ] || die "checksum mismatch for $ASSET"
fi

tar -xzf "$TMP/$ASSET" -C "$TMP"
[ -f "$TMP/lh" ] || die "archive did not contain lh"

mkdir -p "$INSTALL_DIR"
mv "$TMP/lh" "$INSTALL_DIR/lh"
chmod 755 "$INSTALL_DIR/lh"

"$INSTALL_DIR/lh" --version >/dev/null 2>&1 \
    || die "installed $INSTALL_DIR/lh but it doesn't run on this machine"

printf 'Installed %s to %s/lh\n' "$("$INSTALL_DIR/lh" --version)" "$INSTALL_DIR"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\n%s is not on your PATH. Add it with:\n' "$INSTALL_DIR"
        printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        ;;
esac

cat <<'EOF'

Next:
  lh connect https://admin.<your-domain> --token <ADMIN_TOKEN>
  lh doctor                  # run from your app's repo
  lh agent-guide             # how agents deploy with litehouse
EOF
