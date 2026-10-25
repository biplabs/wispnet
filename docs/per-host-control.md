# Independent host control: first #16 checkpoint

[ADR 0002](decisions/0002-per-host-authority.md) is the accepted architecture.
Each host has an independent configuration, protected local administration,
host-local SQLite authority and bounded Iroh catalog protocol on its service
endpoint. Target-host invitations, active-stream cancellation, desktop/headless
IPC and persistent desktop catalogs are implemented; see
[pairing](pairing-bootstrap.md) and [desktop/headless](linux-desktop.md).
Service-incarnation removal/recreation safety is implemented; see
[service lifecycle](service-lifecycle.md). Full-machine power-off, deployed relay
replacement remain open acceptance work (#8/#9/#14/#18).

## Initialize each host independently

Copy `config/node/host.toml` and choose that installation's own state path and UDP bind.
Do not copy another installation's identity, owner token or authority database.
For a direct LAN fixture choose an explicit reachable bind address; loopback is
appropriate for local development. No public home HTTP listener, Caddy, ACME or
remote controller is required by this mode.

```sh
cargo build --workspace --locked
target/debug/wispnet host-init --config /absolute/path/host.toml
target/debug/wispnet agent --config /absolute/path/host.toml
```

`schema_version = 2` selects independent host authority and requires a transport
section. It rejects legacy controller/registry sections and static service arrays,
so there is no hidden remote preflight or static-grant fallback. Schema-1 commands
remain the earlier foundation for regression checks;
they are not the new architecture. Earlier `registry`, `registry-init`, `enroll`
and `registry-directory` commands are rejected with schema-2 configurations.

The agent holds one persistent identity lease and owns one Iroh endpoint with
one accept loop for service and control ALPNs. The same endpoint can initiate
outgoing connections; it does not require a second daemon or another copy of the
private key. The [Linux access/IPC workflow](linux-desktop.md) attaches to that endpoint; the standalone
`host-catalog` diagnostic below uses a separate client's identity and cannot
acquire the key lease of a running host agent.

State is current-user-owned in a 0700 directory:

| File | Purpose |
|---|---|
| `identity.key` and identity locks | This installation's persistent Iroh identity/lease |
| `host.sqlite3` | Local host ID, generation, revision, approved clients, backend bindings and grants |
| `host-owner.token` | Independent local administration secret, mode 0600 |
| `registry.sqlite3`, `owner.token` if retained | Separate legacy foundation data; never implicit host authority |

Initialization preserves an existing valid identity and generates a fresh local
owner credential. It is idempotent only with intact matching authority and owner
state. If the owner credential survives but the database is missing, init refuses
to recreate it. If previously initialized host state exists but the identity is
missing, init loads/fails rather than generating a replacement key. Cold startup
never creates a missing host database, reconstructs policy from other peers, or
promotes another host. Corrupt/wrong-host state fails closed for this host.
Explicit crash/recovery and stale-backup reconciliation remain #19; keep protected
backups and do not delete authority files to work around an error.

## Protected local owner operations

Owner commands operate on this installation's private SQLite store and verify its
local owner token and host identity. They do not send administrator credentials
over Iroh or expose a remote owner RPC. Another host's token fails. Access to the
local OS account/private state is the administration boundary; local filesystem
compromise is not protected by an additional network password.

```sh
target/debug/wispnet host-status --config /absolute/path/host.toml
target/debug/wispnet host-mutate --config /absolute/path/host.toml \
  --request /absolute/path/request.json
```

Requests are bounded to 32 KiB with strict schema/unknown-field rejection. Read
this host's current revision using `host-status`; revisions are not comparable
between unrelated hosts. Each successful mutation atomically bumps it once.

To approve a known client locally, obtain its full public EndpointId through a
trusted channel (`identity-init`/`status` on that client), then submit:

```json
{"op":"approve","expected_revision":0,"peer":"PUBLIC_CLIENT_ID","name":"my tablet"}
```

This is an explicit owner decision for this host only. It does not approve the
client at any other host, trust another server, grant administrator power or imply
reciprocal access. The authenticated Iroh handshake subsequently proves the
connecting client's key. Network invitation/proof/QR convenience for this mode
is still #17; the old HTTPS invitation is not accepted as per-host approval.

Register a local service with separately approved client IDs:

```json
{"op":"service","expected_revision":1,"id":"llama","backend":"127.0.0.1:8201","allow":["PUBLIC_CLIENT_ID"]}
```

Backends must be explicit loopback IPs with nonzero ports. A host allows at most
128 services, 128 distinct grants per service and 1,024 approved clients. Unknown
clients cannot appear in an allowlist. An empty allowlist exposes nothing.
A service update replaces its binding/grants atomically; only protected local
administration can select a backend. No remote RPC accepts an upstream address.
Optional `kind` (`http`/`tcp`) preserves the existing type when omitted.
Legacy `display_name` input is accepted but ignored; the service ID is its only label. See [versioned complete catalogs](catalog-snapshots.md)
for record validation, filtered removal and durable client version checks.

To rename and edit an existing service atomically:

```json
{"op":"edit_service","expected_revision":2,"id":"llama","new_id":"local-model","backend":"127.0.0.1:8201"}
```

Omitting `allow` preserves existing permissions; an explicit `allow` list replaces
them. The old ID must exist, and the new ID must be unused unless unchanged.
The rename preserves pairings and incarnation (unless `kind` also changes), and
advances the revision once. Refresh other devices' catalogs to see the new name;
saved URLs using the old name need updating. See [service lifecycle](service-lifecycle.md).

```json
{"op":"remove","expected_revision":3,"id":"local-model"}
{"op":"revoke","expected_revision":4,"peer":"PUBLIC_CLIENT_ID"}
```

Revocation removes the local approval and cascades its service grants. New control
and service requests reread current local state, including on existing connections.
There is no unrelated-host lease or catalog TTL controlling these durable grants.
Already open streams check durable policy every second, with a one-second store
read deadline, and cancel on lost permission, removal, changed binding or unreadable
authority. These are implementation deadlines, not a universal measured latency
guarantee. Local and physical revocation evidence is recorded in the
[Android](android-app.md) and [desktop](linux-desktop.md) guides; deployed relay
replacement and final release measurements remain #9/#14/#18.

## Catalog/control transport

```sh
target/debug/wispnet host-catalog --config /absolute/path/client.toml \
  --peer PINNED_HOST_ID --address 192.0.2.10:7779
```

The client's config/state belongs to its own persistent identity. The supplied
EndpointId is the expected peer pin; an IP hint never substitutes for it. This
command needs no pre-downloaded catalog or third machine. With an explicit `transport.relay_url`, omit `--address` to connect using only
the pinned identity and relay. The optional address is one direct hint. See
[relay connectivity](relay-connectivity.md) for configuration and tested limits.

The endpoint advertises `wipsnet/service/1` and `wipsnet/control/1`. Control uses
one request per bidirectional stream: a 32-bit big-endian length, bounded JSON
`{"op":"catalog"}`, then FIN. Trailing bytes and unknown fields/operations fail
closed. Requests are limited to 4 KiB; responses to 128 KiB; the entire operation
times out after 5 seconds. Connection/global-stream/per-connection credits are
shared with service transport and owned tasks drain on shutdown. The owner
endpoint is dropped explicitly; a regression verifies UDP port release within
250 ms after transport shutdown rather than assuming same-tick socket teardown. A busy/corrupt
local store denies rather than falling back to static permissions.

Unknown peers are denied before private RPC/service access. Approved peers get
only IDs they may access, in a catalog shaped as:

The response is the [version-1 complete catalog](catalog-snapshots.md): explicit
version/completeness/count plus host generation/revision and sorted typed service
records containing ID and incarnation (plus a derived legacy display-name field). Its canonical service
identity includes `(catalog.host, service.id, service.incarnation)`, verified
against the transport peer pin. Backend addresses, grants, owner credentials and
another host's records are not returned. Receiving a catalog never mutates local
ACLs or pins. The shared native client durably validates per-host high-water
before returning snapshots to Linux desktop/headless or Android callers.
Rollback-safe authoritative restore remains #19.

## Development setup scope

Use fresh, separate state directories for each host. There is no import or
production compatibility workflow in this checkpoint; the project is still in
development. Old controller data is not used as per-host authority. Do not copy
owner credentials or private identity keys between fresh installations.

## Evidence and remaining work

`./scripts/test.sh` includes `scripts/host_smoke.py` alongside preserved foundation
checks. The native test runs the same host binary symmetrically with the other
host stopped, cold-starts/restarts, registers `llama`, changes grants, approves a
fresh client locally, exercises catalog and actual HTTP service transport, then
revokes locally. It verifies independent generations/revisions and no remote
controller preflight. Corrupt/missing authority fails closed without disabling
the healthy peer. No production migration/import behavior is claimed.

Rust tests cover host/token separation, malformed/wrong-protocol requests,
concurrent control/service use on one endpoint, bounded frames, cancellation and
owned-task shutdown. This is local Linux evidence, including a real TLS relay fixture, not physical power-off
on two separate computers, Android app validation, deployed-relay wipe/rebuild recovery, target-bound invitation pairing or bounded cancellation of active
service streams. Those remain the named follow-up issues and release gates.
