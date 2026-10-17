# Android internal-testing build

Live optical camera verification is deferred to the Play Console internal-testing track. Camera-free code, invitation-file and QR-image paths remain available. This guide prepares and validates an artifact; it does not publish to Play or claim that all #12/#10 acceptance is complete.

## Build and inspect

Requirements: the Android build prerequisites in [Android app](android-app.md), SDK platform 36, `uv` for the local suite, and network access on the first bundletool download. Default distribution ABI is arm64-v8a; x86_64 is available for emulator checks.

```sh
# Produce an explicitly unsigned, optimized release AAB:
scripts/build_android_release.sh
# Independent validation of an existing bundle:
python3 scripts/check_android_release.py android/app/build/outputs/bundle/release/app-release.aab
```

Output: `android/app/build/outputs/bundle/release/app-release.aab`. Unsigned output is for inspection, not Play upload. The builder uses Cargo's release profile, target API 36, a non-debuggable Java application, backup disabled, and optional camera hardware. Per-build version overrides are `WIPSNET_ANDROID_VERSION_CODE` and `WIPSNET_ANDROID_VERSION_NAME`; the defaults are `1` and `0.1.0`. Increase the version code for every new Play upload.

The build uses both NDK r27 linker flags for 16 KB LOAD and RELRO alignment, and uncompressed native-library packaging. The checker downloads checksum-pinned Google bundletool 1.18.3 into ignored `target/android-tools`, validates the AAB, reads its actual protobuf manifest/config, and checks the expected application ID, release flags, API target, optional camera, native ABI, ELF LOAD/RELRO boundaries and bundle ZIP-alignment request. It rejects unexpected native libraries and verifies any present bundle signature. Unit regressions for unsafe manifest/alignment/signing cases run through `scripts/test.sh` and the pre-push hook.

Policy references: [Play target API requirements](https://support.google.com/googleplay/android-developer/answer/11926878), [16 KB native alignment and testing](https://developer.android.com/guide/practices/page-sizes), and [bundletool](https://developer.android.com/tools/bundletool). Structural alignment is separate from runtime compatibility.

## Upload signing

Use your chosen upload keystore outside the repository. Supply all four variables through a local secret environment, without placing passwords in command arguments or committed files:

- `WIPSNET_UPLOAD_KEYSTORE`: absolute path to the upload keystore.
- `WIPSNET_UPLOAD_STORE_PASSWORD`.
- `WIPSNET_UPLOAD_KEY_ALIAS`.
- `WIPSNET_UPLOAD_KEY_PASSWORD`.

Then run:

```sh
WIPSNET_ANDROID_VERSION_CODE=2 scripts/build_android_release.sh --signed
python3 scripts/check_android_release.py android/app/build/outputs/bundle/release/app-release.aab --require-signed
```

The signed builder fails before compilation if credentials are missing; partial signing configuration is rejected by Gradle. It never falls back to a debug key. Keystores and signing properties are ignored, but keep real credentials outside the checkout. This work does not create a permanent upload key or access a Play account. See [Play App Signing and upload keys](https://developer.android.com/studio/publish/app-signing).

A Play-signed app cannot update the existing debug-signed tablet APK in place when certificates differ. Keep the current installation intact until the internal-testing switch is intentional; a fresh install loses that sandbox's identity and requires fresh host-specific pairing/approval. Do not uninstall it as a build step. Production migration is not part of this development workflow.

## Internal-testing verification

Once a signed artifact and Play app are available, upload through the internal track and complete the required app/foreground-service/store declarations for the actual behavior. Use the opt-in install link on the intended devices. This repository does not automate that upload or fill declarations speculatively.

Verify live optical scan, permission denial/cancellation and camera-free fallback on the Play-installed app. Repeat independent pairing and the real-service/browser lifecycle checks after fresh installation. Record the version code, Android/device versions and outcomes in #12; camera code being present is not evidence of an optical scan. Remaining public relay/cellular, OEM/long-duration and #10 browser compatibility work remains separate.

## Observed local evidence

On 2026-10-16, ARM and x86 unsigned bundles passed bundletool, manifest and 16 KB ELF/RELRO/ZIP-config validation. A disposable local signing key exercised the signed Gradle build and signature checker successfully, then was deleted; no real upload identity was provisioned. The final distribution artifact was rebuilt unsigned.

An APK generated from the optimized x86 release AAB by bundletool, signed only with the existing local debug certificate for this test, ran on a disposable Android 16/API 36 emulator reporting `getconf PAGE_SIZE = 16384`. The application remained non-debuggable. Matching debug instrumentation exercised protected identity initialization, native access Start, the actual shortened framework timeout, native Stop, explicit foreground restart, and Stop with unchanged identity. No host approval or live tunnel was exercised on this emulator. The test override was restored; this was not a six-hour soak or Play-signed install.

The tablet remains on its existing debug certificate and protected identity. API 36 targeting and explicit system-bar/cutout/keyboard insets are included in the rebuilt app; the tablet's Android 14 runtime does not establish Android 16 behavior by itself. Live camera verification is deferred as requested.

Release lint passed with warnings for the pinned tool/library versions, ARM-only distribution/ChromeOS ABI coverage, explicit backup-rule resources, missing launcher artwork and hard-coded UI text. Those warnings were not suppressed or presented as Play acceptance. Store artwork/declarations and remaining UI polish are still preparation work.

The rebuilt API 36-targeted debug APK, using byte-for-byte the optimized ARM native library from the release AAB, passed the physical tablet normal-app harness: preserved identity/pairings, real GIKE, desktop WispShell, Llama chat UI and streamed API reply, gateway denial cases, 30-second browser handoff and Stop. The emulator settings and disposable AVD were removed afterward; SDK packages remain available for repeat builds.
