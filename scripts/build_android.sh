#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export JAVA_HOME="${JAVA_HOME:-/opt/android-studio/jbr}"
ndk_bin="${WIPSNET_ANDROID_NDK:-$ANDROID_HOME/ndk/27.2.12479018}/toolchains/llvm/prebuilt/linux-x86_64/bin"
android_abi="${WIPSNET_ANDROID_ABI:-arm64-v8a}"
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
cargo build -p wipsnet-android --target "$native_target" --locked
mkdir -p "android/app/src/main/jniLibs/$android_abi"
cp "target/$native_target/debug/libwipsnet_android.so" "android/app/src/main/jniLibs/$android_abi/"
"$ndk_bin/llvm-strip" --strip-debug "android/app/src/main/jniLibs/$android_abi/libwipsnet_android.so"
cd android
./gradlew --no-daemon assembleDebug "-PwispnetAndroidAbi=$android_abi"
