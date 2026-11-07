#!/usr/bin/env bash
# Package a built acidtrip binary for release.
#
#   scripts/package.sh <target> [binary]      (binary defaults to target/<target>/release/acidtrip)
#
# Writes to dist/:
#   acidtrip-<target>.tar.gz   the binary, plus the desktop launcher:
#                              Acidtrip.app (macOS) or share/ (Linux .desktop + icons)
#   Acidtrip-<target>.zip      macOS: the app bundle alone, to drag into Applications
#   acidtrip-<target>.AppImage Linux: when APPIMAGETOOL points at appimagetool
#                              (APPIMAGE_RUNTIME: runtime file for a cross target)
set -euo pipefail

target=${1:?usage: scripts/package.sh <target> [binary]}
bin=${2:-target/$target/release/acidtrip}
root=$(cd "$(dirname "$0")/.." && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
dist=$root/dist
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$dist"

cp "$bin" "$stage/acidtrip"
chmod +x "$stage/acidtrip"

case "$target" in
*apple-darwin)
  app=$stage/Acidtrip.app
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "$bin" "$app/Contents/MacOS/acidtrip"
  sed "s/@VERSION@/$version/g" "$root/packaging/macos/Info.plist" >"$app/Contents/Info.plist"
  cp "$root/packaging/icon/acidtrip.icns" "$app/Contents/Resources/acidtrip.icns"
  # Ad-hoc signature: Apple silicon refuses to run unsigned code at all.
  if command -v codesign >/dev/null; then codesign --force --deep --sign - "$app"; fi
  if command -v ditto >/dev/null; then
    (cd "$stage" && ditto -c -k --keepParent Acidtrip.app "$dist/Acidtrip-$target.zip")
  else
    (cd "$stage" && zip -qry "$dist/Acidtrip-$target.zip" Acidtrip.app)
  fi
  (cd "$stage" && tar czf "$dist/acidtrip-$target.tar.gz" acidtrip Acidtrip.app)
  ;;
*linux*)
  share=$stage/share
  install -Dm644 "$root/packaging/linux/acidtrip.desktop" "$share/applications/acidtrip.desktop"
  for s in 16 32 48 64 128 256 512; do
    install -Dm644 "$root/packaging/icon/acidtrip-$s.png" "$share/icons/hicolor/${s}x${s}/apps/acidtrip.png"
  done
  (cd "$stage" && tar czf "$dist/acidtrip-$target.tar.gz" acidtrip share)

  if [ -n "${APPIMAGETOOL:-}" ]; then
    arch=${target%%-*}
    appdir=$stage/AppDir
    install -Dm755 "$bin" "$appdir/usr/bin/acidtrip"
    cp -r "$share" "$appdir/usr/share"
    cp "$root/packaging/linux/acidtrip.desktop" "$appdir/acidtrip.desktop"
    cp "$root/packaging/icon/acidtrip-256.png" "$appdir/acidtrip.png"
    cat >"$appdir/AppRun" <<'RUN'
#!/bin/sh
# Started from a file manager or menu: open the window. Started with
# arguments (acidtrip mcp, acidtrip --help, a file in a terminal), pass them on.
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/acidtrip" "$@"
RUN
    chmod +x "$appdir/AppRun"
    runtime=()
    if [ -n "${APPIMAGE_RUNTIME:-}" ]; then runtime=(--runtime-file "$APPIMAGE_RUNTIME"); fi
    ARCH=$arch "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream "${runtime[@]}" \
      "$appdir" "$dist/acidtrip-$target.AppImage"
  fi
  ;;
*)
  echo "unknown target: $target" >&2
  exit 1
  ;;
esac

ls -la "$dist"
