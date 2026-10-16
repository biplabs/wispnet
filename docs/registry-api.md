# Single-owner registry API (v1)

The registry stores one mesh and one owner credential, device records, service
ownership, published Iroh socket addresses, and whole-service grants. It does
not forward application payloads. The owner assigns service IDs and hosting
devices; devices can only publish addresses for their assigned services.

## Bootstrap and deployment

1. Copy `config/registry.toml`, replace the state path and choose the mesh name.
   Use a dedicated OS account and a trusted local filesystem. Run:

   ```sh
   cargo build --workspace --locked
   target/debug/wipsnet registry-init --config /absolute/path/registry.toml
   target/debug/wipsnet registry --config /absolute/path/registry.toml
   ```

   Initialization generates a random 256-bit owner bearer credential in
   `state_dir/owner.token` (0600). Only its SHA-256 hash is stored in SQLite.
   Nothing prints the credential. Repeating initialization with the same file
   is idempotent; it cannot replace an existing owner. Preserve this file in a
   secure backup. Owner transfer and owner credential rotation are not v1 APIs.
   A lost owner credential cannot be recovered from the database hash.

2. Provision a domain under your control, point its DNS A/AAAA records at the
   controller server, and allow TCP 80/443 to the HTTPS proxy. Expose only the
   proxy publicly; the Rust listener rejects non-loopback bind addresses.
   A minimal Caddy configuration is:

   ```caddyfile
   {
       servers {
           max_header_size 16KB
           timeouts {
               read_header 5s
               read_body 10s
               write 15s
               idle 30s
           }
       }
   }
   controller.example.com {
       request_body {
           max_size 128KB
       }
       reverse_proxy 127.0.0.1:7780
   }
   ```

   Caddy manages domain certificates and renewals. Validate the config against
   your installed Caddy version before starting it. See the official
   [HTTPS setup](https://caddyserver.com/docs/quick-starts/https),
   [server limits](https://caddyserver.com/docs/caddyfile/options), and
   [reverse proxy reference](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy).
   This deployment recipe has not been exercised against a public domain in
   this repository's tests. No domain or cloud resources are provisioned by the CLI.

3. Copy `config/controller.toml` onto each device. The controller HTTPS origin
   and device credential are provisioned out of band. They do not depend on the
   private service directory, avoiding a bootstrap cycle. The Rust client uses
   its certificate trust roots, verifies the hostname, requires HTTPS, ignores
   ambient proxy settings and refuses all redirects. There is no insecure TLS
   option or HTTP fallback. Never send an owner/device credential to a public
   HTTP URL, even if that server redirects to HTTPS.

The local HTTP hop between proxy and registry trusts the server OS and local
proxy. Local processes without credentials still cannot read the directory.
Use host-level resource controls and protect proxy configuration. Request/header
limits at the HTTPS proxy are part of deployment; the API separately limits
bodies to 128 KiB, active requests to 16, database workers to 16 and requests to
10 seconds. SQLite lock waits are limited to 3 seconds. These are application
bounds, not a complete denial-of-service or process RSS guarantee. Only fixed
lifecycle fields and revision numbers are logged; do not enable credential or
body logging in your proxy.

## Requests and authorization

All private endpoints require exactly one `Authorization: Bearer TOKEN` header.
All responses disable caching. Browser Origin headers are rejected and CORS is
not enabled. JSON errors use a stable `error` code without reflecting submitted
values, tokens, paths, or SQLite messages.

| Endpoint | Access | Behavior |
|---|---|---|
| `GET /healthz` | Public | Process liveness only; no private data |
| `GET /readyz` | Public | 200 only with initialized controller and healthy database |
| `GET /v1/directory` | Owner | Full directory, grants and current revision |
| `GET /v1/directory` | Device | Own device ID, services it hosts or is granted; grant lists only for services it hosts |
| `POST /v1/mutations` | Owner | Enroll, revoke, assign service/grants, publish route |
| `POST /v1/mutations` | Device | Publish route for a service it owns |

Unknown API versions return 404. Errors include `unauthorized` (401), `forbidden`
(403), `revision_conflict` (409), `invalid_json`/`invalid_input` (400),
`body_too_large` (413), `busy`/`not_ready` (503), `request_timeout` (408), and
`storage_failed` (500). Unknown fields and operations fail closed.

Mutations include `expected_revision`. Each successful mutation increments the
shared revision once. Reads take one consistent transaction. A hosting-owner
change clears the old published route in the same transaction as grants and
ownership. Revocation removes the device's credential, grants and hosted services
atomically. Existing service tunnels do not yet consume these revisions; runtime
policy synchronization and stale-cache enforcement remain separate backlog work.

Mutation examples (public endpoint IDs are lowercase hex):

```json
{"op":"enroll","expected_revision":0,"device":"PUBLIC_ENDPOINT_ID"}
{"op":"service","expected_revision":1,"id":"website","host":"PUBLIC_ENDPOINT_ID","grants":[]}
{"op":"publish","expected_revision":2,"id":"website","address":"192.0.2.10:7779"}
{"op":"revoke","expected_revision":3,"device":"PUBLIC_ENDPOINT_ID"}
```

Enrollment returns `{ "revision": 1, "token": "..." }` once. Other mutations
return only the new revision. Duplicate enrollment conflicts rather than rotating
a token. If an enrollment response is lost, revoke that device then enroll again
using the latest revision. This is owner-controlled manual provisioning; #7 adds
expiring invitations and proof of device key possession. Giving a token to a
person/device is a deliberate owner action. Tokens alone authenticate this API;
a stolen token must be revoked. Owner/admin credentials must never go to devices.

## Owner tool and device startup

Use the HTTPS-only stdlib tool to avoid putting credentials into shell arguments
or displaying newly enrolled tokens. Obtain a device's public endpoint ID using
`wipsnet identity-init`/`status`, then write its enrollment JSON into a file:

```sh
python3 scripts/registry_admin.py --origin https://controller.example.com \
  --owner-token /private/controller/owner.token
python3 scripts/registry_admin.py --origin https://controller.example.com \
  --owner-token /private/controller/owner.token \
  --mutation /private/enroll.json --credential-out /private/new-device.token
```

The first command reads the owner directory and current revision. The second
writes the returned token directly into a new 0600 file. Deliver it to the
intended device through a secure channel, preserving mode 0600 and current-user
ownership. Keep its parent directory private. Use the same mutation tool without
`--credential-out` for service assignments, route publication and revocation.
It rejects HTTP, redirects and occupied credential output paths. Like any owner
client it may display private directory metadata; run it in a trusted terminal.

```sh
target/debug/wipsnet registry-directory --config /absolute/path/device.toml
target/debug/wipsnet agent --config /absolute/path/device.toml
```

Both fetch and verify a directory scoped to the persistent local endpoint ID.
A token issued for another device fails the identity check. Configured agent
startup fails if the controller cannot be authenticated or rejects the token;
there is no stale/offline fallback yet. This is an initial connection/preflight,
not a periodic policy synchronization loop. Agents without a controller section
retain the existing development mode using static service grants.

## Recovery and verification

Schema 2 migrates schema 1 transactionally, preserving directory/revision data.
Future versions are refused. See [storage and recovery](registry-storage.md).
The database backup includes owner/device credential hashes and policy, but the
owner's raw credential file must be backed up separately. Device credentials
already provisioned continue to work against a restored database. Restore can
resurrect revoked credentials if the backup predates revocation: reconcile and
revoke them before reconnecting the public proxy.

```sh
target/debug/wipsnet registry-backup --config /absolute/path/registry.toml \
  --output /private/backups/new-registry.sqlite3
./scripts/test.sh
```

Tests cover version migration, role separation, directory filtering, revocation,
route-owner changes, rollback/revision conflicts, private credential bootstrap,
backup authority preservation, invalid/oversized JSON and browser requests. TLS
tests accept a trusted test certificate and reject untrusted roots, wrong
hostnames and redirects. The native CLI smoke starts a real API process, enrolls
and publishes a service, takes a live backup, restores into a fresh private state
directory, restarts, verifies credentials/policy, and revokes the device.
