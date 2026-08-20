#!/bin/sh
set -eu

if [ "$#" -ne 4 ]; then
  printf 'usage: %s VERSION TARGET BINARY OUTPUT_DIR\n' "$0" >&2
  exit 2
fi

version=$1
target=$2
binary=$3
output_dir=$4
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

test -n "$version"
test -n "$target"
test -f "$binary"
test -x "$binary"
test -f "$repo_root/LICENSE"
test -f "$repo_root/skills/bettr/SKILL.md"
test -f "$repo_root/skills/bettr-claude/SKILL.md"
mkdir -p "$output_dir"

archive_name="bettr-$version-$target.tar.gz"
archive_path="$output_dir/$archive_name"
staging_dir=$(mktemp -d "${TMPDIR:-/tmp}/bettr-package.XXXXXX")
trap 'rm -rf "$staging_dir"' EXIT HUP INT TERM

cp "$binary" "$staging_dir/bettr"
chmod 755 "$staging_dir/bettr"
cp "$repo_root/LICENSE" "$staging_dir/LICENSE"
mkdir -p "$staging_dir/skills"
cp -R "$repo_root/skills/bettr" "$staging_dir/skills/bettr"
cp -R "$repo_root/skills/bettr-claude" "$staging_dir/skills/bettr-claude"
revision=${RELEASE_REVISION:-}
if [ -z "$revision" ]; then
  revision=$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf '%s' unknown)
fi
printf '{\n  "format_version": 1,\n  "version": "%s",\n  "target": "%s",\n  "revision": "%s"\n}\n' \
  "$version" "$target" "$revision" > "$staging_dir/manifest.json"
tar -C "$staging_dir" -czf "$archive_path" bettr LICENSE manifest.json skills

if command -v sha256sum >/dev/null 2>&1; then
  (CDPATH= cd -- "$output_dir" && sha256sum "$archive_name" > "$archive_name.sha256")
else
  (CDPATH= cd -- "$output_dir" && shasum -a 256 "$archive_name" > "$archive_name.sha256")
fi

printf '%s\n' "$archive_path"
