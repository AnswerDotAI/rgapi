#!/bin/bash
set -e
profile=${1:-debug}
if [ "$profile" = "release" ]; then flags="--release"; else flags=""; fi
cargo build $flags
