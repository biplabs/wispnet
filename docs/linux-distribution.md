# Linux release bundles

The Linux x86_64 bundle contains the native node, dependency-free owner CLI,
optional Qt desktop, service units, locked Python dependencies and setup guides.
Installing it requires no Git checkout, Cargo or Rust toolchain. Each computer
owns its own identity and authority. A headless computer needs no display or Qt.

## Build and verify

On Linux x86_64, with Rust/Cargo, Python 3.11+ and binutils (`strip`, `readelf`):

```sh
python3 scripts/package_linux.py
cd target/dist
sha256sum -c wispnet-linux-x86_64.tar.gz.sha256
tar -xzf wispnet-linux-x86_64.tar.gz
cd wispnet-linux-x86_64
```

The builder compiles the locked release binary and copies an explicit set of
application files. It never copies a host config, private key, registry, token,
virtualenv or catalog cache. `manifest.json` records the source commit, whether
the checkout had changes, required glibc version and each payload SHA-256.
The archive normalizes ownership and timestamps; `SOURCE_DATE_EPOCH` can override
the commit timestamp. `--binary /absolute/prebuilt/wispnet` explicitly packages
an existing native executable instead of building; its provenance must be
established separately. A checksum detects corruption; obtain the archive and
checksum from a trusted release source. It is not a signed release attestation.

Only native Linux x86_64/glibc is packaged. The target must meet the binary's
`minimum_glibc` in the manifest and have libgcc and libm. Other architectures,
musl, macOS and Windows are unsupported by this bundle. All targets need Python
3.11+ and a working systemd user manager. GUI targets additionally need `uv`,
network/cache access for the locked PySide6 6.10.2 dependencies, and the Qt
Wayland/X11 platform libraries for their desktop. The bundle is not an offline
Qt dependency installer. Existing validation targets are CachyOS/KDE and headless
Ubuntu 24.04; a distro-specific graphical dependency installer is not provided.

## Fresh host: initialize explicitly

First verify the archive checksum. Extract into a new directory rather than
overlaying an earlier extraction. Stage the node without enabling or starting it:

```sh
python3 scripts/install_node_desktop.py --headless --no-start
mkdir -p ~/.config/wispnet-node
chmod 700 ~/.config/wispnet-node
cp -n config/node/host.toml ~/.config/wispnet-node/host.toml
chmod 600 ~/.config/wispnet-node/host.toml
```

Edit that config: set a unique device name, an absolute state directory such as
`/home/YOU/.local/state/wispnet-node`, and the intended bind/relay settings.
For the operated public relay, set `relay_url = "https://relay.wispnet.app"`
inside `[transport]`; a direct-only loopback example cannot reach remote hosts.
Use `bind_addr = "0.0.0.0:60303"` for ordinary IPv4 direct access plus relay.
Do not copy another computer's state, identity or owner token.

```sh
~/.local/libexec/wispnet-node/wispnet host-init --config ~/.config/wispnet-node/host.toml
python3 scripts/install_node_desktop.py --headless
systemctl --user is-active wispnet-host.service
wispnet-node
```

`--no-start` only stages files; it does not change service enablement or stop an
already running node. Normal installation enables the existing user service.
Unattended boot requires a user manager that runs without login (for example,
administrator-configured user linger). No display, Qt, relay server or backend
application is installed in headless mode. On an actual graphical computer,
run the installer without `--headless` to also install/start the desktop tray.
Approve clients and register services using the [owner workflow](per-host-control.md)
and [desktop/headless workflow](linux-desktop.md). Pairing does not grant services
or reciprocal/admin access automatically.

## Upgrade, rollback and removal

Verify/extract the new bundle and run its installer in the selected mode. Payload
checksums and native executable compatibility are checked before installed files
change. The binary is atomically replaced, and normal installation restarts the
node only when its binary changes. This interrupts current connections; services
must be reopened explicitly. Config, identities, pins, grants, receipts and
display caches are not rewritten by the installer. No failed startup triggers
automatic reinitialization. Graphical dependency/install failure is reported;
installation is not a transactional rollback of all binaries and service files.

Keep the previous bundle before upgrading. Rollback means reinstalling a verified
older bundle whose binary supports the current SQLite schema. Schema-5/6/7 binaries
cannot open newer schema-10 mutual-pairing and deletion state (which also retains schema-8 service metadata). Never downgrade by deleting the
database or restoring an old grant snapshot: that could revive revoked access.
Protected backup/recovery and stale-grant reconciliation are deferred in #19.
Losing a host identity without a usable original backup requires a new identity
and explicit client re-pairing; other hosts remain unchanged. This package
does not implement recovery or promise arbitrary downgrade compatibility.

To remove program files while retaining private state:

```sh
systemctl --user disable --now wispnet-desktop.service  # graphical installs only
systemctl --user disable --now wispnet-host.service
rm -f ~/.config/systemd/user/wispnet-desktop.service ~/.config/systemd/user/wispnet-host.service
rm -f ~/.local/bin/wispnet-desktop ~/.local/bin/wispnet-node ~/.local/share/applications/wispnet.desktop
rm -rf ~/.local/libexec/wispnet-desktop ~/.local/libexec/wispnet-node
systemctl --user daemon-reload
```

These commands leave config and state in place. To reinstall, use the same config
and intact state; do not run an identity reset as an uninstall step. Deleting only
`desktop-catalogs/` under the configured state directory discards rebuildable
display snapshots, preserving pins and authority; refetch requires the real host.
Deleting authority or private keys is a separate destructive reset that requires
explicit migration/re-pairing. It is not a repair or cache-clear operation.

## Evidence and remaining release work

The local pre-push suite tests extracted-bundle installation with an isolated
home, genuine fresh host initialization, registered service/approval preservation
through reinstall, missing-authority refusal and checksum rejection before any
installed-file change. It never initializes or mutates either live host.
Physical GUI/headless browser and restart evidence is recorded in the desktop
guide. Full clean-OS distribution validation, signed public release delivery,
independent relay replacement and protected host recovery remain separate release
acceptance; this document does not claim they were exercised by the package test.
