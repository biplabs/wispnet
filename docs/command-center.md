# Optional public Command Center

Wispnet Command Center is the self-hosted account and public sharing service at
`https://cmd.wispnet.app`. Its first login provider is GitHub. Public publishers
register an account and explicitly link their hosting machine. Private pairing,
device grants, service URLs and Shell remain independent of an account or hub.

## Access and credentials

- Private requests authenticate the actual Iroh device and check the hosting
  machine's existing service grant. Owner-local routes retain local owner access.
- Public requests use OAuth authorization-code/PKCE or a named bearer API key.
  Every public MCP endpoint is a distinct OAuth resource. A registered account or
  OAuth client alone grants no access to somebody else's service.
- The hosting machine authorizes public requests, then consumes the public token
  and optionally inserts a separately configured backend bearer credential.
  Existing MCP servers need no code changes for this service-level access model.
- Backend credentials live only in owner-protected host storage. The hub and
  public gateway receive no backend credential. Status returns configuration
  metadata only. This is file-permission protection, not encryption at rest.
- An approved device's applications share its private access. Public grants
  permit the complete service, including any write tools. Separate backend user
  accounts, application data isolation and read-only tools are not synthesized.

The optional Rust hub handles GitHub identities, browser sessions, MCP OAuth,
refresh tokens, Client ID Metadata Documents (CIMD), compatibility Dynamic Client
Registration (DCR), and hashed API keys. It also owns public host linking,
service registration, owner grants and revocation. Access and refresh tokens are
opaque random secrets with hashed, service-bound database records. The supported
OAuth scopes are `mcp:use` and `offline_access`; the hub is a focused MCP
authorization server, rather than a general-purpose OpenID Connect identity server.
API keys are restricted to a single service and expire; their value is shown
only on creation. The hub checks current grants even for a previously issued key.

OAuth supports public PKCE clients and CIMD clients using RS256
`private_key_jwt` authentication. The latter verifies a client's signed assertion
without changing the opaque access-token design. DCR registers public clients;
the supported token endpoints require an explicit body `client_id`.

The host HTTP mediator checks public authorization every two seconds, with a
two-second check deadline, and stops active streams if authorization cannot be
renewed. Public access fails closed during a hub outage; private device access
continues. Credential rotation and service replacement stop stale mediated
streams. Revocation cannot undo an operation already completed.

Desktop disconnect stops managed publications and clears the local account link
even when the hub is unavailable or the dashboard already revoked the link.
If remote revocation cannot be confirmed, the desktop keeps a visible notice
with a link to Command Center so the owner can check the remote account state.

## Components and removal

- `apps/auth-hub`: independent Rust executable and private SQLite state. Its
  bundled SQLite engine needs no separate database service. No private device
  registry or backend credentials are here, and deployment needs no Node runtime.
- `crates/client/src/command_center.rs`: optional account link and public policy
  adapter. It is not contacted for private service authorization.
- `crates/agent/src/http_backend.rs`: host HTTP forwarding and credential injection.
- `crates/registry/src/backend_credentials.rs`: owner-protected credential store.
- `apps/flutter/lib/src/command_center`: desktop account/publishing module, with
  one widget integration in desktop settings.
- `apps/flutter/desktop/command_center.py`: desktop owner-operation/browser bridge.

Build the desktop with `--dart-define=WISPNET_COMMAND_CENTER=false` to remove the
account/publishing UI and its API calls. To remove the module entirely, remove its
single import/widget entry and Python bridge import/dispatch hook; remove the
helper from packaging. The module README gives the precise wiring points.
Stop public publications before removing the server or integration. Private host
keys and grants do not need migration. The backend credential mediator can remain
useful independently or be reset explicitly to pass-through service by service.

## Run and deploy

Create a GitHub OAuth app with homepage `https://cmd.wispnet.app` and callback
`https://cmd.wispnet.app/api/auth/callback/github`. Configure these environment
variables in a protected service environment file:

