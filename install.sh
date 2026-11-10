#!/bin/sh
# install.sh — install acidtrip
#
# Usage: curl -fsSL https://raw.githubusercontent.com/jondot/acidtrip/main/install.sh | sh
#    or: curl -fsSL https://raw.githubusercontent.com/jondot/acidtrip/main/install.sh | sh -s -- --version v0.1.0
#
# Also adds an app launcher (Acidtrip.app in ~/Applications on macOS, a menu
# entry on Linux) when the release has one; --no-app skips it.
set -e

REPO="jondot/acidtrip"
BINARY="acidtrip"
VERSION=""
APP=1

# Parse arguments
while [ $# -gt 0 ]; do
  case "$1" in
    --version|-v) VERSION="$2"; shift 2 ;;
    --no-app) APP=0; shift ;;
    *) shift ;;
  esac
done

# Detect OS and arch
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) OS="apple-darwin" ;;
  linux)  OS="unknown-linux-gnu" ;;
  *) echo "Error: Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *) echo "Error: Unsupported architecture: $ARCH"; exit 1 ;;
esac

TARGET="${ARCH}-${OS}"

# Get version (latest if not specified)
if [ -z "$VERSION" ]; then
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | sed -E 's/.*"([^"]+)".*/\1/')
  if [ -z "$VERSION" ]; then
    echo "Error: Failed to determine latest release version"
    exit 1
  fi
fi

URL="https://github.com/${REPO}/releases/download/${VERSION}/${BINARY}-${TARGET}.tar.gz"

echo "Installing ${BINARY} ${VERSION} for ${TARGET}..."

# Download and extract into a temp dir
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

HTTP_CODE=$(curl -fsSL -w "%{http_code}" -o "$TMPDIR/${BINARY}.tar.gz" "$URL" 2>/dev/null) || true
if [ "$HTTP_CODE" != "200" ] && [ ! -s "$TMPDIR/${BINARY}.tar.gz" ]; then
  echo "Error: Failed to download ${URL}"
  echo "HTTP status: ${HTTP_CODE}"
  echo "Check that release ${VERSION} exists and has a ${TARGET} asset."
  exit 1
fi

tar xzf "$TMPDIR/${BINARY}.tar.gz" -C "$TMPDIR"

# Determine install directory
# INSTALL_DIR wins if set; else /usr/local/bin if writable, else ~/.local/bin
if [ -n "$INSTALL_DIR" ]; then
  mkdir -p "$INSTALL_DIR"
else
  INSTALL_DIR="/usr/local/bin"
  if [ ! -w "$INSTALL_DIR" ] 2>/dev/null; then
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "$INSTALL_DIR"
  fi
fi

mv "$TMPDIR/$BINARY" "$INSTALL_DIR/$BINARY"
chmod +x "$INSTALL_DIR/$BINARY"

echo ""
echo "  Installed ${BINARY} ${VERSION} to ${INSTALL_DIR}/${BINARY}"
echo ""

# App launcher: start acidtrip from an icon, in its own window.
if [ "$APP" = 1 ] && [ -d "$TMPDIR/Acidtrip.app" ]; then
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/Acidtrip.app"
  mv "$TMPDIR/Acidtrip.app" "$HOME/Applications/Acidtrip.app"
  echo "  Added Acidtrip.app to ~/Applications"
  echo ""
elif [ "$APP" = 1 ] && [ -d "$TMPDIR/share/applications" ]; then
  DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
  mkdir -p "$DATA/applications"
  # Menus don't always see ~/.local/bin on PATH: point at the binary itself.
  sed "s|^Exec=acidtrip|Exec=${INSTALL_DIR}/${BINARY}|" "$TMPDIR/share/applications/acidtrip.desktop" \
    > "$DATA/applications/acidtrip.desktop"
  for d in "$TMPDIR"/share/icons/hicolor/*/apps; do
    size=$(basename "$(dirname "$d")")
    mkdir -p "$DATA/icons/hicolor/$size/apps"
    cp "$d/acidtrip.png" "$DATA/icons/hicolor/$size/apps/acidtrip.png"
  done
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$DATA/applications" || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$DATA/icons/hicolor" || true
  echo "  Added acidtrip to your applications menu"
  echo ""
fi

# Remind the user if the install dir isn't in PATH
case ":$PATH:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo "  Note: ${INSTALL_DIR} is not in your PATH."
    echo "  Add it with:"
    echo ""
    echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
    ;;
esac
