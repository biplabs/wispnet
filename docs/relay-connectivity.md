# Explicit relay connectivity (#13 checkpoint)

Known peers connect using a pinned EndpointId and one explicitly configured HTTPS
relay origin. A catalog, current peer IP, third machine or discovery database is
not required. This checkpoint implements the transport for host control/catalog
and services; target-bound pairing is still #17.

Add this section to both host and client configuration:

```toml
[transport]
bind_addr = "0.0.0.0:7779"
relay_url = "https://relay.wipsnet.app"
relay_only = false
max_connections = 16
max_streams = 64
max_streams_per_connection = 8
```

The hostname is the proposed deployment, not a provisioned service supplied by
this change. Use your operated relay. Normal mode permits direct UDP/NAT paths
and relay fallback. Choose a reachable bind address for direct connectivity.
`relay_only = true` removes all peer UDP transports, useful for restrictive
networks and testing. Relay TCP/TLS still needs network access. The config bind
address is unused in relay-only mode; startup reports `bind_addr: "relay-only"`.
Clients may use an ephemeral bind port. Host/client roles in one process share
one endpoint and key; CLI diagnostics acquire their own installation's exclusive
identity lease and cannot run concurrently with its daemon.

Only HTTPS origins without credentials, paths, queries or fragments are accepted.
Omitting `relay_url` preserves explicit direct-only development operation;
relay-only mode then fails configuration validation. Public WebPKI roots are
used by default. For a private relay CA, set an absolute `relay_ca_file` to a
PEM certificate (regular file, at most 16 KiB, one hard link, owned by this user
or root, not group/world writable). This replaces the relay trust roots; TLS
verification is always enabled. The relay's certificate must match its hostname.
No private keys or admission tokens belong in the relay URL.

```sh
target/debug/wipsnet host-catalog --config /absolute/path/client.toml --peer PINNED_HOST_ID
target/debug/wipsnet tunnel --config /absolute/path/client.toml --peer PINNED_HOST_ID --service llama
```

Both commands optionally accept `--address IP:UDP_PORT` immediately after the
peer ID, before `--service` if present. A single direct hint never substitutes
for the identity pin. Relay admission does not approve a host client or grant
service access. Existing host-local approvals and service grants remain required.

## Failure and recovery behavior

Agent startup does not await a relay or another host: local authority remains
available while offline. Pinned Iroh 1.3.0 maintains the outbound relay session
and retries relay failures with its own exponential backoff. Each peer connection
and catalog/service-open operation has a separate five-second deadline. Backend
connects have a three-second deadline; established service streams retain their
existing idle/session limits. Unrelated peer requests do not wait on a shared
all-peer bootstrap. No application service request is automatically replayed.

Connection failures report fixed, redacted diagnostics: relay admission denied,
relay unavailable with failed direct reachability, or control/service peer
connection failure. Relay status is a point-in-time observation, not proof that
the remote peer is offline. Host policy and backend statuses remain distinct
from transport errors. Raw relay error strings/reasons, payloads and credentials
are not surfaced. The minimal endpoint preset plus a custom relay map enables
no default public relays or DNS/Pkarr/DHT/gossip node address lookup/publication.
Ordinary DNS for a configured hostname and relay certificate validation remain
necessary. Packet-level destination auditing remains an acceptance task.

During a single-relay outage, an established usable direct connection can continue.
Relay-dependent new connections fail within their connection deadline. Existing
relay streams can stall or fail; seamless recovery of an application operation
is not promised. After relay recovery, callers can explicitly make a new request
with the same pinned identity. An uncertain write must be reconciled by the
application before retrying.

## Local evidence and remaining acceptance work

The normal local test hook runs actual pinned Iroh relay-server fixtures with
TLS and an explicitly trusted ephemeral certificate. Tests verify:

- Catalog and bidirectional service bytes through the selected relay path with
  **zero bound peer UDP sockets** and no direct hints.
- Native CLI catalog and stdin/stdout service commands without `--address`.
- Unapproved clients denied despite AllowAll relay admission; mismatched catalog
  identity rejected; an absent/wrong target pin fails within five seconds.
- An unavailable peer and a live catalog request operate independently.
- Relay shutdown prevents relay-only connections. A stateless relay restarted
  at the same address/certificate recovers within a 30-second test deadline,
  preserving endpoint IDs and host grants, with no backend operation replay.
- A selected direct loopback path continues after relay shutdown.
- An untrusted relay certificate never brings the endpoint online (one-second
  observation); there is no insecure fallback.

These are local test deadlines, not internet recovery or availability guarantees.
#13 remains open for pairing integration (#17), physical two-host/tablet cold
starts, Wi-Fi/cellular/sleep transitions, changed DNS IP under the same hostname,
packet-level public-destination blocking/audit, and deployed relay replacement
with retained hostname/trust (#18). No production migration is implemented.
