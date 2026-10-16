# Native relay lifecycle validation (#13)

`scripts/relay_smoke.py` runs the real Linux host/CLI and an ephemeral TLS Iroh
relay as separate processes. The default local hook includes its Linux mode.
With `--android`, it additionally runs the same native CLI on an explicitly
selected authorized aarch64 Android device over a temporary ADB TCP reverse.
The device creates its own private identity; no host key or owner token is copied.

## Run locally

```sh
cargo build --workspace --locked
cargo build -p wipsnet-cli --example relay_fixture --locked
python3 scripts/relay_smoke.py
```

The example is a loopback-only disposable **test fixture**, not the deployed relay
from #18. It generates an ephemeral TLS certificate/key, writes only the public
certificate to a newly created private file, reports its HTTPS origin, and drains
on SIGTERM. SIGUSR1 replaces the relay server instance at the same address using
the in-memory certificate/key, retaining no relay client/application database.
Stopping the fixture discards that TLS key. Nothing is provisioned on the cloud.

For the authorized Android device, use an installed NDK:

```sh
export WIPSNET_NDK="$HOME/Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_aarch64_linux_android="$WIPSNET_NDK/aarch64-linux-android34-clang"
export AR_aarch64_linux_android="$WIPSNET_NDK/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
cargo build -p wipsnet-cli --target aarch64-linux-android --locked
python3 scripts/relay_smoke.py --android
# When multiple authorized devices exist:
python3 scripts/relay_smoke.py --android --serial DEVICE_SERIAL
```

Use `--ndk /absolute/path/to/ndk` when the strip tool is elsewhere. The default
path matches the NDK example above. The harness refuses an ambiguous/unauthorized
device and uses ADB `reverse --no-rebind` so it cannot replace an existing mapping.
Only a uniquely named `/data/local/tmp/wipsnet-relay-*` directory and its own
mapping are removed afterward. Remote CLI processes also have a 12-second
on-device timeout. No device browser, Wi-Fi, cellular or display setting changes.

Android mode builds the native CLI from current source with that NDK before
copying it to the device, so an older artifact cannot silently pass the hook.
Set `WIPSNET_ANDROID_NDK` to use a different NDK path in the hook.

To include hardware checks in the pre-push hook:

```sh
WIPSNET_ANDROID_RELAY_TEST=1 ./scripts/test.sh
WIPSNET_ANDROID_RELAY_TEST=1 git push origin main
```

The default hook requires no attached device and runs the Linux version. An
explicitly enabled hardware test fails if its build, NDK tools or authorized
device is missing. The hook uses the sole authorized device; use the standalone
`--serial` command when explicit selection is needed.

## What the harness verifies

- Fresh A and B own distinct local identities, owner state, generations and policy.
  Only one host process runs at a time. Linux and Android clients start with no
  directory/controller state and connect using the expected pin plus relay URL,
  never a peer IP hint. Both sides explicitly disable peer UDP transports.
- Unknown clients cannot fetch catalogs or open backends. A's approval cannot
  authorize B. Separate local owner approval and a service grant on each target
  enable catalog and actual HTTP response bytes over the relay. Wrong pins fail
  without reaching the live host backend.
- Client identities survive CLI cold starts; private Android key permissions are
  checked. Host restart preserves policy, generation/revision and catalog.
- A separate endpoint probes offline A while B serves a healthy catalog within
  four seconds. The CLI key lease is respected by using independent identities.
- With the relay deliberately stalled, the host still starts within two seconds
  and its local authority can be read. Remote catalog reachability fails through
  the normal bounded connection path. Resuming/replacing the relay restores a
  read-only catalog probe within a 30-second harness deadline using unchanged
  host pins, catalog and approvals. Service operations are never retried by the
  harness; only read-only catalog recovery probes are retried.
- B's local revocation persists across B restart while the same client can still
  access A. Backend request counts exclude denied accesses and catalog probes.

## Hardware evidence and limits

The authorized tablet (`arm64-v8a`, Android API 34) passed the native relay run
with Wi-Fi disabled, including per-host approval, HTTP service access, host/relay
restart and host-local persistent revocation. The device generated its own key
and temporary device files/USB mappings were removed afterward.

This is native Android CLI/USB evidence. It does not implement or validate an APK,
Android foreground/background lifecycle, Keystore/JNI DNS integration, QR pairing,
Wi-Fi/cellular handoff, a real two-computer power-off test, or physical LAN direct
paths. Both host processes ran on the same Linux computer and were alternately
absent. The relay was a local TLS server reached through USB on Android, not a
cloud deployment or relay-hostname/DNS test on that device. Those remain #12,
#17, #18 and #13/#14 acceptance work; #13 is not closed by this harness.
