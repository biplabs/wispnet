# Android service discovery

Android now uses the same [local HTTP registry and configurable service
ports](service-registry.md) as Linux. The former exported Messenger service,
per-app consent activity and package-specific grants have been removed.

Native apps use the device's existing host/service grants. All website origins are allowed. Browser navigation retains its existing handoff.
The independent `apps/flutter/android/discovery-example` application demonstrates the
HTTP API without sharing Wispnet's UID, code or private data.

Build the independent sample after `scripts/build_android.sh` initializes the runner:

```sh
cd apps/flutter/android
JAVA_HOME=/opt/android-studio/jbr ./gradlew :discovery-example:assembleDebug
```

The former automated discovery harness drove the retired Java UI and has been
removed. The sample remains available for independent HTTP registry checks;
Flutter device checks use `scripts/test_flutter_android.py`.
