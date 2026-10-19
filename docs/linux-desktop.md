# Linux desktop and KDE tray

The desktop window and tray attach to the running node through its owner-only `node.sock` in the configured state directory. The node retains the one identity lease and Iroh endpoint; UI operations do not start a second endpoint or send owner credentials over the network. Relay and node remain independent roles.

## Build and install

For installation without a checkout or Rust toolchain, use the
[Linux distribution bundle](linux-distribution.md). Files-only staging keeps
first-time initialization explicit and supports headless hosts.

For an already initialized schema-2 node using `~/.config/wipsnet-node/host.toml`:

```sh
cargo build --workspace --locked
python3 scripts/install_node_desktop.py
wispnet-desktop
```

On a graphical host, requires `uv`, an active `graphical-session.target` and Qt's platform dependencies (provided by the target CachyOS/KDE installation). [Qt system-tray support](https://doc.qt.io/qtforpython-6/PySide6/QtWidgets/QSystemTrayIcon.html) provides the KDE StatusNotifier integration. Pinned PySide6 Essentials 6.10.2 is installed into its own user-local virtual environment. The installer atomically updates the development node executable, preserves existing host configuration/identity/grants, restarts the node when its binary changes, installs a desktop launcher and enables the graphical-session tray service. Peer-friendly names are private display metadata saved separately from the durable cryptographic pins. A node restart interrupts current connections; callers explicitly reopen services. Initialization is deliberately separate; the installer does not recreate missing host authority.

Installed GUI files: `~/.local/libexec/wispnet-desktop`, launcher `~/.local/bin/wispnet-desktop`, app entry `~/.local/share/applications/wispnet.desktop`, session service `wispnet-desktop.service`. Existing node service remains `wipsnet-host.service`. Quit the existing desktop first, then run `wispnet-desktop --config /absolute/host.toml` for an alternate existing node. Closing the window hides it to the tray. Quit desktop exits the UI while the node and its active access setting remain running. Relaunching opens the existing window rather than starting another tray process.

The header’s **Go offline** button stops `wipsnet-host.service`, closing the node endpoint, peer connections and local browser routes. The desktop stays running; **Go online** starts the same node and waits for its local API. Identity and grants persist, and the independent relay service is untouched. This lifecycle control uses the installed default node configuration; alternate-config desktops do not control the default service. Browser access has no separate UI switch or success message.

## Headless hosts

The same installer automatically selects node-only mode when `graphical-session.target` is inactive. Force it explicitly with:

```sh
python3 scripts/install_node_desktop.py --headless
~/.local/libexec/wipsnet-node/wipsnet host-status --config ~/.config/wipsnet-node/host.toml
# Query the running node without a second identity lease:
printf '%s\n' '{"op":"catalog","host":"PINNED_HOST_ID"}' | ~/.local/bin/wispnet-node
systemctl --user status wipsnet-host.service
```

This mode installs/enables the node and preserves its existing configuration, identity and grants. It skips Qt, the desktop launcher and tray service; it does not require a display server or a graphical login. The dependency-free `~/.local/bin/wispnet-node` command reads one bounded JSON request from stdin (an empty request returns status) and talks to the running node's owner-only socket. It verifies socket permissions and OS peer credentials. Pairing capabilities and other request material stay out of process arguments. Use operations such as `catalog`, `pair`, `mutate`, `enable`, `open` and `stop` from headless automation; `open` returns a protected browser bootstrap URL for the caller. Treat that URL as a short-lived secret. This reuses the daemon identity/endpoint; the older standalone `host-catalog`/`pairing-request` transport probes require a separate identity lease and are not the live-node management interface. Run the graphical mode later only if a desktop session is actually wanted. For unattended boot, the user service manager must run without a login (for example, an already enabled user linger setting); the installer does not change that host policy.

## Views and actions

