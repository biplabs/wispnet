# Pinned pre-approval bootstrap

An independent schema-2 host offers `wipsnet/pairing/1` on the same owned Iroh
endpoint and key as catalog and service traffic. A fresh, unapproved client can
check the target's pairing protocol without a catalog, directory, peer IP or
another host. Configure the explicit relay and initialize the client's own key:

```sh
target/debug/wispnet identity-init --config /absolute/path/client.toml
target/debug/wispnet peer-hello --config /absolute/path/client.toml --peer PINNED_HOST_ID
```

An optional `--address IP:UDP_PORT` supplies a direct hint. The saved target ID is
still the transport pin. A successful response contains only `version: 1` and
`host: PINNED_HOST_ID`. A caller reusing a connection with a different expected
ID or ALPN fails before sending a request. Response identity, version and schema
are also checked. Raw transport/relay reasons are not exposed.

Hello is transport bootstrap only: it accepts no invitation capability and
creates no approval. The enrollment flow below uses the same pinned endpoint;
there is no remote owner operation. The client receives no host generation, revision, service listing,
backend address or owner credential. Trusting this server does not authorize the
client, grant reciprocal trust, or grant administrative rights. Private control
and service connections still independently require host-local approval/grants.
The caller must obtain the expected host pin through a trusted channel; hello
never discovers or silently replaces a trusted identity.

The contract uses a four-byte big-endian length and strict JSON followed by FIN.
Hello requests are at most 256 bytes; hello replies are limited to 512 bytes.
Enrollment frames below are bounded to 4 KiB in each direction. Unknown fields/operations, trailing bytes and oversized frames fail
closed. Hosts allow one RPC per bootstrap connection, with five-second deadlines
for transport handshake, first-stream arrival, and request/response processing.
Bootstrap uses the existing global connection and stream budgets, and all tasks
are drained on host shutdown. Enrollment also has the invitation/attempt budgets described below.

Local tests exercise unapproved hello through a real TLS relay with UDP disabled,
unchanged authority/private-access denial, malformed/stalled requests, false
response identity/version/schema, pin rejection before stream creation, CLI use,
and shutdown cleanup. Linux namespace network auditing includes the relay hello
in the existing host-control/CLI fixtures. Android invitation review, camera,
code/file and QR-image routes are implemented; [app evidence](android-app.md)
distinguishes optical phone scanning from tablet fixture coverage.

## Invitation contract and protected preflight

`wispnet_core::pairing::Invitation` is the shared Linux/Android contract for a
version-1 target-host ID, HTTPS relay origin, 64-character lowercase hex invitation
ID/capability, `request_access` scope, expiry, and at most one direct socket hint.
Invitations may have at most 15 minutes remaining; challenge proofs at most one
minute, bounded by invitation expiry. Client challenge matching permits a host
clock up to five seconds ahead when checking the maximum future lifetime; it does
not accept an already expired challenge or change host-side nonce expiry. Unknown fields/scopes/versions fail closed.
An invitation is never a global owner credential or an automatic service grant.

