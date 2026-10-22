# Iroh 1.3.0 compatibility patch

Source: crates.io iroh 1.3.0, archive SHA-256 `885787b892b5e2507c701f132ecbd45d2bad4bbb75157a19427087c16dadb833`.
Upstream BSD-3-Clause license is preserved in LICENSE-BSD3.

Local changes: src/net_report.rs plus src/net_report/fallback.rs. An optional
QUIC observer is tried only after the packet relay's discovery request fails or
returns a private/special-use address. A valid public primary result avoids the
outside request. Each attempt has a three-second timeout. Valid cached public
mappings are reused; unusable LAN mappings are retried on the next report. A LAN
observation survives an outside-service failure.

Observer latency is excluded from home-relay selection. Existing TLS verification,
peer authentication and UDP socket ownership are unchanged.

Regression coverage: crates/agent/src/relay_tests.rs runs the exact fallback
policy tests from fallback.rs and local TLS relay/QUIC observer fixtures. It checks
that primary failure falls back using the actual peer socket without changing the
home relay. Remove this patch when upstream supports ordered discovery fallback.
