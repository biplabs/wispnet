# Disposable browser compatibility spike (#2)

This is a local HTTP boundary experiment, not a production gateway. It deliberately
uses fixed loopback HTTP upstreams instead of Iroh. Do not deploy it or expose its
listeners publicly. The production Rust agent is unchanged.

Install uv and Python 3.11 or newer. Dependencies are pinned in pyproject.toml and
uv.lock. From the repository root:

    uv run --project spikes/browser --locked python -m unittest discover -s spikes/browser -p 'test_*.py' -v
    uv run --project spikes/browser --locked python spikes/browser/run.py

Optional existing applications:

    uv run --project spikes/browser --locked python spikes/browser/run.py --llama-port 8201 --website-port 8310

The script prints disposable browser links. Open one within 60 seconds. Each
session lasts at most 10 minutes; requests/streams are capped at 30 seconds in this
experiment. Restart for fresh links. The website/second fixtures have a Run
compatibility checks button for login, root-relative assets, cookies, SSE and
WebSocket echo. Absolute redirect tests exact-upstream Location rewriting. The
unrelated-site fixture has a Run cross-site probes button; expected backend delta
is zero. Fixture backends are intentionally accessible directly for the cookie
collision demonstration and must never contain private data.

## Real USB Android client

Authorize one device with adb. Add --phone to configure temporary reverse mappings:

    uv run --project spikes/browser --locked python spikes/browser/run.py --llama-port 8201 --website-port 8310 --phone

Open the printed phone_handoff URL in Android's ordinary browser, manually or with
an explicit adb browser-launch request. Open website and second and run their
checks. Reload the handoff page for fresh links before opening GIKE/llama. Fetch
the printed phone_reports URL from the development host to collect fixture
self-reports (including browser user agent, with no private application content).

USB reverse only maps the phone's loopback sockets to the development host. It
does not exercise Iroh, Wi-Fi, NAT, relays, cellular, or a native Android agent.
Mappings created by this process are removed on normal exit/SIGINT/SIGTERM.
A forced process kill or USB disconnect can prevent cleanup; list adb reverse
mappings and remove only the ports printed for this run. No device ID, token,
private content, or local model path belongs in committed evidence.

All listeners bind 127.0.0.1. .localhost service hostnames are generated randomly
per run. The capability handoff page expires after 10 minutes; keep its URL and
bootstrap links local. The helper exists only for this disposable USB test and
is not a proposed production administrative interface.

## Test boundaries

The automated tests assert that unauthorized Host/Origin/session/upgrade requests
cannot cause a backend request. They cover bootstrap replay, session expiry,
cookie collision across ports, host-only hostname separation, redirects, login,
SSE first-chunk delivery, disconnect cancellation and WebSocket text/binary data.

The prototype is intentionally limited: HTTP credentials lack Secure/__Host-
prefixes, no production policy/identity/registry authorization exists, cookie
Domain rewriting changes application behavior, external/embedded absolute URLs
are not rewritten, and resource limits are incomplete. See ADR 0001 and #10.
