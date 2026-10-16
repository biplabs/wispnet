#!/bin/sh
set -eu
cd "$(git rev-parse --show-toplevel)"
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke.py
uv run --project spikes/browser --locked python -m unittest discover -s spikes/browser -p 'test_*.py'
