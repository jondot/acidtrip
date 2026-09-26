# Releasing acidtrip

acidtrip ships prebuilt binaries on GitHub Releases: a cosign-signed `tar.gz` per
target. `install.sh` downloads these.

## Cutting a release

```sh
./scripts/release.sh 0.2.0
```

This needs a clean tree on `main`. It bumps the workspace `version` in
`Cargo.toml`, updates the workspace crates in `Cargo.lock`, commits
`release v0.2.0`, tags `v0.2.0`, and pushes. Pushing the `v*` tag starts
`.github/workflows/release.yml`.

## What the Release workflow does

1. **build** runs one job per target:

   | Target | Runner |
   |--------|--------|
   | `x86_64-apple-darwin` | `macos-latest` |
   | `aarch64-apple-darwin` | `macos-latest` |
   | `x86_64-unknown-linux-gnu` | `ubuntu-latest` |
   | `aarch64-unknown-linux-gnu` | `ubuntu-latest` (cross via `gcc-aarch64-linux-gnu`) |

   Each job produces `acidtrip-<target>.tar.gz`.

2. **release** signs every `tar.gz` with cosign. Signing is keyless through sigstore and needs `id-token: write`. It writes a `.sig` and a `.crt` for each archive. It then creates the GitHub Release with generated notes and uploads the archives and signatures.

No secrets are needed: `GITHUB_TOKEN` is provided automatically, and the
workflow requests `contents: write` and `id-token: write`.

## CI

`.github/workflows/ci.yml` runs clippy and the test suite (unit tests, the
end-to-end scenarios and the user flows, all through the headless harness) on
every push to `main` and every pull request.

## Not included

- **Package registries.** Nothing is published to crates.io, npm or Homebrew.
  The binaries on GitHub Releases are the distribution.
- **Windows.** There is no Windows build yet.
- **Apple notarization.** The archives are signed with cosign, not notarized by
  Apple. That needs a Developer ID certificate. `install.sh` downloads with `curl`,
  so Gatekeeper's quarantine doesn't apply.
