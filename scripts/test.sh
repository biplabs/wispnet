#!/bin/sh
set -eu
cd "$(git rev-parse --show-toplevel)"
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 -m unittest discover -s scripts -p test_relay_network_audit.py
python3 scripts/smoke.py
python3 scripts/registry_smoke.py
python3 scripts/host_smoke.py
python3 scripts/tunnel_smoke.py
uv run --project spikes/browser --locked python -m unittest discover -s spikes/browser -p 'test_*.py'

# Opt in on Linux hosts with namespaces, nftables and strace. An explicitly
# requested network audit fails if unavailable; it is never silently skipped.
if [ "${WIPSNET_NETWORK_AUDIT:-0}" = 1 ]; then
    if [ -n "${WIPSNET_STRACE:-}" ]; then
        python3 scripts/relay_network_audit.py --strace "$WIPSNET_STRACE"
    else
        python3 scripts/relay_network_audit.py
    fi
fi
