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
