#!/usr/bin/env bash
# Screenshots of the real app for the website.
#
# Builds acidtrip and the harness, runs every website/shots/*.at scenario in
# a pseudo-terminal, and copies the PNGs each scenario names into
# website/public/shots/. A scenario's `shot NAME` becomes public/shots/NAME.png,
# so name shots after what they show and only `shot` the ones the site uses.
#
#   scripts/shots.sh              # all scenarios
#   scripts/shots.sh gradient     # just shots/gradient.at
#
# Each scenario runs in a scratch directory holding copies of shots/art/ and
# the repo's test corpus (so nothing the app saves lands in the repo), with a
# fresh ACIDTRIP_HOME whose font library links to yours, so TheDraw fonts
# work. Get them once with `acidtrip fonts get`.
set -euo pipefail

SITE="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$SITE/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$REPO/target}"
export CARGO_TARGET_DIR="$TARGET"
BIN="$TARGET/debug/acidtrip"
HARNESS="$TARGET/debug/acidtrip-harness"
OUT="$SITE/public/shots"
COLS="${COLS:-120}"
ROWS="${ROWS:-40}"

(cd "$REPO" && cargo build -q -p acidtrip -p acidtrip-harness)

# Your TheDraw fonts, if you have them.
FONTS="$("$BIN" paths | awk '$1 == "fonts" { sub(/^fonts +/, ""); print }')"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/acidtrip-shots.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$OUT"

scenarios=()
if [ $# -gt 0 ]; then
  for n in "$@"; do scenarios+=("$SITE/shots/${n%.at}.at"); done
else
  scenarios=("$SITE"/shots/*.at)
fi

failed=0
for at in "${scenarios[@]}"; do
  name="$(basename "$at" .at)"
  dir="$WORK/$name"
  home="$dir/home"
  mkdir -p "$dir/files" "$home/data/library"
  cp -R "$SITE/shots/art/." "$dir/files/"
  cp -R "$REPO/crates/acidtrip-io/tests/corpus/." "$dir/files/"
  cp "$REPO"/tests/e2e/fake/*.json "$dir/files/"
  if [ -n "$FONTS" ] && [ -d "$FONTS" ]; then ln -s "$FONTS" "$home/data/library/fonts"; fi
  echo "── $name"
  if (cd "$dir/files" && "$HARNESS" run "$at" --bin "$BIN" --home "$home" --out "$dir/shots" \
        --cols "$COLS" --rows "$ROWS" --timeout 10s --quiet); then
    for png in "$dir"/shots/*.png; do
      [ -e "$png" ] || continue
      case "$(basename "$png")" in FAILURE.png) continue ;; esac
      cp "$png" "$OUT/"
      echo "   $(basename "$png")"
    done
  else
    echo "   FAILED (see $dir/shots/FAILURE.png)" >&2
    trap - EXIT
    failed=1
  fi
done
exit $failed
