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

Better Auth handles GitHub identities, browser sessions, OAuth, refresh tokens,
CIMD, explicitly enabled dynamic client registration and hashed API keys.
Wispnet owns host linking, service registration, owner grants and revocation.
API keys are restricted to a single service and expire; their value is shown
only on creation. The hub checks current grants even for a previously issued key.

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

- `apps/auth-hub`: independent Node 24+ application, SQLite state and pinned Better
  Auth dependencies. No private device registry or backend credentials are here.
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
WISPNET_COMMAND_STATE=/absolute/private/command-state
WISPNET_COMMAND_SECRET=<persistent random secret, at least 32 characters>
WISPNET_GITHUB_CLIENT_ID=<GitHub OAuth app client ID>
WISPNET_GITHUB_CLIENT_SECRET=<GitHub OAuth app client secret>
WISPNET_PUBLIC_DOMAIN=wispnet.app
WISPNET_COMMAND_PORT=8325
```

The process binds IPv4 loopback. Caddy provides HTTPS. Startup fails if the GitHub
configuration is missing; no development login or password endpoint is exposed.
Keep the hub database and signing/encryption secret in backups; unlike the opaque
packet relay, this optional public account service has durable authority.

Run locally after configuring the environment:

```sh
npm --prefix apps/auth-hub ci
npm --prefix apps/auth-hub test
npm --prefix apps/auth-hub start
```

`scripts/install_command_center.py --ssh-host pi3.home --github-env /absolute/private/github.env`
installs the official checksum-verified Node runtime and hub user service. The
input file must be mode 600 and contain only the two GitHub variables. It preserves
the hub secret across upgrades. Review the generated Caddy fragment and the live
HTTPS configuration before adding the explicit `cmd.wispnet.app` host. This
installer does not overwrite relay/gateway/Caddy configuration or publish services.

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
