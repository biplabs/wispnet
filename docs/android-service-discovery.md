# Android service discovery

Android now uses the same [local HTTP registry and configurable service
ports](service-registry.md) as Linux. The former exported Messenger service,
per-app consent activity and package-specific grants have been removed.

Native apps use the device's existing host/service grants. Websites require an
explicitly allowed origin. Browser navigation retains its existing handoff.
The independent `apps/android/discovery-example` application demonstrates the
HTTP API without sharing Wispnet's UID, code or private data.

Build and run the isolated physical discovery check:

```sh
scripts/build_android.sh
cd apps/android
JAVA_HOME=/opt/android-studio/jbr ./gradlew -PwispnetTestBuildType=peercheck :app:assemblePeercheck :app:assemblePeercheckAndroidTest :discovery-example:assembleDebug
cd ../..
python3 scripts/test_android_discovery.py --serial AUTHORIZED_ADB_SERIAL
```
