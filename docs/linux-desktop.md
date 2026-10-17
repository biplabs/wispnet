# Linux desktop and KDE tray

The desktop window and tray attach to the running node through its owner-only `node.sock` in the configured state directory. The node retains the one identity lease and Iroh endpoint; UI operations do not start a second endpoint or send owner credentials over the network. Relay and node remain independent roles.

## Build and install

For an already initialized schema-2 node using `~/.config/wipsnet-node/host.toml`:

```sh
cargo build --workspace --locked
python3 scripts/install_node_desktop.py
wispnet-desktop
```

On a graphical host, requires `uv`, an active `graphical-session.target` and Qt's platform dependencies (provided by the target CachyOS/KDE installation). [Qt system-tray support](https://doc.qt.io/qtforpython-6/PySide6/QtWidgets/QSystemTrayIcon.html) provides the KDE StatusNotifier integration. Pinned PySide6 Essentials 6.10.2 is installed into its own user-local virtual environment. The installer atomically updates the development node executable, preserves existing host configuration/identity/grants, restarts the node when its binary changes, installs a desktop launcher and enables the graphical-session tray service. Peer-friendly names are private display metadata saved separately from the durable cryptographic pins. A node restart interrupts current connections; callers explicitly reopen services. Initialization is deliberately separate; the installer does not recreate missing host authority.

Installed GUI files: `~/.local/libexec/wispnet-desktop`, launcher `~/.local/bin/wispnet-desktop`, app entry `~/.local/share/applications/wispnet.desktop`, session service `wispnet-desktop.service`. Existing node service remains `wipsnet-host.service`. Quit the existing desktop first, then run `wispnet-desktop --config /absolute/host.toml` for an alternate existing node. Closing the window hides it to the tray. Quit desktop exits the UI while the node and its active access setting remain running. Relaunching opens the existing window rather than starting another tray process.

## Headless hosts

The same installer automatically selects node-only mode when `graphical-session.target` is inactive. Force it explicitly with:

```sh
python3 scripts/install_node_desktop.py --headless
~/.local/libexec/wipsnet-node/wipsnet host-status --config ~/.config/wipsnet-node/host.toml
~/.local/libexec/wipsnet-node/wipsnet host-catalog --config ~/.config/wipsnet-node/host.toml --peer PINNED_HOST_ID
systemctl --user status wipsnet-host.service
```

This mode installs/enables the node and preserves its existing configuration, identity and grants. It skips Qt, the desktop launcher and tray service; it does not require a display server or a graphical login. The node's owner-only socket, pairing/control/catalog and browser gateway remain available for headless clients and automation. Run the graphical mode later only if a desktop session is actually wanted. For unattended boot, the user service manager must run without a login (for example, an already enabled user linger setting); the installer does not change that host policy.

## Views and actions

- My services: actual host-local backend bindings and client allowlists; share/edit/remove with revision conflict enforcement. Select **Open local service** or double-click a service after enabling Browser access. Owner-local browsing uses the protected gateway and a direct loopback backend connection, without self-pairing, self-grants or relay traffic. It resolves the registered binding from current durable state, refuses missing/corrupt authority, and interrupts active responses on removal or backend replacement. Existing browser routes cannot switch to a different backend binding. Local owner access does not widen the remote client allowlist. Backend addresses remain loopback-only. Editing replaces the complete selected allowlist and requires confirmation.
- Devices: approved client identities and current authenticated inbound QUIC connection counts. Approved devices with a successful catalog or permitted service request in the last 60 seconds remain in the active-client total even after their connection closes. The Activity column distinguishes a live connection, recent activity and idle. Multiple connections from one peer count once in the tray total. These are transport connections, not a count of people, browser tabs, or service sessions. Short catalog checks may end between tray polls. No extra heartbeat traffic is sent. The timer uses the node’s monotonic clock, starts on an authenticated approved catalog or permitted service request, and refreshes at service-stream completion. Live connections count throughout their lifetime. An idle phone with Wisp space enabled disappears after its requests/connections end and the activity window expires; this is recent activity rather than proof of continuously enabled access. Node restart clears the in-memory activity. Revoked clients are excluded from recent activity immediately. Transport stream totals include control traffic.
- Peer services: independently refreshed catalogs from saved target pins; double-click an available service after enabling Browser access. Private persistent snapshots in `desktop-catalogs/<host-id>.json` retain host provenance, revision, generation and last-refresh time. Restart loads entries only for saved pins, labels them cached, and independently refreshes each host; cached/denied/offline/rejected entries cannot Open. Each remote Open still fetches a live owning-host catalog and the host enforces its current grants. Lower revisions, changed generations and inconsistent same-revision contents remain rejected across desktop restarts until explicit target re-pairing resets that baseline. Snapshots are bounded to 128 KiB, validated, mode 0600, and atomically replaced in a mode-0700 directory; corrupt/unsafe files are ignored and refetched. Cache write failure is reported. Deleting this display-cache directory leaves pins and local authority intact and permits a fresh authenticated refetch; the deleted cache is no longer a rollback high-water source. No LAN scan or central discovery is implied. Empty peers means this node has not paired with another host yet; inbound phone approvals do not grant reciprocal access.
- Pairing requests: approve/reject a specific proved request on this host. Approvals grant no services automatically. Pair with a host accepts a bounded invitation file/code, shows the full host ID for comparison and explicitly requests target-owner approval.

