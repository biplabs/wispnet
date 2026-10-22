# Android app development

The shared Flutter UI and its Android platform integration live in `apps/flutter`.
The Java backend, QR scanner, foreground service and resources are in
`apps/flutter/android/app/src/main`; the Rust JNI crate is in
`apps/flutter/android/native`. The old Java UI and its instrumentation runner
have been removed. Application ID, signing and private storage are unchanged.
Use `adb install -r` to preserve identity and paired hosts.

## Build and install

Requirements: Rust with `aarch64-linux-android`, Android SDK platform 36, NDK 27.2.12479018, Java 17+, and an authorized arm64 Android 14+ device. `scripts/build_android.sh` defaults to `$HOME/Android/Sdk` and Android Studio's JBR; override `ANDROID_HOME`, `JAVA_HOME`, or `WISPNET_ANDROID_NDK` as needed. The checked-in Gradle wrapper pins its distribution checksum.

```sh
scripts/build_android.sh
adb install -r apps/flutter/build/app/outputs/apk/debug/app-debug.apk
```

The APK currently targets API 36, builds for arm64 by default (optional x86_64 for emulator acceptance), and uses the configured public relay `https://relay.wispnet.app`.

## Pair and browse

1. Start a host agent on each computer. `scripts/install_app_hosts.py` installs independent `wispnet-host.service` user units on this computer and `desktop.home`, preserving existing identities/configuration. Its node state is separate from the desktop relay state.
2. Issue a target-host invitation with the CLI's `host-invite` command. Verify that computer's public Endpoint ID independently.
3. In Wispnet, paste its pairing code, review the target machine, and request pairing. This proves the tablet identity; it does not grant access yet.
4. On that computer, inspect `host-invitations`, approve the pending invitation using `host-mutate`, and grant the tablet identity access to the intended services. Repeat independently for the other computer.
5. Refresh hosts, select **Start browser access**, then select a service's **Open** button. Its owning host is shown even when both publish `website`.
6. **Stop browser access**, including the notification action, closes the app's gateway listeners and active connections.

Identity is wrapped using a non-exportable Android Keystore AES-GCM key. Pins and cached catalogs live in private app storage; Android backup is disabled. Imported invitations are cleared after the request. No host owner token or private host key is copied into the app. A corrupt identity envelope fails closed rather than silently replacing the device identity.

Catalog refreshes run independently. Cached listings show their age/status and cannot authorize access: opening fetches the owning host's catalog again and every new tunnel is checked by that host. Catalog rollback or a changed host generation requires explicit attention/re-pairing.


## Verification

Run `scripts/test_flutter.sh` for adapter and widget tests. Build the real Android
app with `scripts/build_android.sh`, then run
`python3 scripts/test_flutter_android.py --serial DEVICE_SERIAL` against an
authorized device. This checks Flutter semantics and real Android handoff;
the retired Java-view test harness is no longer supported.

See [Flutter development and verification](../apps/flutter/README.md),
[release checks](android-release.md) and [service discovery](android-service-discovery.md).
