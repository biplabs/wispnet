# Wispnet Command Center

This optional Rust service manages accounts and **public** Wispnet access. It
provides GitHub sign-in, browser sessions, the MCP OAuth authorization flow and
service-scoped bearer keys. Its public interfaces remain independent of the
desktop and gateway implementations. Private device pairing and host-local
service grants do not depend on this module.

The command center never receives a service's backend credential. Public callers
present a command-center OAuth token or service-scoped bearer key; the hosting
node checks authorization and supplies its own separately stored backend
credential. The Rust gateway remains responsible for MCP transport and streaming.

## Run

Build with the workspace Rust toolchain. The deployed executable needs no Node.js
or npm runtime and uses a bundled SQLite engine without a separate database
service.

```sh
cargo test --locked -p wispnet-command-center
cargo build --release --locked -p wispnet-command-center
# With the variables below supplied by the service environment:
target/release/wispnet-command-center
```

Create the state directory owned by the service user with mode `0700`. Protect
the environment file with mode `0600`. Configuration:

| Environment variable | Purpose |
| --- | --- |
| `WISPNET_COMMAND_URL` | Public HTTPS origin; defaults to `https://cmd.wispnet.app` |
| `WISPNET_COMMAND_STATE` | Absolute owner-only persistent state directory |
| `WISPNET_GITHUB_CLIENT_ID` | GitHub OAuth application client ID |
| `WISPNET_GITHUB_CLIENT_SECRET` | GitHub OAuth application client secret |
| `WISPNET_PUBLIC_DOMAIN` | Published service hostname suffix; defaults to `wispnet.app` |
| `WISPNET_COMMAND_PORT` | Loopback HTTP port; defaults to `8325` |

Register a GitHub OAuth application with homepage `https://cmd.wispnet.app` and
authorization callback `https://cmd.wispnet.app/api/auth/callback/github`.
GitHub is the only login method. Missing provider credentials fail startup with
a setup error; there is no development password endpoint or production test
login switch.

Put the loopback listener behind HTTPS. Preserve the public Host and scheme,
forward `/api/auth/*`, `/api/command/*`, `/.well-known/*`, and the dashboard
routes, and do not log request bodies or authorization values. The service
enforces resource ownership and stores authoritative key bindings.

SQLite stores accounts, hashed sessions and credentials, public service
registrations, grants, and OAuth client/connection state. Access and refresh
tokens are opaque random secrets; their authoritative bindings live in hashed
database records. No JWT signing key or `WISPNET_COMMAND_SECRET` is needed.
Preserve the state directory across restarts. Use a consistent SQLite backup, including
pending WAL data; copying only a live database file is insufficient. Filesystem
permissions protect the state; the module does not claim that the database is
encrypted at rest.

The Rust server detects a legacy Better Auth database and refuses to initialize
over it. There is no automatic account/token migration. Preserve that database
and plan an explicit migration before changing an existing deployment; never
delete it merely to make startup succeed.

Keep the complete configuration and GitHub credentials on the gateway. Edit its
mode-600 environment file over SSH, rather than copying secrets to a workstation:

```sh
ssh -t pi3.home 'nano ~/.config/wispnet-command-center/command.env'
scripts/install_command_center.py --ssh-host pi3.home --use-remote-env
```

The remote file must contain all six settings in the table. For the supplied
service unit and installer, set `WISPNET_COMMAND_STATE` to
`/home/johnny/.local/share/wispnet-command-center/state` and port `8325`; the
configured URL and public domain must match `--url` and `--public-domain` when
those are customized. The installer checks ownership, permissions, required
values and key names on the gateway, then copies the file directly into remote
staging. It never returns its contents or writes a local environment file.

The installer builds locally for a matching architecture. For an ARM64 gateway
from an x86_64 workstation, build a compatible Linux `aarch64` executable
separately and pass `--binary /absolute/path/wispnet-command-center`. It checks
the ELF architecture and upload checksum before installing. The binary must
also match the gateway's system ABI; checking the CPU architecture alone does
not establish libc compatibility.

`--github-env /absolute/private/github.env` remains an explicit alternative for
operators who choose to stage a local protected file containing only the two
GitHub variables. It is mutually exclusive with `--use-remote-env`.

Executable, environment and service-unit updates are staged, then checked via
`/healthz`. A failed update restores the previous files and running/enabled state.
The installer preserves the database and does not change live Caddy, gateway,
relay or publishing policy. A rollback does not revert database schema changes.
The service's 320 MiB memory limit is a configured ceiling, not measured usage.

