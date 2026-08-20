#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
fixture_root=$(mktemp -d)
trap 'rm -rf "$fixture_root"' EXIT HUP INT TERM

mkdir -p "$fixture_root/bin" "$fixture_root/out"
printf '%s\n' '#!/bin/sh' 'printf "%s\\n" "bettr 9.9.9"' > "$fixture_root/bin/bettr"
chmod 755 "$fixture_root/bin/bettr"

"$repo_root/scripts/package-release.sh" 9.9.9 x86_64-unknown-linux-gnu \
  "$fixture_root/bin/bettr" "$fixture_root/out"

archive="$fixture_root/out/bettr-9.9.9-x86_64-unknown-linux-gnu.tar.gz"
checksum="$archive.sha256"
test -f "$archive"
test -f "$checksum"
grep -Eq '^[0-9a-f]{64}[[:space:]][[:space:]]bettr-9\.9\.9-x86_64-unknown-linux-gnu\.tar\.gz$' "$checksum"

"$repo_root/scripts/verify-release.sh" "$archive" "$checksum" 9.9.9

archive_listing=$(tar -tzf "$archive")
printf '%s\n' "$archive_listing" | grep -Fx 'bettr'
printf '%s\n' "$archive_listing" | grep -Fx 'LICENSE'

ci_workflow="$repo_root/.github/workflows/ci.yml"
test -f "$ci_workflow"
for required in \
  'pull_request:' \
  'push:' \
  'cargo fmt --all -- --check' \
  'cargo clippy --all-targets --all-features -- -D warnings' \
  'cargo test --locked' \
  'cargo build --locked --release'; do
  grep -Fq "$required" "$ci_workflow"
done
if grep -Fq 'contents: write' "$ci_workflow"; then
  printf 'CI workflow must not have contents: write\n' >&2
  exit 1
fi

release_workflow="$repo_root/.github/workflows/release.yml"
test -f "$release_workflow"
for required in \
  "tags:" \
  "v*" \
  "x86_64-unknown-linux-gnu" \
  "aarch64-unknown-linux-gnu" \
  "x86_64-apple-darwin" \
  "aarch64-apple-darwin" \
  "ubuntu-24.04" \
  "ubuntu-24.04-arm" \
  "macos-15-intel" \
  "macos-14" \
  "cargo metadata" \
  "cargo build --locked --release --target" \
  "scripts/package-release.sh" \
  "scripts/verify-release.sh" \
  "actions/attest@v4" \
  "actions/upload-artifact@v4" \
  "contents: write" \
  "--verify-tag" \
  "SHA256SUMS"; do
  grep -Fq -- "$required" "$release_workflow"
done

for required in \
  'SHA256SUMS' \
  'gh attestation verify' \
  '/releases/download/v' \
  '.prev' \
  'cargo install --path .' \
  'skill-installer/scripts/install-skill-from-github.py'; do
  grep -Fq -- "$required" "$repo_root/README.md"
done
