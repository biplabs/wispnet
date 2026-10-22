# Arch / AUR binary package

`wispnet-bin` installs the prebuilt Linux x86_64 Flutter desktop and Rust node.
`PKGBUILD.in` is an upstream template, not an AUR submission: generate the final
`PKGBUILD` and `.SRCINFO` only after a real versioned public binary release exists.
There is deliberately no guessed download URL, `SKIP` checksum, or assumed
redistribution license. The source repository may remain private.

## Release and submission

1. Commit and validate the source, build the node with `cargo build --release
   --locked -p wispnet-cli` and desktop with `scripts/build_flutter_desktop.sh`.
2. Build a desktop release with `python3 scripts/package_linux.py --binary
   target/release/wispnet --desktop-binary
   apps/flutter/build/linux/x64/release/bundle/Wispnet`. Use a clean checkout;
   verify the manifest, binary dependencies, source revision and archive checksum.
3. Publish the archive at an immutable/versioned **public HTTPS** download URL.
   GitHub Releases on a private repository are not public downloads. A separate
   public binary-release repository allows the application sources to stay private.
   Include actual redistribution terms and the required third-party notices.
4. On an Arch host with `makepkg` installed, generate a fresh submission directory:

   ```sh
   python3 scripts/prepare_aur.py \
     --archive target/dist/wispnet-linux-x86_64.tar.gz \
     --output /tmp/wispnet-bin-aur \
     --version YOUR_VERSION \
     --url https://YOUR_PUBLIC_RELEASE_URL/wispnet-linux-x86_64.tar.gz \
     --license-id YOUR_LICENSE_IDENTIFIER \
     --license-file /absolute/path/to/distribution-terms
   cd /tmp/wispnet-bin-aur
   makepkg --verifysource
   makepkg
   ```

   Use an Arch version without hyphens (for example `0.1.0` or
   `0.1.0.r123.gabcdef0`). The generator verifies every manifest hash, requires the
   complete Flutter bundle, rejects dirty releases, and pins the archive plus
   all support-file SHA-256 values. It generates `.SRCINFO` with makepkg.
5. Run the disposable-container lifecycle test below against the built package.
   Confirm the URL downloads without GitHub authentication. Submit the generated
   directory to the AUR `wispnet-bin` Git repository using the maintainer's AUR
   account/SSH key. Do not submit the template or private test fixtures.

   AUR hosting credentials and binary distribution permission are separate from
   access to this application's Git repository. Nothing in these tools changes
   repository visibility or automatically publishes an AUR package.

## User workflow

After publication: `paru -S wispnet-bin`. The resulting package is managed by
pacman. Files live under `/opt/wispnet`, `/usr/bin`, `/usr/lib/systemd`, and
`/usr/share`; nothing is installed into another user's home by pacman.

Run `wispnet-setup` as the regular user, or `wispnet-setup --headless`. Fresh setup
creates one private host identity and configures the Wispnet and n0 relays.
Existing config/state is reused, never reset. Missing/broken existing authority
fails setup and requires explicit repair. To opt into the firewall helper:

```sh
sudo wispnet-setup --firewall-uid "$(id -u)"
```

This enables the packaged root helper, authorizes only that user, and checks that
it becomes healthy. The node remains an unprivileged user service. Boot without
login still requires administrator-configured user linger.

A prior standalone installation overrides system user units. Run its documented
uninstaller first; that preserves config/state/identity. Package setup refuses to
overwrite those custom units or a standalone firewall helper. Re-run package
setup and the firewall command after migration. Do not run the standalone
installer against a pacman-owned installation.

`sudo pacman -R wispnet-bin` (or `paru -R wispnet-bin`) runs the installed
`PreTransaction`/`AbortOnFail` ALPM hook. It stops active package-owned user
services, disables/stops the root helper, removes only its verified chains and
jump rules, and deletes its generated UID configuration. Cleanup failure aborts
removal so an administrator can investigate. A foreign rule in `WISPNET-UDP` is
never flushed. Pacman then removes its tracked files. User identity, pairing,
settings, and state remain for reinstall; unrelated UFW rules are untouched.
User service enablement links may remain for reinstall, but reference absent
units while the package is removed.

Upgrades preserve helper configuration and identity, restart an already running
helper, and leave service enablement unchanged. Restart user services to load
updated application binaries.

## Validation

```sh
python3 -m unittest discover -s scripts -p test_uninstall.py
python3 -m unittest discover -s scripts -p test_arch_package.py
python3 scripts/test_arch_lifecycle.py --package /path/to/wispnet-bin-VERSION-1-x86_64.pkg.tar.zst
```

The unit suite verifies manifest rejection, pinned checksums/metadata, real
makepkg output, state retention and failure handling. The opt-in lifecycle test
uses a disposable `archlinux:base` Docker container with its own network namespace
and `NET_ADMIN`, actual pacman install/upgrade/remove and actual iptables rules.
It verifies that foreign-rule cleanup failure aborts package removal, successful
removal cleans both IP families, and unrelated SSH rules and user identity survive.
Systemd calls are recorded by a stub because systemd is not PID 1 in that test;
it is not a reboot or real systemd startup test. Live machines are untouched.
