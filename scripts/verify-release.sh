#!/bin/sh
set -eu

if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
  printf 'usage: %s ARCHIVE CHECKSUM_FILE EXPECTED_VERSION [--skip-binary-execution]\n' "$0" >&2
  exit 2
fi

archive=$1
checksum_file=$2
expected_version=$3
execution_option=${4:-}
archive_name=$(basename -- "$archive")
target=${archive_name#bettr-$expected_version-}
target=${target%.tar.gz}

if [ -n "$execution_option" ] && [ "$execution_option" != '--skip-binary-execution' ]; then
  printf 'unknown option: %s\n' "$execution_option" >&2
  exit 2
fi

test -f "$archive"
test -f "$checksum_file"

checksum_asset=$(awk 'NF >= 2 { print $2; exit }' "$checksum_file")
expected_digest=$(awk 'NF >= 1 { print $1; exit }' "$checksum_file")
test "$checksum_asset" = "$archive_name"
test -n "$expected_digest"

if command -v sha256sum >/dev/null 2>&1; then
  actual_digest=$(sha256sum "$archive" | awk '{ print $1 }')
else
  actual_digest=$(shasum -a 256 "$archive" | awk '{ print $1 }')
fi
test "$actual_digest" = "$expected_digest"

extract_dir=$(mktemp -d "${TMPDIR:-/tmp}/bettr-verify.XXXXXX")
trap 'rm -rf "$extract_dir"' EXIT HUP INT TERM
tar -xzf "$archive" -C "$extract_dir"
test -x "$extract_dir/bettr"
test -f "$extract_dir/LICENSE"
test -f "$extract_dir/manifest.json"
test -f "$extract_dir/skills/bettr/SKILL.md"
test -f "$extract_dir/skills/bettr-claude/SKILL.md"
grep -Fq '"format_version": 1' "$extract_dir/manifest.json"
grep -Fq "\"version\": \"$expected_version\"" "$extract_dir/manifest.json"
grep -Fq "\"target\": \"$target\"" "$extract_dir/manifest.json"
grep -Eq '"revision": "[^" ]+"' "$extract_dir/manifest.json"

if [ "$execution_option" = '--skip-binary-execution' ]; then
  printf 'verified %s (%s; binary execution skipped)\n' "$archive_name" "$expected_version"
  exit 0
fi

version_output=$($extract_dir/bettr --version)
case "$version_output" in
  *"$expected_version"*) ;;
  *)
    printf 'binary version does not contain %s: %s\n' "$expected_version" "$version_output" >&2
    exit 1
    ;;
esac

printf 'verified %s (%s)\n' "$archive_name" "$expected_version"