- My services: actual host-local backend bindings and client allowlists; share/edit/remove with revision conflict enforcement. Click a local service tile to open it; browser access starts automatically. Owner-local browsing uses the protected gateway and a direct loopback backend connection, without self-pairing, self-grants or relay traffic. It resolves the registered binding from current durable state, refuses missing/corrupt authority, and interrupts active responses on removal or backend replacement. Existing browser routes cannot switch to a different backend binding. Local owner access does not widen the remote client allowlist. Backend addresses remain loopback-only. Editing replaces the complete selected allowlist and requires confirmation.
- Devices: approved client identities and current authenticated inbound QUIC connection counts. Approved devices with a successful catalog or permitted service request in the last 60 seconds remain in the active-client total even after their connection closes. The Activity column distinguishes a live connection, recent activity and idle. Multiple connections from one peer count once in the tray total. These are transport connections, not a count of people, browser tabs, or service sessions. Short catalog checks may end between tray polls. No extra heartbeat traffic is sent. The timer uses the node’s monotonic clock, starts on an authenticated approved catalog or permitted service request, and refreshes at service-stream completion. Live connections count throughout their lifetime. An idle phone with Wisp space enabled disappears after its requests/connections end and the activity window expires; this is recent activity rather than proof of continuously enabled access. Node restart clears the in-memory activity. Revoked clients are excluded from recent activity immediately. Transport stream totals include control traffic.
- Peer services: independently refreshed catalogs from saved target pins; click an available service tile to open it; browser access starts automatically. Private persistent snapshots in `desktop-catalogs/<host-id>.json` retain host provenance, revision, generation and last-refresh time. Restart loads entries only for saved pins, labels them cached, and independently refreshes each host; cached/denied/offline/rejected entries cannot Open. Each remote Open still fetches a live owning-host catalog and the host enforces its current grants. Lower revisions, changed generations and inconsistent same-revision contents remain rejected across desktop restarts until explicit authenticated target re-pairing resets that baseline. The shared native client also persists this evidence separately in `catalog-baselines/`, including for headless callers. Snapshots are bounded to 128 KiB, validated, mode 0600, and atomically replaced in a mode-0700 directory; corrupt/unsafe files are ignored and refetched. Cache write failure is reported. Deleting this display-cache directory leaves pins and local authority intact and permits a fresh authenticated refetch; the native `catalog-baselines/` high-water source remains intact. No LAN scan or central discovery is implied. Empty peers means this node has not paired with another host yet; inbound phone approvals do not grant reciprocal access.
- Pairing requests: approve/reject a specific proved request on this host. Approvals grant no services automatically. Pair with a host accepts a bounded invitation file/code, shows the full host ID for comparison and explicitly requests target-owner approval.

Each remote browser Open rechecks the target's permitted catalog; local Open checks the owner's current registry. Both use the same protected loopback HTTP gateway as Android. A mode-0600 handoff HTML file in the private state directory carries the single-use bootstrap; the browser is launched with its file URI, keeping the bootstrap token out of process arguments. The desktop HTML-file association must point to a web browser (verified on the target CachyOS session). The file is deleted after 60 seconds or normal GUI exit. The gateway validates authority/origin/cookie/bootstrap and expires sessions independently. Taking the node offline closes those local sessions. No shell-interpolated browser command or arbitrary upstream request is accepted.

The tray reports node availability, the active-client total (live connections or approved activity within 60 seconds), approved count, pending requests and configured relay connection state. It never counts saved approvals as activity; recent activity is UI metadata and grants no permissions. This does not inspect a separately operated relay's server process.

## Tests and current limits

`scripts/test.sh` includes real two-node local-socket tests for pairing/pins, target-owner approval, permissions, conflict rejection, browser bootstrap/service access, live connection increase/decrease, recent-activity refresh/expiry and revocation filtering, and native Stop, plus real owner-local HTTP/bootstrap/origin/removal/Stop checks with no self-pin or self-grant, and Qt offscreen tests for device selection, catalog persistence across window restart, rollback/generation rejection, stale Open blocking, local Open routing and token-free browser arguments. Cache regressions cover corrupt/oversized/foreign-host files, permissions, symlinks and deletion independent of pins. All original Rust/relay/browser tests remain local; the pre-push hook runs them. Qt tests use display fixtures; live installation is checked against KDE's registered StatusNotifierItem and actual node services/client records.

The version-1 catalog supplies complete sorted records with service IDs, incarnations, bounded display names and explicit HTTP/TCP types; see [catalog snapshots](catalog-snapshots.md). This desktop uses the existing HTTP browser gateway. Own-service local browsing and persistent desktop catalog caching are implemented. The broader full-power-off/rebuild acceptance under #11 remains separate work. Combined multi-host operation status is implemented below. The second physical computer can be headless; it does not require another desktop GUI. Service incarnation IDs now distinguish removal/recreation even at the identical backend address; see [service lifecycle safety](service-lifecycle.md). Versioned complete snapshots and native durable high-water checks are implemented. A checkout-independent Linux x86_64 bundle and installer are available; see
[distribution](linux-distribution.md). Full release distribution acceptance remains #15. This checkpoint does not close those issues.

## Physical GUI/headless checkpoint — 2026-10-17

Installed the updated desktop/node on CachyOS/KDE (`j-strix`) and the same node on headless Ubuntu 24.04 (`desktop.home` / `j-desktop`). The remote host has no graphical session, no Wispnet tray unit or Qt app installation, and its already enabled user linger setting keeps the node independent of SSH sessions. Both nodes were independently paired and explicitly approved through their own local administration; each peer was added to the relevant existing service allowlists, preserving the phone/tablet grants and both host identities/generations.

