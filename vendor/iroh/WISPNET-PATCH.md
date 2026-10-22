# Iroh 1.3.0 compatibility patch

Source: crates.io iroh 1.3.0, archive SHA-256 `885787b892b5e2507c701f132ecbd45d2bad4bbb75157a19427087c16dadb833`.
Upstream BSD-3-Clause license is preserved in LICENSE-BSD3.

Local changes: src/net_report.rs, src/net_report/fallback.rs, src/socket.rs and
src/socket/transports/relay/actor.rs. An optional
QUIC observer is tried only after the packet relay's discovery request fails or
returns a private/special-use address. A valid public primary result avoids the
outside request. Each attempt has a three-second timeout. Valid cached public
mappings are reused; unusable LAN mappings are retried on the next report. A LAN
observation survives an outside-service failure.

Observer latency is excluded from home-relay selection unless that URL is also
explicitly present in the packet RelayMap. Existing TLS verification,
peer authentication and UDP socket ownership are unchanged.

Regression coverage: crates/agent/src/relay_tests.rs runs the exact fallback
policy tests from fallback.rs and local TLS relay/QUIC observer fixtures. It checks
that primary failure falls back using the actual peer socket without changing the
home relay. Remove this patch when upstream supports ordered discovery fallback.

The optional `NetReportConfig::prefer_relay` policy carries an explicit primary
URL, failure grace and stable recovery window to the relay actor. Per-connection
health tracks the actual authenticated relay session lifetime, not QAD results.
Latency-based home changes are ignored only when this policy is enabled. The
primary stays connected/retrying while a backup is home, so recovery and flaps
are observed; other idle relay connections retain upstream cleanup behavior.
Wispnet configures five-second failure grace and 30-second stable failback.
Endpoint cancellation drains the same owned relay actor tasks as upstream.
The integration fixtures cover healthy primary without UDP discovery, primary
failure, brief recovery, stable failback, cold starts and unchanged approvals.

`NetReportConfig::keep_all_relays` enables parallel retained packet/QAD sessions.
The opt-in multi-observer implementation is in src/net_report/multi.rs. It keeps
one QUIC observation connection per relay/family and publishes all latest
addresses in Report::discovery_addresses; src/socket.rs advertises the distinct
candidate set. Initial observations are stored immediately for later reports.
The primary preference remains independent of UDP address-discovery success.

Additional changes: src/endpoint.rs exposes a read-only relay session snapshot;
src/net_report/report.rs retains per-observer addresses;
src/socket/biased_rtt_path_selector.rs ranks direct > preferred relay > backup;
src/socket/remote_map/remote_state.rs validates and retries alternate relay paths,
including after a relay-based handshake. This is opt-in and retains upstream
path selection for other endpoints. Existing peer authentication, TLS verification,
bounded candidate exchange and endpoint-owned task cancellation remain in force.
The active-session map is exposed through owner-local redacted diagnostics.
