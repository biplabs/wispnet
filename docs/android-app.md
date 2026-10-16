# Android app development build

The native Android app pairs independently with each hosting computer, displays its permitted services, and opens them in Chrome through an app-owned loopback proxy. It does not require a VPN profile. This is a development APK, not a completed v1 release.

## Build and install

Requirements: Rust with `aarch64-linux-android`, Android SDK platform 35, NDK 27.2.12479018, Java 17+, and an authorized arm64 Android 14+ device. `scripts/build_android.sh` defaults to `$HOME/Android/Sdk` and Android Studio's JBR; override `ANDROID_HOME`, `JAVA_HOME`, or `WIPSNET_ANDROID_NDK` as needed. The checked-in Gradle wrapper pins its distribution checksum.

```sh
scripts/build_android.sh
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

The APK currently targets API 35, supports arm64 only, and uses the configured public relay `https://relay.wispnet.app`.

## Pair and browse

1. Start a host agent on each computer. `scripts/install_app_hosts.py` installs independent `wipsnet-host.service` user units on this computer and `desktop.home`, preserving existing identities/configuration. Its node state is separate from the desktop relay state.
2. Issue a target-host invitation with the CLI's `host-invite` command. Verify that computer's public Endpoint ID independently.
3. In Wispnet, enter the machine label and verified ID, and paste or import its invitation JSON. Request pairing. This proves the tablet identity; it does not grant access yet.
4. On that computer, inspect `host-invitations`, approve the pending invitation using `host-mutate`, and grant the tablet identity access to the intended services. Repeat independently for the other computer.
5. Refresh hosts, select **Start browser access**, then select a service's **Open** button. Its owning host is shown even when both publish `website`.
6. **Stop browser access**, including the notification action, closes the app's gateway listeners and active connections.

Identity is wrapped using a non-exportable Android Keystore AES-GCM key. Pins and cached catalogs live in private app storage; Android backup is disabled. Imported invitations are cleared after the request. No host owner token or private host key is copied into the app. A corrupt identity envelope fails closed rather than silently replacing the device identity.

Catalog refreshes run independently. Cached listings show their age/status and cannot authorize access: opening fetches the owning host's catalog again and every new tunnel is checked by that host. Catalog rollback or a changed host generation requires explicit attention/re-pairing.

## Local and physical tests

```sh
scripts/test.sh
# Build the instrumentation APK after building the app:
cd android
./gradlew --no-daemon assembleDebugAndroidTest
cd ..
adb install -r android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
python3 scripts/test_android_app.py --serial DEVICE_SERIAL
# Recheck persisted pairings without issuing new invitations:
python3 scripts/test_android_app.py --serial DEVICE_SERIAL --resume
```

The physical harness requires the host installer metadata at `/tmp/wispnet-app-hosts.json`, the local GIKE service on 8310, local llama.cpp on 8201, and the desktop website on 8080. It explicitly grants this test tablet access to these services; use it only for this intended development setup. Unlock the tablet and keep its screen awake during browser accessibility checks. No ADB reverse is used. The harness retains the approved pairings and cleans invitation files.

Physical validation covers two actual computers, independent approval, duplicate service IDs, GIKE and desktop website browser handoff, Llama browser/API streaming, session denial cases, cold app reopening, 30 seconds of browser foreground handoff, and explicit disconnect. The issue comment records the final observed results.

## Current limits

Foreground access uses Android's `dataSync` service type and notification. It is intentionally stopped at the platform timeout (including Android 15's time limit); process death needs an explicit restart. Service sessions expire after ten minutes, with at most 16 open service listeners and 32 active connections. Bootstrap tokens are single-use and expire after one minute.

The HTTP loopback gateway is a development implementation: isolated random localhost origins, origin/authority checks, HttpOnly session cookies, bounded uploads, and cross-origin redirect rejection. Secure-cookie/prefix behavior, service workers, arbitrary login flows, full HTTP compatibility, and the stricter release security work remain under #10. Streaming responses and WebSocket upgrades are implemented; physical WebSocket acceptance remains to be demonstrated. Chunked request uploads and Expect requests are currently rejected.

Camera/QR scanning is not implemented; file import and pasted target-bound invitation JSON work on the camera-less tablet. App-level unrelated Wi-Fi/cellular transitions, relay rebuild recovery, screen-off/battery restrictions, active-stream revocation, and fresh pairing while the other computer is down remain #12 acceptance work. Transport-only #13 checks do not establish these app-level results.
