# Wipsnet

**Your services. Any device. Wherever you are.**

Project domain: **`wipsnet.app`** · [v1 roadmap](https://github.com/biplabs/wispnet/issues/1) · [Implementation issues](https://github.com/biplabs/wispnet/issues)

Wipsnet is a project for a shared directory of your self-hosted services, with secure access from your computers and phone. Find a service by name and open it without tracking machine IP addresses, configuring individual router port forwards, or making the service public.

> **Status: planning and initial implementation.** The repository contains project documentation, the issue backlog, and an initial Rust development workspace, not a working release. The workflows and capabilities below describe the v1 target. There is no published installer, stable CLI, or production-readiness claim yet.

## Why Wipsnet?

You might run a website on one computer and a `llama.cpp` server on another. You should be able to find both in one place and use either from either computer, whether the devices share a Wi-Fi network or one is at a coffee shop. Your phone should have the same access while you are out.

Wipsnet is intended to make **services**, rather than machines and network configuration, the thing you interact with.

| Example service | Hosting device | Intended access |
| --- | --- | --- |
| Personal website | Machine 1 | Either computer or the phone |
| Llama UI and API | Machine 2 | Either computer or the phone |
| Another local web application | Either computer | Any enrolled, authorized device |

The initial target is two Linux computers and an Android phone, with one installed Wipsnet application per device. A computer can both host services and access services on its peers. Android is initially a client.

## The intended experience

1. Install the Linux agents and Android companion app, and explicitly enroll each device into your private mesh.
2. Register a service on its hosting computer using a name and a local backend address. Registration is explicit; Wipsnet will not automatically expose every listening process.
3. See the same permitted services in the directory on all your devices.
4. Select a service to open it in the device's normal browser, or obtain a protected local endpoint for an API client.
5. Keep using the service by the same name as devices move between networks. Wipsnet resolves its host and attempts a direct connection, with encrypted relay fallback when necessary.

The hosting computer must be awake, connected, and running the backend. Wipsnet is not a service-hosting platform or a wake-on-LAN system.

## Architecture

The v1 design separates the **control plane**, which distributes service information and permissions, from the **data path**, which carries service traffic.

| Component | Responsibility |
| --- | --- |
| Central registry | Device enrollment records, stable service IDs, approved hosting endpoints, whole-service grants, and policy revisions |
| Linux agent | Publish local services, enforce access to hosted services, access peer services, and provide a local browser/API gateway |
| Android app | Enroll the phone, display the directory, manage local access, and keep connections alive during browser handoff |
| Iroh transport and relay infrastructure | Connect accessing agents to hosting agents, using authenticated encrypted transport with direct paths and relay fallback |

```text
                   Central registry
             directory + permissions
                  /       |       \
          Machine 1   Machine 2   Android
             agent       agent      app
               |           |
            Website     Llama UI/API

Service traffic goes from the accessing device to the actual
hosting agent, directly or through an encrypted relay path.
The registry does not proxy application payloads.
```

A small publicly reachable HTTPS registry is the initial deployment proposal. Agents cache authorized directory and policy information for a defined period. The registry and a relay may share a server initially, but they remain separate roles.

### Example: opening Llama from the phone

```text
Phone browser
    |
    | Device-local connection
    v
Wipsnet local gateway
    |
    | Authenticated, encrypted Iroh connection
    | Direct when possible; relay fallback otherwise
    v
Machine 2 hosting agent
    |
    | Host-local connection
    v
llama.cpp backend
```

The phone looks up Llama's stable service ID and approved hosting endpoint. It opens a tunnel to that endpoint and requests the service ID. The hosting agent authorizes the authenticated caller before opening its configured local backend connection.

The client cannot supply an arbitrary upstream address. Machine 1 is not an intermediate application gateway for a service hosted on Machine 2. A service hosted on the accessing computer should use a local path while preserving the applicable access policy.

### Browser access is an early validation task

The starting proposal is a managed loopback gateway on each accessing device. Selecting a service opens a local URL in the normal browser; the agent carries its traffic to the hosting device.

The exact browser-facing URL and authentication scheme are **not finalized**. The compatibility spike must validate the actual website and Llama UI/API, including redirects, cookies, login flows, streaming, WebSockets, and Android browser handoff.

Two requirements are explicit:

- Binding to loopback is not sufficient authorization. Unrelated websites must not gain access merely because the browser can reach the local gateway.
- Different local ports must not be treated as cookie isolation. Applications may require distinct hostnames or another narrowly scoped compatibility solution.

The service tunnel can be byte-oriented, but the gateway must implement any HTTP-aware protections selected by the threat model. See [browser architecture and validation](https://github.com/biplabs/wispnet/issues/2) and [the local gateway](https://github.com/biplabs/wispnet/issues/10).

`wipsnet.app` is the project domain. It does not imply that private services will be published through a public web gateway or that each service must receive a public subdomain. Custom service hostnames, browser HTTPS, and automated certificates will be added only where the chosen access design requires them.

## Security requirements

These are implementation and release requirements, not claims that a security review has already been completed.

- **Independent device identities.** Each installation generates and protects its own persistent private key. Enrollment requires explicit approval and proof of possession; pairing must not copy another device's private key.
- **Authorization at the hosting agent.** Use the authenticated remote identity and current local policy. Reject unknown callers, unregistered services, and unauthorized requests before connecting to a protected backend.
- **Host-controlled service bindings.** Service ownership and backend addresses are configured and validated, not supplied by a remote caller.
- **A protected local gateway.** Protect browser sessions and the administrative interface, validate relevant HTTP authority/origin information, and test cross-site requests, WebSocket origins, cookie behavior, and listener exposure.
- **Bounded cached permissions.** Define a maximum policy age and active-stream behavior. Revocation cannot be promised to be instantaneous on a disconnected host; stale grants must not remain usable indefinitely.
- **Safe failure and recovery.** Apply snapshots atomically, reject invalid or rollback policy, redact secrets, bound resource use, and never silently replay a request with possible side effects after reconnecting.

The local-process trust boundary, controller trust, clock assumptions, and relay metadata exposure must be documented in the threat model. A trusted device identity is not, by itself, proof of a human user's intent. Existing backend applications may still need their own authentication and authorization.

## v1 roadmap

[Issue #1](https://github.com/biplabs/wispnet/issues/1) tracks the release. Each implementation issue includes scope, acceptance criteria, priority, and dependencies.

| Phase | Tasks |
| --- | --- |
| **1. Prove the access path** | [#2 Browser compatibility and threat model](https://github.com/biplabs/wispnet/issues/2), [#3 Rust workspace and CI](https://github.com/biplabs/wispnet/issues/3), [#4 Persistent identity](https://github.com/biplabs/wispnet/issues/4), [#5 Iroh service tunnel](https://github.com/biplabs/wispnet/issues/5) |
| **2. Registry and device trust** | [#6 Registry API](https://github.com/biplabs/wispnet/issues/6), [#7 Enrollment and pairing](https://github.com/biplabs/wispnet/issues/7), [#8 Service registration](https://github.com/biplabs/wispnet/issues/8), [#9 Policy synchronization and revocation](https://github.com/biplabs/wispnet/issues/9) |
| **3. Everyday device access** | [#10 Local browser/API gateway](https://github.com/biplabs/wispnet/issues/10), [#11 Shared directory and Linux workflow](https://github.com/biplabs/wispnet/issues/11), [#12 Android companion app](https://github.com/biplabs/wispnet/issues/12) |
| **4. Reliability and release** | [#13 Relay fallback and network recovery](https://github.com/biplabs/wispnet/issues/13), [#14 End-to-end and security tests](https://github.com/biplabs/wispnet/issues/14), [#15 Packaging and documentation](https://github.com/biplabs/wispnet/issues/15) |

The browser compatibility spike and workspace setup can start in parallel. Dependencies govern feature integration and completion; unit and security tests should be developed alongside the features, not postponed until the final phase.

### First-release acceptance target

A website on Machine 1 and Llama on Machine 2 must both be discoverable and usable from both computers and the Android phone. The walkthrough must cover a shared LAN, separate Wi-Fi, cellular data, and a forced relay path without manually forwarding home-router ports.

The same release must test unauthorized access, lost-device revocation, expired policy, controller outages, sleeping hosts, restart persistence, streaming, WebSockets, Android background behavior, and network changes. A fresh installation must be reproducible from the published instructions.

### Not default v1 scope

VPN/TUN integration, arbitrary LAN routing, gossip discovery, multi-hop application gateways, process auto-discovery, enterprise user/group roles, path-level permissions, identity-header injection, and automatic wildcard certificates are deferred. A deferred component should enter v1 only to resolve a demonstrated requirement, not simply because it could be useful later.

## Development

Start with the [roadmap](https://github.com/biplabs/wispnet/issues/1) and the dependencies in the relevant issue. Inspect the repository state before introducing new scaffolding, and link implementation pull requests to their issues.

The Rust workspace implements configuration checking, agent lifecycle, persistent Linux device identity, and the [SQLite registry storage foundation](docs/registry-storage.md). See the [contributor guide](docs/development.md) for build commands and the [identity guide](docs/identity.md) for initialization, status, storage, and reset. The [authenticated Iroh service tunnel](docs/service-tunnel.md) now supports fixed loopback TCP backends and static peer grants, with a raw development CLI client. Enrollment, registry policy, everyday directory access, and the production browser gateway remain unimplemented. The [browser compatibility spike](spikes/browser/README.md), [ADR](docs/decisions/0001-browser-access.md), and [evidence matrix](docs/browser-compatibility.md) track #2. These development interfaces are not stable; Android integration and packaging remain backlog work.

Keep the implementation focused on the original goal: **one service directory, access from every enrolled device, and secure connections across networks without per-service router configuration.**
