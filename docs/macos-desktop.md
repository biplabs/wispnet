# macOS app (Tauri)

Wispnet for macOS uses Tauri 2, a Rust backend and a bundled HTML/CSS/JavaScript
interface in the system WKWebView, with an always-available menu bar icon.
It shares the Rust node and protocols with Linux and Android; Qt is not part of the installed Mac app. Requires macOS 13 or newer. Build on the Mac
architecture you intend to run; this checkout has been built on Apple Silicon.

## Build and install

Install Xcode Command Line Tools, the pinned Rust toolchain (via rustup), and
Python 3.11 or newer for the installer. Node.js is required for tests only; the
installed app uses Python 3.11+ for the existing private catalog and owner-operation helper; it needs no Node.js, Swift or Qt runtime. From this checkout:

```sh
scripts/install-hooks.sh
python3 scripts/install_macos.py --init
```

`--init` explicitly creates an independent identity only when no host configuration
exists. It configures `https://relay.wispnet.app` and an ephemeral UDP port.
On upgrades omit `--init`; existing configuration, authority, grants and pins
are preserved. Missing authority in an existing installation is an error, never
an instruction to generate a replacement identity. `--no-start` installs files
without starting/restarting jobs; `--skip-build` reuses local build products.
Quit the app before upgrading if it was opened manually.

The installer creates `~/Applications/Wispnet.app`, the node executable under
`~/.local/libexec/wispnet-node/`, and the `~/.local/bin/wispnet` terminal command.
Configuration is `~/.config/wispnet-node/host.toml`; default private state is
`~/.local/state/wispnet-node`. It installs two per-user launch agents:

- `app.wispnet.node`: the independent node, started at graphical login.
- `app.wispnet.desktop`: the Tauri app, started quietly in the menu bar at login.

Logs are in `~/Library/Logs/Wispnet/`. Open Wispnet from Finder, Spotlight or the
menu bar. Closing the window hides it and removes the Dock icon; the menu bar
icon remains. Reopening the app reveals its existing window. **Quit Wispnet**
exits the UI while the node keeps running. **Go offline** unloads the node job
and closes its connections and browser routes. **Go online** starts that same
node again; neither action changes identity or service permissions. Login starts
the node again, just as the enabled Linux user service starts at login.

The content area follows the Linux desktop's drifting 96-star background and
blue nebula, with faster motion online and a 0.9-second hyperjump when opening
local or remote services. The browser opens after the jump finishes; access is
prepared concurrently. Accessible web controls sit above transparent, outlined panels.
Animation pauses while the window is hidden or minimized, and respects the
macOS Reduce Motion setting.

## Services and pairing

**Services** shows local services and independently refreshed catalogs from paired
machines. Each peer reports its own failure; unavailable peers do not block other
catalogs. Only a successful live refresh enables its HTTP services. Opening one
rechecks authority through the shared node and opens the protected local URL in
your default browser. TCP services use the existing terminal tunnel commands.
Private validated display snapshots now persist using the same cache helper as Linux. Cached entries remain disabled until a successful live refresh; the Rust node independently persists authenticated catalog high-water evidence.

**My services** shares or edits a loopback backend and its selected allowed
devices. Changes carry the revision captured when the editor opens, so conflicting
owner edits are rejected. Editing replaces the complete selected allowlist.
**Devices** lists approved clients and their live/recent activity. The menu bar
reports active devices, approvals, pending requests and relay state.

**Pair this machine** shows a five-minute pairing code and its matching QR.
**Copy pairing code** lets you use it on another computer; code and QR disappear
on expiry or receipt of a request. **Pair a machine** accepts a visible multiline
pairing code, shows the full target identity for comparison, and requests its
owner's approval. Invalid or expired codes keep a visible error in the dialog.
Paired hosts awaiting their owner's decision show **Awaiting approval**. **Pairing requests** approves a specific proved request and selected
service grants together. Mutual machine pairing grants no access to the other
machine's services automatically. Device revocation and forgetting a paired host
remain separate owner decisions. The **Owner operations** screen exposes the existing durable SSH owner workflow, including explicit target selection, confirmation, per-host receipts and pending-target retries. Machine and service cards support drag ordering and keyboard arrow ordering; preferences are private to this desktop. **Access settings** configures registry/service gateway ports and allowed website origins. **My services** also controls public access with explicit confirmation.

IPC uses an owner-only Unix socket plus `getpeereid` (macOS) or `SO_PEERCRED` (Linux) to authenticate the local
node. Pairing material stays out of process arguments and logs. Browser handoff
accepts only the node's token-free `http://*.localhost:PORT/` URL and uses
the macOS system browser launcher. The webview loads only bundled assets under a restrictive content security policy;
no shell or filesystem plugin is exposed. No second identity lease or public
management listener is introduced.

## Validation

Run `scripts/test.sh` with Node.js, Python 3.11+, uv, Rust and OpenSSL 3 on
PATH. macOS runs shared Rust/protocol/browser tests, Tauri Rust tests for IPC
permissions/protocol limits and browser URL validation, JavaScript state tests,
and the real two-node integration contract through the Tauri executable’s IPC
implementation. Linux bundle packaging tests run on Linux, where that distribution is supported.
The pre-push hook runs this same local suite.

This is a development app signed locally with an ad-hoc signature. App Store
signing, notarization, universal binaries, and an independent download installer
are outside this implementation.

## Development

The isolated Cargo package is `apps/macos/src-tauri`; static frontend assets live
in `apps/macos/ui`. Its separate committed lockfile keeps Tauri dependencies out
of Linux and Android workspace builds. Linux uses this same Tauri application and its system WebKitGTK; the default Linux installer now installs Tauri. The Qt UI and its runtime dependency have been removed. The shared stdlib Python helper reuses existing private journals and catalog snapshots without importing Qt.

```sh
cargo run --manifest-path apps/macos/src-tauri/Cargo.toml --locked
```

The app reads the existing default host configuration to locate its private node
socket, including custom absolute state directories. `--background` starts in the
menu bar; `--request /absolute/node.sock` reads one JSON request from stdin and
exercises the same IPC transport without starting a GUI. The installer assembles
the `.app` from the release binary, whose web assets are embedded at compile time.
