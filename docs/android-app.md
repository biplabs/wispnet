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

The HTTP loopback gateway is a development implementation: isolated random localhost origins, origin/authority checks, HttpOnly session cookies, bounded uploads, and cross-origin redirect rejection. Secure-cookie/prefix behavior, service workers, arbitrary login flows, full HTTP compatibility, and the stricter release security work remain under #10. Streaming responses and WebSocket upgrades are implemented; a physical controlled WebSocket upgrade/echo is covered by the lifecycle harness. Chunked request uploads and Expect requests are currently rejected.

Camera/QR scanning is not implemented; file import and pasted target-bound invitation JSON work on the camera-less tablet. Physical app checks now cover hotspot/home Wi-Fi transitions, relay service restart, screen-off/Doze recovery, fresh pairing with the other node stopped, and active-stream revocation. Independently verified cellular egress for this app run, full relay replacement, broader battery restrictions/Android 15 timeout behavior, and longer-duration lifecycle acceptance remain under #12. Transport-only #13 checks do not establish these app-level results.

## Lifecycle and network recovery checks

The app now invalidates existing loopback sessions when the default Android network changes. An interrupted HTTP request is never replayed by the app. The access setting stays enabled; return to Wispnet and open the service again to create a fresh session. The UI shows network loss/change and actual service state, including a notification Stop action. Network callback processing drains the latest change even when another callback arrives during recovery. Activity recreation does not own or stop the service; process death closes its sockets and requires an explicit Start after reopening.

The repeatable physical test uses a unique temporary host-local HTTP/SSE/WebSocket fixture granted only to the existing approved tablet. It restores Wi-Fi enabled state, screen timeout, USB stay-awake setting, battery simulation, and device idle state; it restarts the relay and removes the fixture grant on exit. With `--force-relay`, both host agents temporarily use relay-only transport; original configuration and normal transport are restored afterward. It does not change host identities, real service grants, or app identity/pairings. Run only when these temporary network/relay interruptions are acceptable:

```sh
# Install rebuilt app and instrumentation APKs first.
python3 scripts/test_android_lifecycle.py --serial DEVICE_SERIAL
# Disable both host agents' direct IP transports for this test, then restore:
python3 scripts/test_android_lifecycle.py --serial DEVICE_SERIAL --force-relay
# Include a human-assisted hotspot -> home Wi-Fi switch:
python3 scripts/test_android_lifecycle.py --serial DEVICE_SERIAL --wait-for-network-switch
# At the printed network-switch barrier, reconnect the tablet, then acknowledge:
touch /tmp/wispnet-network-switch-ready
```

Checks cover an actual WebSocket upgrade/echo, activity recreation, a 40-event SSE response with Chrome foreground and screen off for 30 seconds, forced Doze followed by explicit recovery, Wi-Fi off/on interrupting an accepted side-effecting POST (backend count must remain exactly one), temporary relay restart, notification cancellation, and process force-stop/cold reopening with unchanged identity and pairings. No ADB reverse or TCP forwarding carries application traffic. The temporary backend is a controlled protocol fixture; the separate normal app test checks GIKE, the desktop website, and Llama.

Doze can defer network access; the app does not request a battery-optimization exemption or promise uninterrupted execution. The physical tablet runs Android 14: Android 15's six-hour `dataSync` timeout requires a separate Android 15 device/emulator test. See the official [Doze test procedure](https://developer.android.com/training/monitoring-device-state/doze-standby) and [foreground-service timeout behavior](https://developer.android.com/develop/background-work/services/fgs/timeout).

Observed on the physical Android 14 tablet (2026-10-16): the lifecycle harness passed on the phone hotspot, including a user-assisted switch from `10.153.228.240` to home Wi-Fi `192.168.1.59`. A second pass on home Wi-Fi disabled direct IP transports on both hosts and passed the same lifecycle/relay-restart checks through the public relay. Both runs retained the tablet identity and both host pins; the accepted POST count remained one. In the forced-relay run Wi-Fi interruption/recovery completed in approximately 14 seconds and a five-second relay outage plus recovery in approximately 10 seconds. These are measured checks on this setup, not general latency guarantees. The phone's cellular upstream was not independently reverified during this app run. Both host configurations returned to normal direct-plus-relay transport, all three user services remained registered, and the temporary fixture was removed.

A separate check launched the production debug APK normally with no app instrumentation, opened the desktop website in Chrome, and verified Android's foreground-service process state. The same process and foreground service survived 60 seconds with the screen off, including 15 seconds forced deep Doze. After waking, a fresh Open from Wispnet rendered the desktop site again in Chrome, and the normal app Stop control ended access. This strengthens the instrumented lifecycle evidence but does not establish indefinite execution or behavior on other Android versions/devices.

## Independent pairing and revocation

The host now rechecks durable policy for active service streams approximately once per second, with a bounded policy read. Client revocation, grant removal, service deletion, a changed backend binding, or unreadable policy closes the active stream and its backend. New requests remain subject to the host's current policy. Static fixture configurations are immutable and do not need this monitor.

Authenticated catalog denial has a distinct native error code. The app retains cached catalog provenance while marking a host's denial and disabling its cached Open controls. Pending approval, unavailable/unverified responses, service withdrawal, and a host's explicit denial are distinct states. An outage does not erase an already-known denial or silently promote a pending request. Other hosts continue refreshing independently. Re-pairing is an explicit target-host request and still needs that host's approval and fresh service grants.

The physical permission harness uses a separately installed `com.biplabs.wispnet.peercheck` app sandbox. Clearing that sandbox produces fresh test identities without touching the normal app's Keystore key, pins, or host grants. It temporarily stops only one **node** service at a time; the independent desktop relay keeps running. It creates unique loopback fixture services, removes them, revokes test peers, restores both node services, and uninstalls the test variant afterward.

```sh
scripts/build_android.sh
cd android
ANDROID_HOME="$HOME/Android/Sdk" JAVA_HOME=/opt/android-studio/jbr ./gradlew --no-daemon \
  assemblePeercheck assemblePeercheckAndroidTest -PwispnetTestBuildType=peercheck
cd ..
adb install -r android/app/build/outputs/apk/peercheck/app-peercheck.apk
adb install -r android/app/build/outputs/apk/androidTest/peercheck/app-peercheck-androidTest.apk
python3 scripts/test_android_peers.py --serial DEVICE_SERIAL
```

Checks cover both fresh pairing orderings, browser/cold-start access with the other host offline, independently pending owner approval, wrong host pins, consumed invitation replay, cross-host catalog substitution and rollback rejection, duplicate fixture service IDs, already-active streams at service grant withdrawal and client revocation, stale browser/session denial, cached-entry UI denial, unaffected access to the other host, and denial surviving host restart and a cold app start. Temporary fixture services never receive the normal app's identity or owner credentials.
