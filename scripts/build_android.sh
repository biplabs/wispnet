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
cd apps/android
./gradlew --no-daemon "$gradle_task" "-PwispnetAndroidAbi=$android_abi" "-PwispnetVersionCode=${WISPNET_ANDROID_VERSION_CODE:-1}" "-PwispnetVersionName=${WISPNET_ANDROID_VERSION_NAME:-0.1.0}"
