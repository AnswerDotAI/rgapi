#!/bin/bash
set -e
cur=$(grep '^version = ' pyproject.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
new=$(echo "$cur" | awk -F. '{print $1"."$2"."$3+1}')
sed -i '' "s/^version = \"$cur\"/version = \"$new\"/" pyproject.toml Cargo.toml
echo "$cur -> $new"
