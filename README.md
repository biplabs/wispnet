# Wipsnet

**Your services. Any device. Wherever you are.**

Project domain: **`wipsnet.app`** · [v1 roadmap](https://github.com/biplabs/wispnet/issues/1) · [Implementation issues](https://github.com/biplabs/wispnet/issues)

Wipsnet is a project for a shared directory of your self-hosted services, with secure access from your computers and phone. Find a service by name and open it without tracking machine IP addresses, configuring individual router port forwards, or making the service public.

> **Home is the authority. The cloud is disposable.** Machine 1 runs the authoritative registry. Each device keeps its own identity and an authorized local directory/policy copy. The cloud runs only a transient connection relay: no Wipsnet registry, enrollment, permissions, or durable node-discovery database lives there.

> **Status: initial implementation and architecture migration.** The repository contains working development foundations, documentation, and an issue backlog, not a complete v1 release. The architecture and recovery guarantees below are required v1 behavior, not claims that the migration or cloud deployment is already complete. There is no published installer, stable CLI, or production-readiness claim yet. [#16](https://github.com/biplabs/wispnet/issues/16) tracks the home-authority migration; [#18](https://github.com/biplabs/wispnet/issues/18) tracks disposable-relay deployment and recovery validation.

## Why Wipsnet?

You might run a website on one computer and a `llama.cpp` server on another. You should be able to find both in one place and use either from either computer, whether the devices share a Wi-Fi network or one is at a coffee shop. Your phone should have the same access while you are out.

Wipsnet is intended to make **services**, rather than machines and network configuration, the thing you interact with.

| Example service | Hosting device | Intended access |
| --- | --- | --- |
| Personal website | Machine 1 | Either computer or the phone |
| Llama UI and API | Machine 2 | Either computer or the phone |
| Another local web application | Either computer | Any enrolled, authorized device |

The initial target is two Linux computers and an Android phone, with one installed Wipsnet application per device. A computer can both host services and access services on its peers. **An installable Android app that lists and opens services in the normal browser is mandatory for v1**, not a later add-on. Android is initially a client, not a service host or replacement authority.

## The intended experience

1. Initialize the authoritative registry on Machine 1 and configure the shared relay. Explicitly enroll Machine 2 and the Android phone using trusted pairing information.
2. Register a service on its hosting computer using a name and a local backend address. Registration is submitted to Machine 1; Wipsnet will not automatically expose every listening process.
3. See your permitted services in the locally cached directory on each device. Machine 1 supplies authoritative updates; devices do not invent their own permissions.
4. Select a service to open it in the device's normal browser, or obtain a protected local endpoint for an API client.
5. Keep using the same service name as devices move between networks. Wipsnet connects to the known hosting identity using the configured relay and optional direct address hints, with direct paths where available.

The hosting computer must be awake, connected, and running the backend. Wipsnet is not a service-hosting platform or a wake-on-LAN system.

## Architecture: home authority, local copies, transient cloud relay

This architecture **supersedes the earlier proposal to put the authoritative registry in the cloud or alongside the cloud relay**. Machine 1 owns the control plane. The cloud supplies connectivity only.

| Location | Role | Durable Wipsnet state |
| --- | --- | --- |
| **Machine 1 at home** | Sole authoritative registry, service host, and client | Authority/device identity, registry database, owner credentials, enrolled devices, service ownership, permissions, revocations, and local service configuration |
| **Machine 2** | Service host and client | Its own device key, local service configuration, trusted bootstrap, and an authorized directory/policy copy |
| **Android phone** | Client and local browser/API gateway | Its own device key, trusted bootstrap, credentials, and an authorized directory/policy copy |
| **Cloud server** | Iroh relay only | **None of the mesh's authoritative application state.** Live connections and packet-routing state are transient; infrastructure settings must be reproducible separately. |

Machine 1 is the **single writer for trust and policy**. Device copies are read replicas scoped to their roles, not independent authorities, full copies of private credentials, or a multi-writer database. Devices never elect themselves authority or approve a returning peer just because Machine 1 is unavailable.

```text
HOME / ENROLLED DEVICES

Machine 1                         Machine 2                 Android phone
  Authoritative registry            Agent                     Wipsnet app
  Agent + website                   Llama UI/API              Local gateway
  Durable trust/policy              Local directory copy      Local directory copy

  Machine 2 and the phone synchronize with Machine 1 over Iroh.
  Each device can dial an already-known peer using its saved identity.

CLOUD

  Iroh relay only
  Temporary live connections and encrypted packet forwarding
  No Wipsnet service registry or authorization database

  Devices connect outward to the relay. Connections may move to a direct
  peer-to-peer path when available; otherwise they remain relayed.
```

The authority is reachable over authenticated Iroh without publishing a home HTTPS controller or forwarding home-router ports. Optional local administration and browser-facing service HTTPS are separate concerns.

**Service payloads go from the accessing device to the actual hosting agent**, directly or through the encrypted relay path. Machine 1 does not proxy application payloads for Machine 2, and the cloud does not terminate the application connection or decide service permissions.

### How one machine finds another

Every enrolled device retains the permitted peer identities and enough trusted bootstrap information to reach Machine 1. v1 uses **one explicitly configured relay**, with `relay.wipsnet.app` as the proposed hostname. This is a configuration proposal, not confirmation that DNS or a server has been provisioned.

A known peer is addressed using:

```text
Expected peer:  <the peer's public EndpointId>
Initial route: https://relay.wipsnet.app
Direct hints:  optional, not required for initial relay reachability
```

Each running endpoint establishes an outbound relay connection. A sender addresses packets to a known EndpointId, and the relay forwards them through the destination's live connection. The endpoints authenticate each other and attempt a direct path where possible. This is connection rendezvous, not downloading a cloud service directory. The destination must be online and connected to the configured relay for that route to work.

The implementation must not require public Pkarr/DHT records, gossip, a cloud EndpointId-to-IP database, or default public-relay fallback. Ordinary DNS resolution of the configured relay hostname is allowed. Current peer IPs are optional reachability hints and never establish identity or permissions. See [#13](https://github.com/biplabs/wispnet/issues/13).

### Finding the authority before downloading the directory

Pairing supplies the **pinned Machine 1 authority EndpointId**, mesh identity, configured relay address, and a short-lived enrollment invitation. The client authenticates that authority before sending enrollment secrets or fetching private directory data.

This avoids a bootstrap cycle: reaching the registry does not depend on a route or service grant that can only be downloaded from the registry. After enrollment, the persistent authority pin and relay configuration remain usable independently of the spent invitation. Relay DNS, relay TLS, or a different endpoint responding must never silently replace the authority pin. See [#16](https://github.com/biplabs/wispnet/issues/16) and [#17](https://github.com/biplabs/wispnet/issues/17).

## Cloud is disposable

**The cloud server is transient infrastructure, not a source of truth. Its VM and entire disk must be replaceable without restoring a Wipsnet database or re-enrolling devices.**

The relay must not store the service catalog, device approval history, service ownership, access permissions, revocations, enrollment records, owner credentials, or authority private keys. It must not maintain a durable node-discovery database. Those responsibilities stay on Machine 1 and the enrolled devices as described above.

The required recovery behavior is:

1. Destroy the relay VM and its writable disk in an isolated test deployment.
2. Recreate the relay from its deployment recipe, including valid TLS and any externally recoverable infrastructure configuration. Keep the configured relay hostname; its underlying IP may change through DNS.
3. Agents reconnect, rebuilding the relay's temporary live-connection table.
4. Existing device identities, registrations, permissions, and revocations remain intact. No cloud database restoration, new mesh initialization, or device re-pairing is needed.

**Disposable does not mean uninterrupted.** Relayed connections can stall or fail during replacement. Clients must re-resolve DNS and reconnect with bounded backoff, report interruptions, and never silently replay an application request that might already have had side effects.

**Transient does not mean literally zero state or zero metadata.** The relay necessarily handles live sockets, encrypted packets, and temporary routing information. It may observe endpoint identities, addresses, timing, and traffic volume. Payload capture and unnecessary persistent peer metadata must be disabled. Relay acceptance is not permission to use a service; the hosting agent enforces that independently.

DNS/domain control, relay TLS provisioning, abuse/resource limits, and any required deployment or admission configuration remain infrastructure responsibilities. Their recovery must not depend on the destroyed VM being the only copy. They are distinct from Wipsnet's durable application authority, and home authority keys must never be copied to the relay.

[#18](https://github.com/biplabs/wispnet/issues/18) owns deployment and destructive recovery testing. [#14](https://github.com/biplabs/wispnet/issues/14) makes the full recovery behavior a release gate, including proof that a previously revoked test device stays denied after the cloud is rebuilt under the documented policy-validity guarantees.

## Home state is durable; client copies support bounded offline access

**Machine 1's registry is not disposable.** Its database, authority identity, owner credential, and recovery metadata require protected backups outside the relay. A lost authority cannot safely be reconstructed from device heartbeats or by trusting whichever client reconnects first. See [#19](https://github.com/biplabs/wispnet/issues/19).

Each enrolled client persists the directory and policy information it needs. Machine 1, Machine 2, and the phone retain their own identities; copies do not distribute owner credentials or another device's private key.

| Situation | Required v1 behavior |
| --- | --- |
| Machine 1 is available | Refresh local copies, enroll approved devices, and apply authorized registration/policy changes. |
| Machine 1 is temporarily offline | Already-enrolled devices may browse cached services and access Machine 2 under still-valid cached permissions, including after an app/agent restart. |
| Policy is missing, invalid, or expired | Deny data access that needs those permissions, while retaining the ability to reconnect to the pinned authority for fresh policy. |
| A device or grant is confirmed revoked | Do not fall back to an older cache or static development grant. Apply the documented denial/cancellation rules. |
| New enrollment or permission changes are requested while Machine 1 is offline | Wait for Machine 1. Clients and the relay cannot approve or commit these changes. |
| The cloud relay is replaced | Rebuild connectivity, not trust. Device and authority state remain on their respective machines. |

[#9](https://github.com/biplabs/wispnet/issues/9) defines atomic snapshot updates, runtime enforcement, cache validity, and revocation bounds. The numeric offline-access window is not finalized. Revocation cannot be instantaneous at an unreachable host; stale permissions must not remain valid indefinitely. Temporary reachability announcements must not extend a grant's validity or resurrect a removed service.

### Example: opening Llama from the phone

```text
Phone browser
    |
    | Device-local connection
    v
Wipsnet local gateway + valid cached directory
    |
    | Authenticated, encrypted Iroh connection
    | Direct when possible; relayed when necessary
    v
Machine 2 hosting agent + valid local policy
    |
    | Host-local connection
    v
llama.cpp backend
```

The phone resolves Llama's stable service ID from its authorized local directory, connects to the approved Machine 2 endpoint, and requests that service ID. Machine 2 independently authorizes the authenticated caller before opening its configured local backend.

No cloud catalog or live authority query is required for each browser request. Machine 1 remains the policy authority, not an intermediate application gateway. A service hosted on the accessing computer should use a local path while preserving policy and browser-boundary checks. Remote callers cannot supply arbitrary upstream addresses.

## Browser access boundary

A managed loopback gateway on each accessing device opens services in the normal browser. The [browser compatibility spike](spikes/browser/README.md), [ADR 0001](docs/decisions/0001-browser-access.md), and [evidence matrix](docs/browser-compatibility.md) establish the tested direction: service-specific hostnames and an HTTP-aware session/origin boundary, not an unrestricted local byte proxy.

Two requirements remain explicit:

- Binding to loopback is not sufficient authorization. Unrelated websites must not gain access merely because the browser can reach the local gateway.
- Different local ports are not cookie isolation. Preserve the per-service hostname direction and validate application cookies, redirects, login flows, streaming, WebSockets, and Android handoff.

The service tunnel can be byte-oriented, but the browser-facing gateway must enforce the selected HTTP protections. Production Secure-cookie/prefix, HTTPS, hostname reuse, and application compatibility decisions remain release gates in [#10](https://github.com/biplabs/wispnet/issues/10). The prototype's successful tests do not establish a shipping gateway or complete mobile networking implementation.

`wipsnet.app` is the project domain. It does not imply that private services are publicly published, that the cloud hosts their directory, or that every service must receive a public subdomain. Browser-facing HTTPS and certificate requirements are separate from home-registry transport and must not silently introduce a cloud application gateway.

## Security requirements

These are implementation and release requirements, not claims that a security review has already been completed.

- **Independent device identities and a pinned home authority.** Each installation protects its own persistent private key. Pairing requires explicit approval and proof of possession, authenticates Machine 1, and never copies another device's private key.
- **One authority for policy.** Only Machine 1 commits device approvals, ownership, grants, and revocations. Replica caches, peer announcements, and relay connections cannot create authority. Owner/admin privileges remain separate from ordinary enrolled-device access.
- **Authorization at the hosting agent.** Use the authenticated remote identity and current valid local policy. Reject unknown callers, unregistered services, and unauthorized requests before connecting to a protected backend.
- **Host-controlled service bindings.** Backend addresses are configured and validated on the host, not supplied by a remote caller. Stale host configuration cannot resurrect removed or reassigned registrations.
- **A protected local gateway.** Authenticate browser/API sessions and the administrative interface; validate HTTP authority/origin information; test cross-site requests, WebSockets, cookie isolation, and listener exposure. Applications must not receive gateway or owner credentials.
- **Bounded cached permissions.** Apply snapshots atomically, persist trust/revision information, reject invalid or rollback policy, and enforce documented expiry and active-stream revocation. Network or authority failures do not enable permissive development fallbacks.
- **Separate infrastructure and authority recovery.** Cloud replacement must not reset permissions. Home restore must not silently re-enable revoked devices from an old backup or replace a pinned authority. If current trust cannot be established, fail closed and require explicit owner-assisted recovery.
- **Safe failure and resource use.** Redact secrets, bound buffering and concurrency, clean up interrupted sessions, and never silently replay requests with possible side effects. Cancelling a connection cannot undo work already performed by a backend.

The local-process trust boundary, authority trust, clock assumptions, and relay metadata exposure must be documented. A trusted device identity is not, by itself, proof of a human user's intent. Existing backend applications may still need their own authentication and authorization.

## v1 roadmap

[Issue #1](https://github.com/biplabs/wispnet/issues/1) tracks the accepted architecture and release. Each implementation issue includes scope, acceptance criteria, priority, and dependencies. Existing completed foundations remain useful; the new transport and recovery behavior must be implemented and tested rather than assumed from their completion.

| Workstream | Tasks |
| --- | --- |
| **Existing foundations** | [#2 Browser spike and threat model](https://github.com/biplabs/wispnet/issues/2), [#3 Rust workspace and local checks](https://github.com/biplabs/wispnet/issues/3), [#4 Persistent identity](https://github.com/biplabs/wispnet/issues/4), [#5 Service tunnel](https://github.com/biplabs/wispnet/issues/5), [#6 Registry storage/API](https://github.com/biplabs/wispnet/issues/6), [#7 Enrollment foundation](https://github.com/biplabs/wispnet/issues/7) |
| **Home-authority migration and early connectivity** | [#13 Known-peer explicit-relay transport](https://github.com/biplabs/wispnet/issues/13), [#16 Home registry over Iroh](https://github.com/biplabs/wispnet/issues/16), [#17 Pinned-authority pairing](https://github.com/biplabs/wispnet/issues/17), [#18 Disposable relay deployment/rebuild](https://github.com/biplabs/wispnet/issues/18) |
| **Shared state and everyday access** | [#8 Home service registration](https://github.com/biplabs/wispnet/issues/8), [#9 Cached policy and revocation](https://github.com/biplabs/wispnet/issues/9), [#10 Browser/API gateway](https://github.com/biplabs/wispnet/issues/10), [#11 Linux directory/access](https://github.com/biplabs/wispnet/issues/11), [#12 Mandatory Android app](https://github.com/biplabs/wispnet/issues/12) |
| **Recovery and release** | [#19 Home-authority backup/recovery](https://github.com/biplabs/wispnet/issues/19), [#14 End-to-end security/recovery validation](https://github.com/biplabs/wispnet/issues/14), [#15 Packaging and documentation](https://github.com/biplabs/wispnet/issues/15) |

Explicit-relay transport (#13) and home-authority composition (#16) are early integration work. Pairing, registration, and cached policy then support the complete Linux/Android workflows. Android scaffolding and relay deployment can proceed in parallel. Unit and security tests belong with their features; #14 combines the evidence for release rather than postponing all testing until the end.

### First-release acceptance target

A website on Machine 1 and Llama on Machine 2 must both be discoverable and usable from both computers and the installable Android app. The walkthrough must cover a shared LAN, separate Wi-Fi, cellular data, and a forced relay path without manually forwarding home-router ports.

The release must also prove:

- Machine 1 is the registry authority and is reached over Iroh without a public home HTTPS controller. Known peers are reachable by identity plus the configured relay without a cloud directory or public node-discovery service.
- Clients persist authorized copies and can restart and use a known Machine 2 service during a bounded Machine 1 outage. Unknown, expired, and confirmed revoked access remains denied.
- The entire test cloud relay and disk can be deleted and rebuilt, including at a new IP under the same hostname, without restoring a Wipsnet database or re-pairing devices. Registrations and policy survive; a previously revoked test device stays denied under the documented bounds.
- Home-authority backup/restore, wrong identities, stale policy, clock rollback, sleeping hosts, backend failures, browser security, streaming, WebSockets, Android backgrounding, and network changes have tested behavior.

A fresh installation and the distinct cloud-rebuild/home-backup procedures must be reproducible from published instructions. Destructive tests use isolated fixtures, never a live user's deployment.

### Not default v1 scope

A cloud-authoritative registry, persistent cloud application/discovery database, and a cloud application gateway are outside the accepted design. VPN/TUN integration, arbitrary LAN routing, gossip/DHT discovery, multi-relay fleet discovery, multi-writer registry/leader election, process auto-discovery, enterprise user/group roles, path-level permissions, identity-header injection, and automatic wildcard certificates are deferred. Add a deferred component only for a demonstrated requirement, not simply because it could be useful later.

## Development and migration status

Start with the [roadmap](https://github.com/biplabs/wispnet/issues/1) and follow [AGENTS.md](AGENTS.md) and the [contributor guide](docs/development.md). Inspect the repository before replacing scaffolding. This project works directly on `main` with local pre-push checks; GitHub Actions and mandatory feature branches/PRs are not the current workflow. Link implementation commits to their issues and preserve the existing security/storage tests.

The existing Rust foundation includes configuration checking, agent lifecycle, [persistent Linux identities](docs/identity.md), the [authenticated service tunnel](docs/service-tunnel.md) with fixed loopback TCP backends/static grants, the [persistent authenticated registry API](docs/registry-api.md), and [key-bound invitation enrollment](docs/enrollment.md). Existing browser and native CLI evidence is not equivalent to a completed Android application or an internet/relay deployment.

**Migration distinction:** the pre-migration registry client/configuration and enrollment guides describe an HTTPS-origin controller and an online startup check. That is the existing development path, not the accepted deployment target. #16 and #17 must migrate it to the pinned home authority over Iroh while preserving identity, ownership, and permissions. They also own updating the deeper ADR, configuration, and deployment guides; this README does not make those code changes or mark those issues complete.

Runtime directory/policy synchronization (#9), explicit-relay connectivity (#13), the production browser gateway (#10), the Android app (#12), and the recovery/release gates remain tracked work. Consult each issue for current implementation evidence rather than treating this target architecture as already validated. Development interfaces are not stable.

Keep the implementation focused on the original goal: **one service directory, access from every enrolled device, durable authority at home, and a disposable cloud relay for secure connections across networks.**
