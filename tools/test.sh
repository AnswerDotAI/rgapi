#!/bin/bash
set -e
cargo test
pytest -q
