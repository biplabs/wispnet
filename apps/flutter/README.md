# Wispnet Flutter UI

Android and Linux desktop use the same Flutter home screen, fonts, machine/service
cards, launch animation, pairing, and settings sheets. Desktop uses the wider
service-card grid already shared with Android; there is no persistent sidebar.
Local services appear in their own section, without a repeated machine name or
status badge. Its Manage button opens service sharing and removal. The Remote
sector has its own pairing management and refresh controls. The device-name gear
opens desktop owner tools in sheets: service sharing, device grants, incoming
pairing requests, public links and durable multi-host owner operations.

The macOS runner and installer use these widgets too, but require native build
and runtime validation on a Mac. Windows/iOS node adapters are not implemented.

## Build and run

Use Flutter **3.47.7** (Dart 3.13.5), Android SDK 36, and the existing Rust/Android
prerequisites. `scripts/flutter.sh` resolves `FLUTTER_ROOT`, `PATH`, then
`~/.local/share/flutter`. The Flutter runner uses the SDK's NDK; the Rust library
continues to use the existing NDK 27 build and 16 KB alignment flags.

From the repository root:

```sh
scripts/build_android.sh
adb install -r apps/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n app.wispnet.android/com.biplabs.wispnet.FlutterMainActivity
scripts/test_flutter.sh
```

The standard Android build/release scripts now build Flutter by default, retaining
existing artifact paths, version identity, upload signing variables, and release
checks. `WISPNET_ANDROID_UI=native scripts/build_android.sh` builds the legacy UI
for regression comparison. Never uninstall/clear the app to migrate it.

## Boundaries and upgrade compatibility

- `lib/src/backend.dart`: UI-facing platform interface, without Android imports.
- `lib/src/theme.dart`, `header.dart`, `cards.dart`, `space.dart`, `sheets.dart`: shared visual components.
- `lib/src/app.dart`: responsive directory and orchestration.
- `android/.../FlutterMainActivity.java`: Android platform adapter. Reuses the
  existing `Native`, `HostStore`, `DeviceProfile`, `AccessService`, and bounded QR
  decoder directly from `apps/android`; the Rust core is unchanged.

The application ID remains `app.wispnet.android`, with the same signing settings,
Keystore alias, private state paths, preferences, host validation, card ordering,
and backup exclusions. Notification taps resolve the package launcher so both
runners work. Flutter never receives the device key. Opening cached services still
requires the Rust client's fresh owner/permission/incarnation check.

The old Java UI remains available inside the legacy runner while parity is
validated. The new launcher and text-share entry point use Flutter; its legacy
activities are not exported. Existing Java-view instrumentation targets the old UI
and does not establish Flutter UI coverage.

## Verification

`test/widget_test.dart` covers service handoff, denied cached entries, failed
pairing/input retention, settings validation, saved ordering, and narrow/wide
layouts. To render review images with the bundled fonts:

```sh
cd apps/flutter
WISPNET_SCREENSHOTS=/tmp/wispnet-flutter ../../scripts/flutter.sh test
```

## Desktop

```sh
scripts/build_flutter_desktop.sh
python3 scripts/install_node_desktop.py   # existing initialized Linux node
# On a Mac: python3 scripts/install_macos.py
```

Flutter desktop bundles include the engine, assets and the private Python adapter.
`desktop/bridge.py` uses the existing `NodeAPI`, `CatalogCache` and `Combined`
operations through private stdin/stdout pipes. It creates no management listener
and never imports the node key. The adapter keeps polling independent of bounded
network operations, checks the socket's owner/permissions and preserves the
existing lifecycle (Go offline stops the node; quitting the UI leaves it running).

Private labels and ordering are imported read-only from Tauri WebKit local storage
and the earlier desktop preference files. New display preferences live in
owner-only `desktop-flutter.json` in the existing state directory; cached catalogs
remain display-only. Opening any service asks its current owner again with the
exact incarnation. The installer stages complete runtimes and switches the
launcher only after the replacement builds, retaining previous Flutter runtimes for recovery.
The old Tauri source remains available for regression comparison, not the default
installed desktop.

Linux build prerequisites: Flutter, Clang, CMake, Ninja, pkg-config, GTK 3 and
Ayatana AppIndicator development libraries. Runtime also requires Python 3.11+
and xdg-open. macOS uses its existing Python runtime, launch agents and menu bar.

`integration_test/desktop_test.dart` runs the actual native Linux window against
an already online node, checks sheets and registry settings, captures review
images, and opens an existing local HTTP service in the real browser. Stop the
installed desktop UI before running (the runner is single-instance), then restart
it afterwards:

```sh
systemctl --user stop wispnet-desktop.service
(cd apps/flutter && ../../scripts/flutter.sh drive -d linux \
  --driver=test_driver/integration_test.dart --target=integration_test/desktop_test.dart)
systemctl --user start wispnet-desktop.service
```

Run fixture-only adapter and widget checks with `scripts/test_flutter.sh`.
Live native coverage is distinct from a wide widget rendering.

For the connected paired debug app, run the upgrade-only device checks:

```sh
python3 scripts/test_flutter_android.py --serial DEVICE_SERIAL
```

This checks identity/profile/host preservation, settings loading, online/offline
controls, real catalogs, and browser handoff. It captures the Flutter layer for
visual review and restores the normal app in a `finally` block. It uses explicit
`adb install -r` and attaches a test driver to the installed app; it never invokes
Flutter's uninstall-on-install-failure fallback.

## Android visual reference

The header ports `MainActivity.deviceHeader` and the docked branch of
`SpaceToggle.onDraw`: settings follows the intrinsic device-label width; the
control has the native 6 dp downward offset, 38 dp pill within a 44 dp control,
layered cyan indicator, and original fill/border colors. Its fixed caption uses
the native measured advance and baseline because Android's hinted text metrics
differ from Flutter's fractional layout. The reference phone (density 1.875)
renders the control at 222 × 83 pixels. Widget checks cover this reference size,
settings adjacency in the online directory, and narrow/wide layouts. The installed-app
check also captures the isolated control for comparison.

`SpaceChecks.buttonLayout` in the existing Android instrumentation runner renders
the original control to private `files/button-layout.png` and reports its metrics
without changing pairings or services. This is a native reference, not a Flutter
coverage claim. Bundled Roboto 3.005 (2022) Regular/Medium/Bold are static instances
of the reference Android system font (weight 400/500/700, width 100, italic 0),
under the included Apache 2.0 license. Bundling them keeps that typography shared
across future desktop targets.
