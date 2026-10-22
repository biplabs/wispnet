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
[deployment and TLS instructions](desktop-relay.md). Use your operated relay. Normal mode permits direct UDP/NAT paths
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
target/debug/wispnet host-catalog --config /absolute/path/client.toml --peer PINNED_HOST_ID
target/debug/wispnet tunnel --config /absolute/path/client.toml --peer PINNED_HOST_ID --service llama
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
WISPNET_NETWORK_AUDIT=1 ./scripts/test.sh
WISPNET_NETWORK_AUDIT=1 git push origin main
```

For a locally unpacked strace binary, pass `--strace /absolute/path/strace`, or
set `WISPNET_STRACE` alongside `WISPNET_NETWORK_AUDIT=1` for the hook. Missing
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

## Public UDP address discovery

The standalone relay can optionally run Iroh QUIC address discovery on UDP 7842.
This lets endpoints learn their public UDP addresses and attempt direct NAT
traversal. Forwarding HTTPS alone provides relay traffic but does not provide
this discovery service. Enabling discovery does not guarantee a direct path on
every carrier/router combination.

For the existing Caddy-fronted deployment:

```sh
python3 scripts/install_relay_discovery.py --ssh-host desktop.home
cargo run -p wispnet-agent --example relay_discovery_probe --locked
```

Forward external TCP 443 to the host's TCP 8443 and external UDP 7842 to the
host's UDP 7842. The installer preserves the loopback HTTP packet relay and uses
Caddy's certificate for authenticated UDP discovery. A systemd path watcher
restarts the relay when Caddy renews the certificate/key. The public gateway
installer preserves these discovery settings on subsequent deployments.

The relay's opt-in settings are:

```toml
quic_listen = "0.0.0.0:7842"
quic_certificate = "/absolute/caddy/relay.wispnet.app.crt"
quic_private_key = "/absolute/caddy/relay.wispnet.app.key"
```

Private-key protection and certificate matching use the same checks as the TLS
packet relay. HTTP-only mode requires an explicit complete discovery certificate
and key. In a relay terminating its own TLS, discovery may inherit that TLS
configuration instead. Omitting `quic_listen` leaves UDP discovery disabled.
A discovery probe verifies the TLS certificate and an actual observed-address
response; a UDP listener or HTTPS 200 by itself is not proof of discovery.

## Optional outside-NAT address discovery

For a relay hosted behind the same router as an endpoint, explicitly configure
an outside observer in that endpoint's `[transport]` section:

```toml
relay_url = "https://relay.wispnet.app"
discovery_relay_url = "https://usw1-1.relay.n0.iroh.link"
```

This is opt-in. Fresh discovery tries the primary relay first. A timeout,
connection/DNS failure, or a private/special-use observed address triggers the
outside observer on UDP 7842 through the endpoint's same socket. A usable public
primary result does not contact the outside observer. Valid cached mappings are
reused until a full refresh or network change; an unusable cached LAN result is
retried on the next report. If both attempts fail, any LAN observation remains
available for local connectivity.

The primary relay remains the invitation/bootstrap URL and a permitted packet relay. The outside
observer cannot become the home relay unless it is also explicitly configured as a packet relay. The local compatibility patch in
`vendor/iroh` keeps discovery fallback separate from relay selection. No DNS/Pkarr
peer lookup or publication is enabled. Certificate trust applies to both servers.
n0's public service is rate-limited and has no uptime guarantee. Omit
`discovery_relay_url` to restore the original single-relay behavior.

The Android app enables the same ordered discovery policy at native initialization:
`relay.wispnet.app` first, then `usw1-1.relay.n0.iroh.link` when required. This needs
an updated APK because both the native transport and its initialization setting
are bundled in the app. Existing device identity and pairings are retained by an
in-place update.

## Redundant packet relays

Configure the same relay set on every paired node, including Android:

```toml
[transport]
bind_addr = "0.0.0.0:0"
relay_url = "https://relay.wispnet.app"
backup_relay_urls = ["https://usw1-1.relay.n0.iroh.link"]
relay_only = false
```

The primary URL remains the invitation/bootstrap URL. All configured packet relay
URLs are included as candidates when dialing a pinned peer, so the primary is
not needed to locate a peer that has moved to a backup. Existing pairings and
host-local approvals remain valid. Both endpoints need this configuration and
updated software; older clients still know only the original URL. A discovery-only
observer is never implicitly promoted to a packet relay.

Wispnet explicitly prefers `relay_url` as its home packet relay whenever its
TLS-authenticated relay connection works. UDP discovery failure, a private
observed address, and a faster n0 discovery response do not cause packet failover.
After five seconds of detected primary unavailability (including startup dial
failure), it selects a backup. Socket failure detection and reconnect timeouts
can add time to recovery; five seconds is a grace period, not an outage SLA.

While on a working backup it continues reconnecting to the primary and requires
30 seconds of uninterrupted authenticated primary health before failing back.
Any primary disconnect resets that window. If the backup also fails and the
primary already works, it may recover to the primary immediately. Healthy-primary
startup does not establish a permanent backup connection for standby monitoring.
Additional relay connections can be opened as needed to reach a peer using a
backup, and idle non-primary/non-home connections expire after 60 seconds.
Existing direct connections remain preferred; established peer streams may
continue on an existing path while the home relay changes.

Applications reconnect using the same pinned identity. Relays do not replicate
Wispnet identity, catalogs or permissions. Public n0 service remains rate-limited
and has no uptime guarantee; dedicated independent relays are preferable for
production. Omitting `backup_relay_urls` restores single-packet-relay operation.
Both the host and Android use the same policy; an updated APK is required.
The Android app and new Linux app-host installations explicitly configure the
n0 US West relay as a packet-relay candidate. Existing Linux config is preserved
by the installer and must be updated explicitly.

For an opt-in real public-service check, run:

```sh
cargo run -p wispnet-agent --example relay_failover_probe --locked -- https://usw1-1.relay.n0.iroh.link
```

This uses two disposable identities, an unreachable primary and relay-only
transport. It verifies an authenticated, pinned-ID echo through the backup;
it does not read device state or change pairing policies. The regular test suite
uses local TLS fixtures to cover primary-offline cold startup, loss of the
selected relay, and restart with the other relay offline while preserving the
same identities, catalog and approval policy. Recovery is bounded by transport
probing, reconnect and request timeouts; it is not instantaneous.

Split-home connectivity is also covered: the same primary URL resolves to a
reachable listener for the phone and an unreachable listener for the host. The
phone keeps the primary as home, the host uses the backup, and their first
pinned catalog request succeeds using the locally configured relay candidates,
without an earlier exchange through the primary or any direct UDP path.