- Local GUI → headless host: a Qt interaction check using the installed desktop selected the actual remote `website` row and double-clicked it. The real private handoff HTML opened the remote WispShell site in Chromium with an HttpOnly gateway session. The GUI displayed the authenticated peer catalog and persisted its private snapshot.
- Headless host → local computer: the existing Firefox installation, run headlessly through its existing geckodriver, loaded the actual GIKE and Llama UIs through the headless node's protected browser gateway. The Llama health API returned HTTP 200 through that browser session. No display server, Qt GUI, USB forwarding or SSH port forwarding carried the service traffic.

Browser access returned to its original disabled setting on both nodes; temporary browser profiles, handoff files and test processes were cleaned. The paired peer identities and explicit grants remain for normal use. This proves physical GUI/headless peer browsing in both directions, not machine power-off, deployed relay replacement, a second visible GUI, or a new Llama generation/streaming run.

## Combined owner operations

The **Combined operations** tab adds or revokes a specified client independently on selected hosts and shows one **applied**, **pending** or **rejected** row per host. Adding a client does not grant services automatically. Current service grants and local approvals remain owned by each host. This is an optional owner workflow, not a required controller or delegated mesh-wide administration.

The local host uses its existing private node socket. Other hosts require an explicitly configured, already trusted SSH owner connection; service pairing alone never supplies administrative rights. Store the selected owner routes in the private (mode 0600) `~/.config/wipsnet-node/admin-targets.json`, for example:

```json
[
  {"host": "FULL_LOCAL_HOST_ID"},
  {"host": "FULL_REMOTE_HOST_ID", "ssh": "desktop.home"}
]
```

The UI lists only these owner routes (or just this host if no file exists), requires target selection and confirmation, and shows pending/rejected results without claiming global completion. SSH uses existing strict host-key verification and noninteractive authentication; it does not copy host keys, Wispnet owner tokens, or global credentials. The contacted node checks its own full host ID before touching policy. There is no new public or Iroh admin RPC.

Headless clients use the same workflow without Qt. A private plan file contains an action and its selected targets:

```json
{
  "action": {"op": "revoke", "peer": "FULL_CLIENT_ID"},
  "targets": [
    {"host": "FULL_LOCAL_HOST_ID"},
    {"host": "FULL_REMOTE_HOST_ID", "ssh": "desktop.home"}
  ]
}
```

```sh
chmod 600 /private/revoke-plan.json
wispnet-node --batch /private/revoke-plan.json
wispnet-node --operations
wispnet-node --retry OPERATION_ID
```

For explicit owner approval, use `{"op":"approve","peer":"FULL_CLIENT_ID","name":"Client name"}` as the action. This is the existing owner-local known-key approval decision, not an invitation proof, reciprocal grant or admin delegation. Target service grants still require separate host-owner decisions.

An intent is atomically saved before dispatch in private `combined-operations/` journals. Up to four targets execute independently. An unreachable target, failed owner connection, timeout or unconfirmed reply remains **pending**; a negative host acknowledgement is **rejected**. Only all-applied targets make `complete` true. A batch with rejections can be `finished` without being complete. Retry acts only on pending targets; rejected targets need a newly reviewed operation. No background retries or automatic replay of arbitrary HTTP/application writes occur.

Each host stores the operation ID, immutable action hash and acknowledgement atomically with its own authority mutation in SQLite schema 8 (receipts introduced in schema 6). A repeated ID returns the saved receipt; changing its action fails closed. If a reply was lost after commit, retry confirms the earlier result without re-executing it. An old approval receipt cannot recreate approval after a later local revocation. Applied revisions are historical acknowledgements, not a promise that current permissions still match that earlier action; the serving host's current policy always wins. New-access policy is durable at acknowledgement; affected active streams close under the existing bounded host-policy checks, not through catalog propagation.

Limits are 32 selected hosts, four concurrent attempts, 128 local operation journals and 10,000 retained target receipts. Receipt replay protection is not automatically evicted; hitting its limit rejects new operations. Completed local journals can be archived explicitly without changing host permissions or deleting target receipts. Host recovery must preserve receipts along with authority; stale-backup recovery remains #19 work. Older binaries that support only schema 5/6/7 cannot open schema 8; the new installer preserves identities/configuration rather than resetting authority.

Observed on both physical computers on 2026-10-17: combined approval applied at both hosts; revocation with headless desktop.home stopped showed local applied / remote pending, while its offline registry still contained the client. Pending status survived a local node restart and completed after the remote node restarted, without repeating the local mutation. Already-revoked and wrong-host actions produced explicit rejection. A deliberately discarded real applied reply remained pending; after remote restart and a later revocation, retry returned its earlier applied revision without restoring access or advancing the current revision. The headless CLI also performed a target-local combined operation and reported its rejection. The installed GUI displayed actual applied/pending/rejected journals, including a pending host, and its rendered table was inspected. Tests used a temporary client without service grants; fixture approvals were removed, phone/tablet grants and backend bindings were preserved, and original browser settings were restored.
