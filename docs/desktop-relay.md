# Temporary desktop relay, independent of the node role

`desktop.home` temporarily hosts the standalone `wipsnet-relay` binary. It is a
separate process/systemd unit, not the desktop node's registry or transport
endpoint. The pinned Iroh 1.3.0 server authenticates relay clients and forwards
end-to-end encrypted packets. Relay admission never approves an application
client, grants service access, or owns a node identity.

## Installed layout

| Item | Desktop path |
| --- | --- |
| Relay executable | `~/.local/libexec/wipsnet-relay/wipsnet-relay` |
| Relay-only configuration | `~/.config/wipsnet-relay/relay.toml` |
| Public CA certificate (client trust) | `~/.local/share/wipsnet-relay/relay-ca.pem` |
| Server TLS certificate | `~/.local/share/wipsnet-relay/relay-cert.pem` |
| CA private key (desktop only) | `~/.local/share/wipsnet-relay/relay-ca-key.pem` |
| Relay TLS private key | `~/.local/share/wipsnet-relay/relay-key.pem` |
| User service | `~/.config/systemd/user/wipsnet-relay.service` |

The protected relay directories contain no `host.sqlite3`, owner token, node
secret key, service configuration or pairing capabilities. The TLS key is created
on desktop, mode 0600, and never copied to clients or committed. This deployment
uses a temporary private CA and a separate server certificate valid for 30 days. Clients explicitly
trust its public certificate; TLS verification remains enabled. Replace/renew the
certificate before expiry and restart the relay. A WebPKI certificate can replace
it later without changing any node identity or permissions. The installer reuses
existing TLS material and checks its hostname/expiry; it does not silently rotate
trust or import node state.

Install from this checkout with a trusted SSH alias:

```sh
python3 scripts/install_desktop_relay.py --ssh-host desktop.home \
  --hostname relay.wispnet.app --lan-ip 192.168.1.26
```

The installer builds the locked source locally, copies a stripped Linux binary,
and enables only `wipsnet-relay.service` in the SSH user's systemd account. Desktop
has user lingering enabled, so this service can run independently of login. No
sudo is needed. Existing Caddy/WispShell services and node state are untouched.
The source unit is `deploy/relay/wipsnet-relay.service`; systemd applies a 256 MiB
memory limit, restart-on-failure and filesystem hardening. Relay admission is
limited to 64 authenticated connections, with 1 MiB/s incoming rate per connection
and a 2 MiB burst. This is not a claim of pre-authentication DoS protection.
Third-party payload/identity trace logging is not enabled; lifecycle logs contain
fixed role/event labels and the listening address.

## Network and client configuration

The actual DNS name is **relay.wispnet.app**, not the earlier `wipsnet.app` typo.
At installation it resolved to `162.156.47.159`. Reserve desktop's LAN address
`192.168.1.26` and forward **external TCP 443 → 192.168.1.26 TCP 8443**.
The relay listens on `0.0.0.0:8443`; auxiliary HTTP is loopback-only, and relay
QUIC/metrics listeners are disabled. No UDP or port 80 forwarding is required for
this TLS WebSocket relay. Existing desktop listeners on 80/443 are preserved.

Copy only `relay-ca.pem` over trusted SSH to a protected local path on each
client/host. Configure its own node state separately:

```toml
[transport]
bind_addr = "0.0.0.0:7779"
relay_url = "https://relay.wispnet.app"
relay_ca_file = "/absolute/path/to/relay-ca.pem"
relay_only = false
```

Use `relay_only=true` to verify the relay path with no peer UDP transport. There
is no automatic node config rewrite or global authority configuration. Relocating
the relay changes only transport configuration/DNS and TLS trust as necessary;
it does not migrate identities, catalogs, owner approvals or grants.

## Verification and limits

The local pre-push suite builds/tests the relay and checks verified TLS, wrong
hostname rejection, unsafe TLS-key rejection, graceful shutdown and absence of
node/application state. To run the physical application test against this
already-installed relay, build the current Linux and Android CLI first, obtain
the public certificate via SSH, then run:

```sh
python3 scripts/physical_relay_smoke.py --ssh-host desktop.home \
  --serial AUTHORIZED_ADB_SERIAL --relay-url https://relay.wispnet.app \
  --relay-ca-file /absolute/path/to/relay-ca.pem --restart-desktop-relay
```

The existing-relay mode creates no local relay, SSH reverse forward or USB
reverse. Each computer generates independent temporary host/client identities.
The tablet proves its invitation to each host, remains denied until target-local
owner approval, gets a separate service grant, and exercises catalog plus HTTP
bytes. Each host restarts with its own state; the relay restarts independently,
and read-only reconnects must recover within 30 seconds without replacing pins,
permissions or node processes. Fresh Linux clients exercise both cross-computer
catalog/HTTP directions, and tablet revocation remains host-local. Stopping a
node leaves the relay service active. Temporary **node test resources** are
removed; the intentionally installed relay service/TLS files remain.

Android native CLI hostname lookups use the OS `getaddrinfo` resolver without a
JVM, public fallback or TXT peer discovery. The OS owns caching and network DNS
policy; the asynchronous lookup has a three-second deadline. A timed-out OS
lookup may finish later on its blocking worker. This does not implement APK/JNI
lifecycle, Keystore or cellular acceptance.

Tests from these computers/tablet on the home LAN through the public hostname
exercise the router's public address/hairpin forwarding. They do not establish
reachability from an unrelated external network. Desktop's **node** can stop
while its relay keeps running; powering off desktop removes the temporary relay
and therefore breaks relay-dependent paths, the expected single-relay v1
limitation. Whole-machine-off independence requires relocating that relay first.

Inspect/restart/remove only the relay role:

```sh
ssh desktop.home systemctl --user status wipsnet-relay.service
ssh desktop.home systemctl --user restart wipsnet-relay.service
# When relocating/removing the temporary relay:
ssh desktop.home systemctl --user disable --now wipsnet-relay.service
```

Do not stop the node to administer the relay, or stop the relay as part of node
shutdown. Retain TLS files until clients have switched to the replacement relay.

The complete existing-relay scenario passed on 2026-10-16 using the two physical
Linux computers and the arm64 tablet, with `https://relay.wispnet.app` and explicit
CA trust. Both target-local tablet pairing/approval/service cases, both Linux
client directions, host restart, independent relay restart/recovery and local
revocation isolation passed. No SSH/USB traffic forwarding was used. The server
certificate installed during this run expires 2026-11-15; the temporary CA must
also be checked before renewal. Existing WispShell services remained active.