```sh
target/debug/wispnet pairing-check --config /absolute/path/client.toml --peer PINNED_HOST_ID --invitation /private/invitation.json
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
Preflight does not accept or persist a pin. `Bootstrap` provides a separate
serializable pin/relay/hint record without invitation IDs or secrets for client persistence after acknowledged pairing.

The challenge transcript is domain-separated from legacy HTTPS enrollment and
binds version, target host, enrolling key, name, invitation ID, scope, nonce,
purpose and expiry. Before signing, clients must match it against their validated
invitation, own key/name and expected purpose. Targets must verify against the
actual authenticated connection identity and their stored challenge. Signature
verification alone does not prevent replay: the host-local state machine below
consumes the stored nonce and requires an independent owner decision.


## Host-local enrollment and approval

Issue an invitation on its target host. The destination must be a new absolute
file in a current-user-owned mode-0700 directory; the command writes mode 0600
and never prints its capability. The lifetime is five minutes.

```sh
target/debug/wispnet host-invite --config /absolute/path/host.toml --output /private/new-invitation.json
target/debug/wispnet pairing-request --config /absolute/path/client.toml --peer PINNED_HOST_ID --invitation /private/new-invitation.json
target/debug/wispnet host-invitations --config /absolute/path/host.toml
```

Deliver only the invitation to the intended client through a trusted channel.
The client sends its capability/name only after transport pin authentication,
checks the returned target-bound challenge, signs with its own key, and submits
that proof over another pinned connection. Both use the configured relay without
a catalog or third host lookup. There is no automatic request retry. A valid
proof atomically consumes its stored nonce and becomes `pending_owner_approval`.
It does not change policy revision or authorize catalog/service access.

On the target host, inspect the pending peer ID/name and confirm the intended
client. Use the current policy revision from `host-status` and a protected local
`host-mutate` request, for example:

```json
{"op":"approve_invitation","expected_revision":0,"invitation":"INVITATION_ID"}
```

`reject_invitation` uses the same fields to reject an issued, challenged or pending
invitation. These are owner-local operations using that host's protected owner
token. Approval inserts only the proved peer/name, consumes the invitation, and
increments policy revision atomically. The optional `services` array selects existing
service IDs to grant to this proved peer in the same transaction, for example
`"services":["llama","gike"]`. The desktop and terminal approval forms offer this
selection with new grants unchecked. Existing grants and other clients' access
are preserved. Unknown/duplicate IDs, an expired or unproved invitation, and a
stale reviewed revision reject the complete approval without partial grants.
Omitting `services` (or selecting none) retains pairing-only behavior; grants may
also be edited later through the owner service controls. Approval never restores a grant
removed by revocation or grants reciprocal/admin rights. No complete-purpose proof
or client bearer token is required: the client's transport identity is its key.

States are `issued` → `challenged` → `pending` → `spent`, with `expired`, `rejected`
and `revoked` terminal outcomes. A challenged invitation is pinned to the actual
client identity/name; challenge refresh rotates the nonce. Changed keys/names,
stale/forged proofs, consumed nonces, wrong-host use and spent capabilities fail
closed. Revocation removes that host's grants and also cancels unfinished requests
already bound to the revoked peer. Other hosts are not consulted or changed.

The target stores only a capability hash. State and attempt budgets survive host
restart. There are at most 128 unexpired invitation rows (including spent/rejected
rows); new issuance prunes expired rows. Each valid capability/proof record permits
12 challenge/proof attempts per 60-second window. Bad capability guesses do not
consume the legitimate capability's budget. Failed signatures do consume it,
committed separately from rejection. Pending listings never contain capabilities
or owner credentials. All state transitions use local SQLite transactions.

A lost reply can leave a proved request pending despite a client-side error. Inspect
`host-invitations` on the target before making another decision; failure is not
proof that no state changed. A failed private-file save can similarly leave an
issued row: reject it and issue a new invitation. Never retry a service operation
or treat a queued/offline multi-host intent as authorization.

Linux and authorized Android native relay fixtures exercise issuance, proof,
pending denial, local approval, separate service grants, restart and revocation
against each host while the other is off. These are native CLI fixtures, not APK,
QR/UI, cellular or physical two-computer acceptance. The later Linux/headless combined-owner status workflow is documented below; final app/deployment checks remain separate.

## Saved client bootstrap

After an authenticated `pairing-request` acknowledgement, the client atomically
saves `state_dir/peer-pins/HOST_ID.json` in a protected mode-0700 directory. The
record is mode 0600 and contains only version, expected host ID, relay URL and
optional direct hint. It contains no invitation ID, capability, client key,
expiry or authority/grants. Pending acknowledgement saves trust/bootstrap data,
not approval. `pairing-check` and `peer-hello` do not save pins.

Once pairing is acknowledged, the invitation can be removed from the client.
Fresh `host-catalog`, `peer-hello`, `peer-probe`, `peer-watch` and `tunnel` processes
load the saved record for the requested `--peer` ID. Explicit `--address` hints
win over saved hints. Explicit local `[transport]` configuration wins over the
saved relay, including a configuration with no relay URL (relay disabled). If
there is no transport block, the saved bootstrap supplies the sole relay, with
normal certificate verification and direct transport enabled. Private relay CAs
remain independently configured; a saved record never introduces a trust root.

Changing a relay's IP under its hostname needs no pin update. An explicit local
relay configuration may override saved location without replacing the host key.
A new key cannot replace an existing pin: records are keyed by EndpointId and
must match that filename. Corrupt/mismatched/oversized records, insecure modes,
final symlinks, hardlinks and an unsafe pin directory fail closed before dialing;
commands do not fall back to an invitation or silently repair malformed trust.
Saves serialize through a protected lock, use atomic replacement and fsync, and
refuse to overwrite a corrupt existing record. Missing records still permit the
existing explicit-pin/address/config workflow; no identity is discovered or
accepted automatically.

A local save error after the pending acknowledgement can leave an actual request
pending on the host while the CLI reports failure. Check the target's local
invitation list; do not infer rollback or replay a service request. Durable pin
state does not bypass host-local revocation or restore removed grants.

Tests reconnect from fresh CLI processes after deleting the invitation, use a
saved direct hint for both diagnostics and service bytes with relay explicitly
disabled, and check secret-free independent pin records, protected modes,
corruption/link rejection, locking and location overrides. The native lifecycle
harness deletes issued invitation files on Linux and the authorized tablet before
approval/restart/recovery checks and verifies both saved records remain secret-free.

## Combined owner workflows

Linux desktop and headless owner tools now provide durable per-target add/revoke results. See [combined owner operations](linux-desktop.md#combined-owner-operations) for explicit owner routes, private plans, pending-only retries and atomic target receipts. This uses each target's protected local administration (optionally reached through an already trusted SSH owner session); ordinary Iroh pairing never grants remote admin rights. A queued offline revocation is not delivered until that host acknowledges it. Full-machine power-off, deployed relay replacement, Android combined-owner UI and stale-backup recovery are separate from this Linux/headless checkpoint.
