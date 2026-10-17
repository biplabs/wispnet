# Linux desktop and KDE tray

The desktop window and tray attach to the running node through its owner-only `node.sock` in the configured state directory. The node retains the one identity lease and Iroh endpoint; UI operations do not start a second endpoint or send owner credentials over the network. Relay and node remain independent roles.

## Build and install

For an already initialized schema-2 node using `~/.config/wipsnet-node/host.toml`:

```sh
cargo build --workspace --locked
python3 scripts/install_node_desktop.py
wispnet-desktop
```

Requires `uv`, a graphical Linux session and Qt's platform dependencies (provided by the target CachyOS/KDE installation). [Qt system-tray support](https://doc.qt.io/qtforpython-6/PySide6/QtWidgets/QSystemTrayIcon.html) provides the KDE StatusNotifier integration. Pinned PySide6 Essentials 6.10.2 is installed into its own user-local virtual environment. The installer atomically updates the development node executable, preserves existing host configuration/identity/grants, restarts the node when its binary changes, installs a desktop launcher and enables the graphical-session tray service. Peer-friendly names are private display metadata saved separately from the durable cryptographic pins. A node restart interrupts current connections; callers explicitly reopen services. Initialization is deliberately separate; the installer does not recreate missing host authority.

Installed GUI files: `~/.local/libexec/wispnet-desktop`, launcher `~/.local/bin/wispnet-desktop`, app entry `~/.local/share/applications/wispnet.desktop`, session service `wispnet-desktop.service`. Existing node service remains `wipsnet-host.service`. Quit the existing desktop first, then run `wispnet-desktop --config /absolute/host.toml` for an alternate existing node. Closing the window hides it to the tray. Quit desktop exits the UI while the node and its active access setting remain running. Relaunching opens the existing window rather than starting another tray process.

## Views and actions

- My services: actual host-local backend bindings and client allowlists; share/edit/remove with revision conflict enforcement. Backend addresses remain loopback-only. Editing replaces the complete selected allowlist and requires confirmation.
- Devices: approved client identities and current authenticated inbound QUIC connection counts. Approved devices with a successful catalog or permitted service request in the last 60 seconds remain in the active-client total even after their connection closes. The Activity column distinguishes a live connection, recent activity and idle. Multiple connections from one peer count once in the tray total. These are transport connections, not a count of people, browser tabs, or service sessions. Short catalog checks may end between tray polls. No extra heartbeat traffic is sent. The timer uses the node’s monotonic clock, starts on an authenticated approved catalog or permitted service request, and refreshes at service-stream completion. Live connections count throughout their lifetime. An idle phone with Wisp space enabled disappears after its requests/connections end and the activity window expires; this is recent activity rather than proof of continuously enabled access. Node restart clears the in-memory activity. Revoked clients are excluded from recent activity immediately. Transport stream totals include control traffic.
- Peer services: independently refreshed catalogs from saved target pins; double-click an available service after enabling Browser access. Session-only caches retain host provenance, revision and generation; denial/offline/catalog rejection blocks Open. Restart refetches from durable pins rather than treating cached data as authorization. No LAN scan or central discovery is implied. Empty peers means this node has not paired with another host yet; inbound phone approvals do not grant reciprocal access.
- Pairing requests: approve/reject a specific proved request on this host. Approvals grant no services automatically. Pair with a host accepts a bounded invitation file/code, shows the full host ID for comparison and explicitly requests target-owner approval.

Each browser Open rechecks the target's permitted catalog and uses the same protected loopback HTTP gateway as Android. A mode-0600 handoff HTML file in the private state directory carries the single-use bootstrap; the browser is launched with its file URI, keeping the bootstrap token out of process arguments. The desktop HTML-file association must point to a web browser (verified on the target CachyOS session). The file is deleted after 60 seconds or normal GUI exit. The gateway validates authority/origin/cookie/bootstrap and expires sessions independently. Turning Browser access off closes those local sessions. No shell-interpolated browser command or arbitrary upstream request is accepted.

The tray reports node availability, the active-client total (live connections or approved activity within 60 seconds), approved count, pending requests and configured relay connection state. It never counts saved approvals as activity; recent activity is UI metadata and grants no permissions. This does not inspect a separately operated relay's server process.

## Tests and current limits

`scripts/test.sh` includes real two-node local-socket tests for pairing/pins, target-owner approval, permissions, conflict rejection, browser bootstrap/service access, live connection increase/decrease, recent-activity refresh/expiry and revocation filtering, and native Stop, plus Qt offscreen tests for device selection, catalog versions, offline state and token-free browser arguments. All original Rust/relay/browser tests remain local; the pre-push hook runs them. Qt tests use display fixtures; live installation is checked against KDE's registered StatusNotifierItem and actual node services/client records.

The current catalog supplies service IDs, not richer descriptions or explicit web/non-web types. This desktop uses the existing HTTP browser gateway. Own-service local fast-path browsing, persistent catalog caching and the broader two-physical-computer/offline/rebuild acceptance under #11 remain separate work. Package distribution beyond this development user-local installation remains #15. This checkpoint does not close those issues.
