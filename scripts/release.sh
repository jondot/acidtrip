#!/bin/sh
# Usage: ./scripts/release.sh <version>
# Example: ./scripts/release.sh 0.2.0
set -e

VERSION="${1#v}"

if [ -z "$VERSION" ]; then
  echo "usage: $0 <version>  (e.g. 0.2.0)"
  exit 1
fi

if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "error: version must be X.Y.Z, got: $VERSION"
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "error: working tree is dirty — commit or stash changes first"
  exit 1
fi

if [ "$(git rev-parse --abbrev-ref HEAD)" != "main" ]; then
  echo "error: must be on main branch"
  exit 1
fi

git pull --ff-only origin main

# Bump Cargo.toml (first version field = the [package] version)
sed -i '' "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
cargo update --workspace --quiet

git add Cargo.toml Cargo.lock
git commit -m "release v$VERSION"
git tag "v$VERSION"
git push origin main "v$VERSION"

echo ""
echo "  v$VERSION tagged and pushed — release workflow started"
echo "  https://github.com/jondot/acidtrip/actions"
echo ""