Each remote browser Open rechecks the target's permitted catalog; local Open checks the owner's current registry. Both use the same protected loopback HTTP gateway as Android. A mode-0600 handoff HTML file in the private state directory carries the single-use bootstrap; the browser is launched with its file URI, keeping the bootstrap token out of process arguments. The desktop HTML-file association must point to a web browser (verified on the target CachyOS session). The file is deleted after 60 seconds or normal GUI exit. The gateway validates authority/origin/cookie/bootstrap and expires sessions independently. Turning Browser access off closes those local sessions. No shell-interpolated browser command or arbitrary upstream request is accepted.

The tray reports node availability, the active-client total (live connections or approved activity within 60 seconds), approved count, pending requests and configured relay connection state. It never counts saved approvals as activity; recent activity is UI metadata and grants no permissions. This does not inspect a separately operated relay's server process.

## Tests and current limits

`scripts/test.sh` includes real two-node local-socket tests for pairing/pins, target-owner approval, permissions, conflict rejection, browser bootstrap/service access, live connection increase/decrease, recent-activity refresh/expiry and revocation filtering, and native Stop, plus real owner-local HTTP/bootstrap/origin/removal/Stop checks with no self-pin or self-grant, and Qt offscreen tests for device selection, catalog persistence across window restart, rollback/generation rejection, stale Open blocking, local Open routing and token-free browser arguments. Cache regressions cover corrupt/oversized/foreign-host files, permissions, symlinks and deletion independent of pins. All original Rust/relay/browser tests remain local; the pre-push hook runs them. Qt tests use display fixtures; live installation is checked against KDE's registered StatusNotifierItem and actual node services/client records.

The current catalog supplies service IDs, not richer descriptions or explicit web/non-web types. This desktop uses the existing HTTP browser gateway. Own-service local browsing and persistent desktop catalog caching are implemented. The broader full-power-off/rebuild acceptance and combined multi-host operation status under #11 remain separate work. The second physical computer can be headless; it does not require another desktop GUI. Service-ID removal/recreation generation semantics remain #8/#9 work; binding checks do not claim to distinguish recreation at the identical backend address. Package distribution beyond this development user-local installation remains #15. This checkpoint does not close those issues.

## Physical GUI/headless checkpoint — 2026-10-17

Installed the updated desktop/node on CachyOS/KDE (`j-strix`) and the same node on headless Ubuntu 24.04 (`desktop.home` / `j-desktop`). The remote host has no graphical session, no Wispnet tray unit or Qt app installation, and its already enabled user linger setting keeps the node independent of SSH sessions. Both nodes were independently paired and explicitly approved through their own local administration; each peer was added to the relevant existing service allowlists, preserving the phone/tablet grants and both host identities/generations.

- Local GUI → headless host: a Qt interaction check using the installed desktop selected the actual remote `website` row and double-clicked it. The real private handoff HTML opened the remote WispShell site in Chromium with an HttpOnly gateway session. The GUI displayed the authenticated peer catalog and persisted its private snapshot.
- Headless host → local computer: installed Firefox in headless mode, driven locally by its existing geckodriver, loaded the actual GIKE and Llama UIs through the headless node's protected browser gateway. The Llama health API returned HTTP 200 through that browser session. No display server, Qt GUI, USB forwarding or SSH port forwarding carried the service traffic.

Browser access returned to its original disabled setting on both nodes; temporary browser profiles, handoff files and test processes were cleaned. The paired peer identities and explicit grants remain for normal use. This proves physical GUI/headless peer browsing in both directions, not machine power-off, deployed relay replacement, a second visible GUI, or a new Llama generation/streaming run.
