#!/bin/sh
set -eu
cd "$(git rev-parse --show-toplevel)"
if { [ -n "${WIPSNET_ANDROID_LAN_ADDRESS:-}" ] || [ "${WIPSNET_ANDROID_WIFI_CYCLE:-0}" = 1 ]; } && [ "${WIPSNET_ANDROID_RELAY_TEST:-0}" != 1 ]; then
    echo "Android LAN/Wi-Fi checks require WIPSNET_ANDROID_RELAY_TEST=1." >&2
    exit 1
fi
if [ -n "${WIPSNET_SSH_RELAY_HOST:-}" ] && [ "${WIPSNET_ANDROID_RELAY_TEST:-0}" != 1 ]; then
    echo "Physical SSH/tablet checks require WIPSNET_ANDROID_RELAY_TEST=1 to rebuild the current Android CLI." >&2
    exit 1
fi
node --test sites/relay-compatibility/test/*.test.js
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo build -p wipsnet-cli --example relay_fixture --locked
python3 -m unittest discover -s scripts -p test_relay_network_audit.py
python3 scripts/smoke.py
python3 scripts/registry_smoke.py
python3 scripts/host_smoke.py
if [ "${WIPSNET_ANDROID_RELAY_TEST:-0}" = 1 ]; then
    set -- --android
    if [ -n "${WIPSNET_ANDROID_LAN_ADDRESS:-}" ]; then
        set -- "$@" --lan-address "$WIPSNET_ANDROID_LAN_ADDRESS" --lan-port "${WIPSNET_ANDROID_LAN_PORT:-0}"
    fi
    if [ "${WIPSNET_ANDROID_WIFI_CYCLE:-0}" = 1 ]; then
        set -- "$@" --wifi-cycle
    fi
    python3 scripts/relay_smoke.py "$@"
else
    python3 scripts/relay_smoke.py
fi
if [ -n "${WIPSNET_SSH_RELAY_HOST:-}" ]; then
    python3 scripts/physical_relay_smoke.py --ssh-host "$WIPSNET_SSH_RELAY_HOST"
fi
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
