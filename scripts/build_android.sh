#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export JAVA_HOME="${JAVA_HOME:-/opt/android-studio/jbr}"
ndk_bin="${WISPNET_ANDROID_NDK:-$ANDROID_HOME/ndk/27.2.12479018}/toolchains/llvm/prebuilt/linux-x86_64/bin"
android_abi="${WISPNET_ANDROID_ABI:-arm64-v8a}"
case "$android_abi" in
  arm64-v8a)
    native_target=aarch64-linux-android
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk_bin/aarch64-linux-android34-clang"
    export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
    export AR_aarch64_linux_android="$ndk_bin/llvm-ar"
    ;;
  x86_64)
    native_target=x86_64-linux-android
    export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$ndk_bin/x86_64-linux-android34-clang"
    export CC_x86_64_linux_android="$CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER"
    export AR_x86_64_linux_android="$ndk_bin/llvm-ar"
    ;;
  *) echo 'Supported Android ABIs: arm64-v8a, x86_64' >&2; exit 1 ;;
esac
# NDK r27 needs both flags: segment alignment and RELRO boundaries.
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
android_profile="${WISPNET_ANDROID_PROFILE:-debug}"
android_artifact="${WISPNET_ANDROID_ARTIFACT:-apk}"
case "$android_profile:$android_artifact" in
  debug:apk) cargo_flags=(); gradle_task=assembleDebug ;;
  release:apk) cargo_flags=(--release); gradle_task=assembleRelease ;;
  release:bundle) cargo_flags=(--release); gradle_task=bundleRelease ;;
  *) echo 'Supported builds: debug/apk, release/apk, release/bundle' >&2; exit 1 ;;
esac
cargo build -p wispnet-android --target "$native_target" --locked "${cargo_flags[@]}"
mkdir -p "apps/android/app/src/main/jniLibs/$android_abi"
rm -f "apps/android/app/src/main/jniLibs/$android_abi/libwipsnet_android.so"
cp "target/$native_target/$android_profile/libwispnet_android.so" "apps/android/app/src/main/jniLibs/$android_abi/"
"$ndk_bin/llvm-strip" --strip-debug "apps/android/app/src/main/jniLibs/$android_abi/libwispnet_android.so"
shared_version="$(python3 -c 'import runpy; print(runpy.run_path("apps/node/desktop/wispnet_desktop/version.py")["VERSION"])')"
shared_code="$(python3 -c 'import sys; a,b,c=map(int,sys.argv[1].split(".")); print(a*1000000+b*1000+c)' "$shared_version")"
# Flutter is the default UI; the native runner remains for migration regression checks.
if [[ "${WISPNET_ANDROID_UI:-flutter}" == flutter ]]; then
  case "$android_abi" in arm64-v8a) flutter_target=android-arm64 ;; x86_64) flutter_target=android-x64 ;; esac
  case "$android_artifact" in apk) flutter_artifact=apk ;; bundle) flutter_artifact=appbundle ;; esac
  flutter_name="${WISPNET_ANDROID_VERSION_NAME:-$shared_version}"
  flutter_code="${WISPNET_ANDROID_VERSION_CODE:-$shared_code}"
  (cd apps/flutter && ../../scripts/flutter.sh build "$flutter_artifact" "--$android_profile" --target-platform "$flutter_target" --build-name "$flutter_name" --build-number "$flutter_code")
  # Keep existing release/publishing scripts' artifact locations stable.
  if [[ "$android_artifact" == apk ]]; then
    mkdir -p "apps/android/app/build/outputs/apk/$android_profile"
    cp -a "apps/flutter/build/app/outputs/apk/$android_profile/." "apps/android/app/build/outputs/apk/$android_profile/"
  else
    mkdir -p apps/android/app/build/outputs/bundle/release
    cp -a apps/flutter/build/app/outputs/bundle/release/. apps/android/app/build/outputs/bundle/release/
  fi
  exit 0
fi
if [[ "$WISPNET_ANDROID_UI" != native ]]; then
  echo 'WISPNET_ANDROID_UI must be flutter or native' >&2
  exit 1
fi
cd apps/android
./gradlew --no-daemon "$gradle_task" "-PwispnetAndroidAbi=$android_abi" "-PwispnetVersionCode=${WISPNET_ANDROID_VERSION_CODE:-$shared_code}" "-PwispnetVersionName=${WISPNET_ANDROID_VERSION_NAME:-$shared_version}"