## Link a host

1. The hosting node requests `POST /api/command/links/start` with `host`, `name`,
   `nonce`, and `signature`. The host and nonce are 64 lowercase hexadecimal
   characters; the Ed25519 signature is 128 lowercase hexadecimal characters.
2. The signed UTF-8 message is exactly:

   ```text
   wispnet-command-link-v1
   <command center origin without trailing slash>
   <full host endpoint ID>
   <host name>
   <nonce>
   ```

3. The response contains a secret `device_code`, human `user_code`,
   `verification_url`, `expires_in: 600`, and `interval: 5`.
4. The user opens that URL, signs in with GitHub, checks the named computer/code,
   and explicitly approves. Session-authenticated
   `POST /api/command/links/approve` takes `user_code`.
5. The node polls `POST /api/command/links/poll` with `device_code`. Pending is
   `{state:"pending"}`. The first successful poll returns
   `{state:"linked",publisher_token,account:{id,name}}` and consumes the code.
   The node must save the publisher token atomically; a lost successful response
   requires a fresh link. Publisher tokens are stored only as hashes here.

An account cannot take over a live host linked to another account. A host may
be linked again by the same owner; explicit unlink permits a subsequent owner
transfer using a new device proof.

## Host APIs

These require `Authorization: Bearer <publisher_token>`:

- `POST /api/command/host/register`: `host`, `service`, `incarnation`, `name`,
  `resource`. The resource is the exact HTTPS MCP endpoint URL, including its
  actual case-sensitive path. Its hostname must equal Wispnet's existing
  generated hostname for the supplied owning host, name, and service. Returns
  `ok`, `resource`, `resource_id`, and `auth_mode` (`oauth`, `bearer`, or `both`).
- `POST /api/command/authorize`: the same `host`, `service`, `incarnation`,
  `resource`, plus the caller's `token`. A successful response contains
  `allowed:true`, `subject`, and `lease_seconds:5`. Both the edge and hosting node
  can call this operation. It checks current account/service grants on every
  request and accepts neither a publisher credential nor a generic account
  session as a visitor token.
- `POST /api/command/host/unlink`: `host`. Revokes its publisher registration,
  public service generations, keys, connections, and refresh credentials. It
  does not modify private device policy.

## Public access and dashboard

The root OAuth discovery document advertises issuer `WISPNET_COMMAND_URL` and
endpoints under `/api/auth/oauth2/`. Authorization-code clients use S256 PKCE and
one registered public resource. Client ID Metadata Documents (CIMD) are supported;
bounded unauthenticated Dynamic Client Registration (DCR) is also available for
compatibility. The supported scopes are `mcp:use` and `offline_access`. Scope
`mcp:use` means access to the whole MCP service, not a claim of per-tool or
per-user application-data isolation.

Public clients use PKCE without a client secret. CIMD clients may authenticate
with RS256 `private_key_jwt` assertions, including ChatGPT's current client
profile. This verifies the client's signed proof; issued access and refresh
tokens remain opaque. DCR registers public clients only. Token, refresh and
revocation requests must include an explicit body `client_id`. Remote client
metadata and JWKS use HTTPS on port 443, checked public addresses and bounded,
redirect-free fetches.

Service owners automatically have public access. Other users register with
GitHub and share their account ID, which the owner grants in the dashboard.
Access requires the current grant even after token issuance. OAuth access
tokens last five minutes; refresh tokens rotate, and refresh issuance checks
the grant again. Token records bind durable service/grant/connection generations.
Disconnecting a client, standard OAuth revocation, unlinking a host, revoking a
grant, replacing a service, and changing authentication mode invalidate the
affected credentials. Regranting does not revive old keys or refresh tokens.

The dashboard can create named bearer keys scoped to one service. Complete keys
appear only in the creation response and are stored as hashes. Revocation
is separate from deleting a local client configuration. The UI creates 30-day
keys; the API accepts expirations from one hour to one year. Keys cannot act as
dashboard sessions or administer publishers.

The hosting node must terminate public streams when their authorization lease
expires or renewal fails. That cancellation is enforced in the Rust integration,
not by the HTTP response from this service. A stopped hub therefore denies
public access while private device access remains independent.

## Removal boundary

Disable or unlink public publishing before stopping this module. Remove its
desktop integration/configuration without touching device identities, private
pairings, service registries, or host-local backend credentials. The command
center's database contains public account/grant state only; it is not a
replacement for a host's private authority.
