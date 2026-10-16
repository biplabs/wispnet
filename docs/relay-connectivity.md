# Explicit relay connectivity (#13 checkpoint)

Known peers connect using a pinned EndpointId and one explicitly configured HTTPS
relay origin. A catalog, current peer IP, third machine or discovery database is
not required. This checkpoint implements the transport for host control/catalog
and services; target-bound pairing is still #17.

Add this section to both host and client configuration:

```toml
[transport]
bind_addr = "0.0.0.0:7779"
relay_url = "https://relay.wispnet.app"
relay_only = false
max_connections = 16
max_streams = 64
max_streams_per_connection = 8
```

The current temporary relay runs on desktop as a separate service; see the
[deployment and explicit TLS trust instructions](desktop-relay.md). Use your operated relay. Normal mode permits direct UDP/NAT paths
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
necessary. Android uses its OS address resolver without a Java VM; TXT peer
discovery is disabled, and no library-selected public DNS fallback is added. DNS uses the operating system's configured resolvers; library-selected
public fallback DNS servers are explicitly disabled. A system DNS outage can
therefore prevent hostname-based relay reconnection. No implicit public DNS
service is used to hide that failure.

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
- A real UDP DNS fixture changes `relay.test` from `127.0.0.1` to `127.0.0.2`
  with a one-second TTL and the same HTTPS port/hostname/certificate. Persistent
  host/client endpoints reconnect within a 30-second test deadline without
  resolver-cache resets, endpoint replacement, new pins or authority changes.
  DNS queries are checked to contain only the configured relay hostname.
- UDP-enabled endpoints cold-start against each host independently with the other
  host absent and a stale direct hint. The isolated audit repeats catalog and
  service byte checks with kernel UDP dropping, verifies nonzero drop counters
  and a selected relay path, and keeps identities/host approvals separate.
- A relay admission-denial fixture produces the distinct fixed diagnostic and
  never echoes its server-supplied sentinel reason.
- An untrusted relay certificate never brings the endpoint online (one-second
  observation); there is no insecure fallback.

A separate [native lifecycle harness](native-relay-validation.md) runs the real
Linux agent/CLI and authorized Android native CLI over a temporary USB relay
mapping. It verifies per-host approval, host/relay restart, wrong pins, independent
offline-peer failure and host-local persistent revocation. The ordinary hook runs
its Linux mode; hardware checks are opt-in. Its LAN extension verifies a physical Android direct
path and HTTP bytes, then direct → USB relay → direct through one persistent
endpoint with explicit Android network-change notifications; see
[peer diagnostics](peer-diagnostics.md).

These are local test deadlines, not internet recovery or availability guarantees.
#13 remains open for pairing integration (#17), physical two-host/tablet cold
starts, Wi-Fi/cellular/sleep transitions, and deployed relay replacement with
retained hostname/trust (#18). DNS replacement and destination blocking/auditing
now have isolated Linux fixture evidence, not deployed internet/mobile evidence.
No production migration is implemented.

## Isolated destination and UDP-block audit

On Linux with unprivileged user/network namespaces, `ip`, `nft` and `strace`:

```sh
python3 scripts/relay_network_audit.py
# Or include the audit in the normal tests and pre-push hook:
WIPSNET_NETWORK_AUDIT=1 ./scripts/test.sh
WIPSNET_NETWORK_AUDIT=1 git push origin main
```

For a locally unpacked strace binary, pass `--strace /absolute/path/strace`, or
set `WIPSNET_STRACE` alongside `WIPSNET_NETWORK_AUDIT=1` for the hook. Missing
dependencies or unsupported namespaces fail an explicitly requested audit.
The ordinary hook always runs the DNS/recovery and stale-hint Rust tests plus
eight audit-parser regression tests; isolated firewall checks are opt-in because
those Linux kernel/tool permissions are not available on every development host.

The runner builds fixtures first, then runs the DNS replacement, host control,
native CLI and forced-UDP cases in private network namespaces with only loopback
and default-drop nftables output policy. It changes no host routes, firewall,
interfaces or resolver files. Non-loopback Internet destinations cannot be reached;
a separate `strace -f -yy` audit rejects attempted TCP connections or addressed
packet sends outside loopback, including failed attempts. It reassembles
interleaved unfinished/resumed syscalls and fails incomplete/empty traces.
The UDP-block case drops all UDP before the loopback allow rule and requires
nonzero firewall packet counters while catalog/service bytes still succeed.

Pinned netdev performs no-packet UDP `connect` route probes to
`10.254.254.254:1` and `[fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff]:1`. The audit
accounts only for those exact datagram calls returning `ENETUNREACH`; it rejects
packet sends, TCP, other addresses/ports and successful calls under that exception.
These probes do not publish node state. Trace payload strings are suppressed and
temporary fixture traces are deleted after auditing. This validates the exercised
pinned native transport paths, not unimplemented Android/UI flows or every future
library feature.
