# ADR 0002: Per-host authority and a disposable relay

Status: accepted architecture; implementation and migration remain tracked by #1 and #8-#19.

## Context and supersession

The owner requires peer-to-peer service access without one computer acting as the registry or permission authority for all others. A shared directory means one assembled user experience, not one source of authority.

This decision supersedes both the cloud-authoritative registry proposal and the subsequent single home authority on Machine 1. Earlier closed issues record useful development foundations, not completion of this migration. ADR 0001's browser-security boundaries remain in force; its registry-administrator role is scoped to the particular host under this decision.

## Decision

Every hosting machine runs the same independent control/service agent. It owns a durable local registry, backend bindings, client approvals, administrative authorization, service grants, and revocations for its own services. Machine 1 and Machine 2 are symmetric. The Android client owns its identity, trusted host pins, local sessions, and cached listings, not other machines' permissions.

A service is identified by `(host EndpointId, local service ID)`. Friendly names may collide. Each host is the sole writer of its records; versions, deletions, and any recovery generation are scoped to that host. No machine may publish authoritative records in another host's namespace. Moving a service to another host does not silently preserve its old identity or transfer grants.

Clients fetch permitted catalogs directly from authenticated owning hosts and merge them locally. Preserve origin, scope, version, and freshness independently per host. No global revision, quorum, consensus database, elected master, or full mesh of simultaneously available nodes is required. Catalog caches are rebuildable views, not host policy stores. Public node-record discovery and gossip are not v1 requirements. Forwarded peer hints cannot establish trust; any future forwarded catalog needs independently verifiable provenance and replay protection.

## Authorization and freshness

The host evaluates the actual authenticated requesting EndpointId and requested local service against its current durable policy before opening a backend. A cached directory entry, client-supplied grant, or another host's approval cannot override it. Local changes atomically update runtime authorization, reject new access, and cancel affected active streams within documented tested bounds.

There is no Machine 1-issued policy lease controlling Machine 2. Local authoritative grants do not expire merely because an unrelated peer is unreachable. Explicit time-limited grants and browser sessions may expire according to that host's chosen policy. Catalog TTLs govern listing freshness, not another host's right to operate. Stale or missing metadata can be refreshed by contacting the known owning host; unknown identities and malformed records remain denied.

After restart, a host loads its own identity/policy and starts without contacting another host. Loss/corruption of that authoritative state fails closed for that host and enters explicit recovery; it is not an excuse to reconstruct approvals from cached listings or an old static development fallback.

## Pairing and administration

Pairing pins the particular target host's EndpointId and relay bootstrap before disclosing invitation secrets. Invitations and signed proof bind the target host, enrolling key, scope, protocol version, nonce, purpose, and expiry. Each target explicitly approves its own client grant. Client trust of a server, server permission for a client, and permission to administer a host are distinct.

v1 supports direct per-host approval, not implicit transitive trust or a mandatory global membership authority. Knowing a key, using the same relay, or pairing with Machine 1 does not authorize access to Machine 2. Each host has a protected local administration path, so it can approve/revoke clients even when all other computers are off. Remote administrative delegation must be explicit and host-scoped if supported; it is not assumed.

A combined setup or revoke action is a set of independent target-host operations. Show applied, rejected, and pending/offline status per target. Offline targets cannot apply unseen revocations. Do not claim atomic global completion, guaranteed delivery from a lost coordinating client, or promotion of a coordinator to authority. A fresh phone must be pairable with Machine 2 without first contacting Machine 1.

## Connectivity and cloud state

Known peers use their saved target EndpointId and one explicitly configured relay URL in v1, with optional direct hints. Pairing/control/catalog/service protocols share this addressing approach without a service-directory bootstrap cycle. The proposed relay hostname is `relay.wispnet.app`; provisioning is a separate task.

The relay holds transient connections and forwards encrypted packets. It is not an application TLS terminator, service catalog, grant authority, enrollment database, or durable node-discovery server. Peer identities and payload authorization do not depend on the relay's opinion. No host private key or administrative credential belongs on it. Infrastructure TLS/DNS/admission configuration must be reproducible outside the disposable VM; metadata visibility and logging/resource limits are documented separately.

Destroying/rebuilding the relay and disk must not reset any host's state or require re-pairing. Connections can be interrupted and must not silently replay backend operations. One relay remains a v1 connectivity dependency. Redundant relays are future work and must preserve per-host authority; no claim of universal failure-free service is made.

## Failure isolation and recovery

Turning off Machine 1 affects its services, not Machine 2's ability to start, serve, change its services, or approve/revoke clients. Repeat symmetrically. Each service still needs its own host/backend; application replication is not supplied by this architecture.

Each host separately backs up its identity, policy/registry, administration credentials, backend configuration, and version/recovery metadata. Recovering one host must not reset other peers. An old backup cannot establish later revocations without an additional trusted recovery source or explicit owner reconciliation. Protected restore must detect/reconcile rollback where possible and otherwise stay closed for that host, not trust client heartbeats or silently issue a new identity. Never run two independent writers with a cloned active identity.

## Implementation mapping

- #16: independent host registry/control transport, local administration, schema/config migration, and deeper documentation.
- #17: pairwise trust, target-bound enrollment, host-scoped grants, and explicit multi-host operation results.
- #8: local service registration and composite host/service namespace.
- #9: per-host catalog synchronization and current host-local authorization/revocation, replacing centralized leases.
- #10/#11/#12: gateway, Linux, and mandatory Android experience without a master dependency.
- #13/#18: explicit-relay transport, disposable deployment, and reconnect recovery.
- #19: isolated per-host backup, restore, and trust repair.
- #14/#15: symmetric independence/security/recovery tests and release packaging.

## Required evidence

Power off either computer, restart the other host and phone, access that host's existing services, register/update a service there, and pair a fresh client using only that host's approval. Repeat in both directions and do not impose an unrelated-peer policy timeout. Exercise duplicate local service IDs, forged catalogs, independent catalog versions, host-local revocation, partial multi-host revocation, lost caches, relay destruction/rebuild, and one-host recovery while the other continues operating.

Documentation acceptance is not implementation acceptance. Keep completed evidence, build instructions, and current limitations accurate throughout migration.

## Explicit public publishing extension

[Owner-controlled public access](../public-access.md) is an opt-in extension:
a separate HTTPS/public gateway terminates public application traffic while the
packet relay remains opaque. An owning node alone enables publication of its
current local HTTP binding. Private device grants do not confer publish rights
and are not changed by publishing. The public gateway has a separate identity
and publisher admission list; live authenticated owner connections register
transient public routes. It stores no host owner credentials and cannot authorize
private services. Public traffic trusts the TLS-terminating gateway with plaintext;
this is explicit and separate from private end-to-end encrypted paired traffic.

The optional [Command Center](../command-center.md) is a separate, durable public
authorization service. It stores public accounts, proven publisher links and
public grants only. It cannot create private host grants or enable publication on
a host. The hosting node keeps backend credentials and checks public authorization
before forwarding; private device authorization does not contact the hub.