```
WISPNET_COMMAND_URL=https://cmd.wispnet.app
WISPNET_COMMAND_STATE=/home/johnny/.local/share/wispnet-command-center/state
WISPNET_GITHUB_CLIENT_ID=<GitHub OAuth app client ID>
WISPNET_GITHUB_CLIENT_SECRET=<GitHub OAuth app client secret>
WISPNET_PUBLIC_DOMAIN=wispnet.app
WISPNET_COMMAND_PORT=8325
```

The process binds IPv4 loopback. Caddy provides HTTPS. Startup fails if the GitHub
configuration is missing; no development login or password endpoint is exposed.
Keep a consistent backup of the hub database, including pending WAL data; unlike
the opaque packet relay, this optional public account service has durable
authority. The Rust hub does not require a JWT signing key or a separate
`WISPNET_COMMAND_SECRET` because tokens are validated against stored hashes.

Run locally after configuring the environment:

```sh
cargo test --locked -p wispnet-command-center
cargo build --release --locked -p wispnet-command-center
# With the service environment configured:
target/release/wispnet-command-center
```

Keep this complete environment file on `pi3.home`, owned by its service user and
mode `600`. Edit the GitHub values directly there:

```sh
ssh -t pi3.home 'nano ~/.config/wispnet-command-center/command.env'
scripts/install_command_center.py --ssh-host pi3.home --use-remote-env \
  --binary /absolute/path/wispnet-command-center
```

The installer validates all six settings on the gateway and copies the protected
file directly into remote staging. It does not download secrets or create a
local environment file. URL and public domain must match the installer defaults
or its `--url`/`--public-domain` options, and the supplied service unit uses the
state path above and port `8325`. The alternative `--github-env` option explicitly
stages a local protected file containing the two GitHub values; it cannot be
combined with `--use-remote-env`.

For `pi3.home` from an x86_64 workstation, `--binary` must be built for compatible
Linux `aarch64`; the installer verifies the ELF architecture and uploaded
checksum. On a matching Linux architecture, omit `--binary` to build locally.
It stages executable, environment and unit changes, checks `/healthz`, and
restores their previous versions and service state if the update fails. It
preserves the database; rollback does not revert schema changes.

Review the generated Caddy fragment and the live HTTPS configuration before
adding the explicit `cmd.wispnet.app` host. The installer does not overwrite
relay/gateway/Caddy configuration or publish services. The unit's 320 MiB memory
limit is a configured ceiling, not a measured runtime requirement.

A legacy Better Auth database is detected and rejected instead of being silently
overwritten. Existing accounts and tokens need a deliberate migration; preserve
that state until migration is complete. The Rust replacement is intended to
start with fresh protected state for the first deployment.

Upgrade the public gateway and merge `deploy/command-center/gateway.example.toml`.
`allow_legacy=false` requires registered version-2 publications, including for
known publisher devices. Migrate existing publications deliberately first.
Never set `allow_legacy=true` as a fallback for a protected service.

## GIKE pilot

1. Keep the existing GIKE backend and private grants unchanged. Its configured
   MCP path is `/mcp` by default; preserve the actual path and its case.
2. On the hosting desktop, use **Connect public account**. Verify the displayed
   host/code in the system browser, sign in with GitHub, and approve that host.
3. In the GIKE service's backend authentication setting, choose **No backend
   credential** if GIKE has none, or configure its existing bearer credential.
   Never use a hub publisher credential or public API key as the backend secret.
4. Enable authenticated public access. The owner-only operation is
   `configure_public` with `managed:true`, the current service/incarnation, and
   the actual `mcp_path`. Older gateways cannot accept this protected protocol.
5. In Command Center choose OAuth, bearer, or both. The owner has access;
   explicitly grant registered account IDs to other users. They use their own
   OAuth connection or service-scoped API key rather than the backend key.
6. Validate only MCP initialization and tool listing initially. Confirm missing,
   revoked and wrong-service credentials reach no backend, then test the existing
   private URL with the hub stopped. Do not run mutating GIKE tools as a probe.

The existing Wispnet management MCP is separate from a hosted service's MCP.
Its installation credential is not a public-service credential.
