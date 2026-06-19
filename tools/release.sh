#!/bin/bash
set -e
v=$(grep '^version = ' pyproject.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
git tag "v$v"
git push origin main --tags
echo "Released v$v"
