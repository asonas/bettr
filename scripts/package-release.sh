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
mkdir -p "$output_dir"

archive_name="bettr-$version-$target.tar.gz"
archive_path="$output_dir/$archive_name"
staging_dir=$(mktemp -d "${TMPDIR:-/tmp}/bettr-package.XXXXXX")
trap 'rm -rf "$staging_dir"' EXIT HUP INT TERM

cp "$binary" "$staging_dir/bettr"
chmod 755 "$staging_dir/bettr"
cp "$repo_root/LICENSE" "$staging_dir/LICENSE"
tar -C "$staging_dir" -czf "$archive_path" bettr LICENSE

if command -v sha256sum >/dev/null 2>&1; then
  (CDPATH= cd -- "$output_dir" && sha256sum "$archive_name" > "$archive_name.sha256")
else
  (CDPATH= cd -- "$output_dir" && shasum -a 256 "$archive_name" > "$archive_name.sha256")
fi

printf '%s\n' "$archive_path"
