# macOS desktop

Wispnet uses the shared Flutter UI in `apps/flutter`, with its native macOS
runner in `apps/flutter/macos`. The node runs independently of the desktop.
The private Python bridge and persistence helpers live in `apps/flutter/desktop`;
the shared node API, catalog cache and owner-operation journals remain in
`apps/node/desktop/wispnet_desktop`.

## Build and install

Build on a Mac with Xcode and its command-line tools, Flutter 3.47.7, the pinned
Rust toolchain, and Python 3.11 or newer. The installed app requires macOS 13+.

```sh
scripts/install-hooks.sh
python3 scripts/install_macos.py --init
```

Use `--init` only to explicitly initialize a new independent host. For upgrades,
omit it: configuration, identity, grants and pins are preserved. Missing authority
in an existing installation is an error. `--no-start` installs without restarting
jobs; `--skip-build` uses existing build products. Quit a manually opened app
before upgrading.

The installer builds the Flutter release bundle and installs
`~/Applications/Wispnet.app`, the node under `~/.local/libexec/wispnet-node/`,
and the terminal command `~/.local/bin/wispnet`. It also installs the private
`wispnet-node` owner command for explicitly configured SSH owner operations.
Configuration is `~/.config/wispnet-node/host.toml`; default state is
`~/.local/state/wispnet-node`.

Two per-user launch agents start at graphical login:

- `app.wispnet.node`: the independent node.
- `app.wispnet.desktop`: the Flutter desktop and menu bar.

Logs are in `~/Library/Logs/Wispnet/`. The UI uses the same directory, pairing,
service configuration and owner sheets as Android/Linux. Browser handoff uses
the existing private node API and the macOS system browser. Cached catalogs are
display-only; opening services requires a fresh owner check. Closing the desktop
does not remove the node's identity or permissions.

## Development and validation

```sh
scripts/build_flutter_desktop.sh
scripts/test_flutter.sh
scripts/test.sh
```

The desktop build script bundles the Flutter engine/assets, private Python
adapter and shared owner/cache helpers. The installer stages the app and retains
`Wispnet.previous.app` under `~/Library/Application Support/Wispnet/Backups/`
when replacing an existing installation, keeping backups out of Applications.
Extra launches reveal the existing desktop instead of starting a second copy.
On macOS, left-clicking the menu-bar item opens its menu; right-click has no
action. Choose **Open Wispnet** from the menu to show the desktop window.
Signing is local and ad hoc; notarization and App Store distribution are not
implemented.

The local test suite covers shared Rust protocols, node API integration,
Flutter widgets, desktop adapters, cache validation and owner operations.
See [Flutter development and native verification](../apps/flutter/README.md).
Native macOS build, menu-bar, lifecycle and browser verification require a Mac;
Linux checks do not establish macOS runtime coverage.
