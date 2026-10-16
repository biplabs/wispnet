# Pinned pre-approval bootstrap

An independent schema-2 host offers `wipsnet/pairing/1` on the same owned Iroh
endpoint and key as catalog and service traffic. A fresh, unapproved client can
check the target's pairing protocol without a catalog, directory, peer IP or
another host. Configure the explicit relay and initialize the client's own key:

```sh
target/debug/wipsnet identity-init --config /absolute/path/client.toml
target/debug/wipsnet peer-hello --config /absolute/path/client.toml --peer PINNED_HOST_ID
```

An optional `--address IP:UDP_PORT` supplies a direct hint. The saved target ID is
still the transport pin. A successful response contains only `version: 1` and
`host: PINNED_HOST_ID`. A caller reusing a connection with a different expected
ID or ALPN fails before sending a request. Response identity, version and schema
are also checked. Raw transport/relay reasons are not exposed.

This is the transport bootstrap for #13/#17, not completed pairing: no invitation
capability is accepted, no approval is created, and no remote owner operation is
available. The client receives no host generation, revision, service listing,
backend address or owner credential. Trusting this server does not authorize the
client, grant reciprocal trust, or grant administrative rights. Private control
and service connections still independently require host-local approval/grants.
The caller must obtain the expected host pin through a trusted channel; hello
never discovers or silently replaces a trusted identity.

The contract uses a four-byte big-endian length and strict JSON followed by FIN.
The only request is `{"op":"hello"}` (maximum 256 bytes); replies are limited to
512 bytes. Unknown fields/operations, trailing bytes and oversized frames fail
closed. Hosts allow one RPC per bootstrap connection, with five-second deadlines
for transport handshake, first-stream arrival, and request/response processing.
Bootstrap uses the existing global connection and stream budgets, and all tasks
are drained on host shutdown. Invitation budgets and per-client rate limiting belong to #17's pending
enrollment implementation.

Local tests exercise unapproved hello through a real TLS relay with UDP disabled,
unchanged authority/private-access denial, malformed/stalled requests, false
response identity/version/schema, pin rejection before stream creation, CLI use,
and shutdown cleanup. Linux namespace network auditing includes the relay hello
in the existing host-control/CLI fixtures. Android app invitation UI, signed
proofs, durable per-host pins and spent-capability protection remain #17/#12 work.

## Invitation contract and protected preflight

`wipsnet_core::pairing::Invitation` is the shared Linux/Android contract for a
version-1 target-host ID, HTTPS relay origin, 64-character lowercase hex invitation
ID/capability, `request_access` scope, expiry, and at most one direct socket hint.
Invitations may have at most 15 minutes remaining; challenge proofs at most one
minute, bounded by invitation expiry. Unknown fields/scopes/versions fail closed.
An invitation is never a global owner credential or an automatic service grant.

```sh
target/debug/wipsnet pairing-check --config /absolute/path/client.toml --peer PINNED_HOST_ID --invitation /private/invitation.json
```

Preflight validates the expected pin, expiry and configured relay before creating
an endpoint or dialing. Relay origins are compared after URL normalization;
invitations cannot replace the locally configured relay or CA. Direct hints do
not replace the pin. The document must be a current-user-owned private regular
file (normally mode 0600), at an absolute path, at most 8 KiB, with no final
symlink or hardlinks. Group/other permissions, special files and `..` components
are rejected. Errors never print input fields. Secret file buffers and invitation
capabilities are zeroized when dropped.

The command sends only the existing hello request, not the capability or proof.
It returns `state: bootstrap_verified`, `capability_checked: false` and
`approval_checked: false`. A syntactically valid invented capability can pass
this transport check; it establishes neither issuance, enrollment nor approval.
No pin is silently accepted or persisted. `Bootstrap` provides a separate
serializable pin/relay/hint record without invitation IDs or secrets for future
client persistence.

The challenge transcript is domain-separated from legacy HTTPS enrollment and
binds version, target host, enrolling key, name, invitation ID, scope, nonce,
purpose and expiry. Before signing, clients must match it against their validated
invitation, own key/name and expected purpose. Targets must verify against the
actual authenticated connection identity and their stored challenge. Signature
verification alone does not prevent replay: host-local issuance, nonce consumption,
owner approval, spent-capability storage and revocation remain #17 implementation
work. No network proof exchange or durable enrollment state is added here.
