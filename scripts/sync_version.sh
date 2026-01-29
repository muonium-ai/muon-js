#!/bin/sh
set -e

VERSION_FILE="${1:-VERSION}"
TOML_FILE="${2:-Cargo.toml}"

if [ ! -f "$VERSION_FILE" ]; then
  echo "VERSION file not found: $VERSION_FILE" >&2
  exit 1
fi

version=$(tr -d '[:space:]' < "$VERSION_FILE")
if [ -z "$version" ]; then
  echo "VERSION file is empty" >&2
  exit 1
fi

perl -0pi -e "s/^version = \"[^\"]+\"/version = \"$version\"/m" "$TOML_FILE"
