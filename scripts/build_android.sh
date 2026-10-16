#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export JAVA_HOME="${JAVA_HOME:-/opt/android-studio/jbr}"
ndk_bin="${WIPSNET_ANDROID_NDK:-$ANDROID_HOME/ndk/27.2.12479018}/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk_bin/aarch64-linux-android34-clang"
export CC_aarch64_linux_android="$ndk_bin/aarch64-linux-android34-clang"
export AR_aarch64_linux_android="$ndk_bin/llvm-ar"
cargo build -p wipsnet-android --target aarch64-linux-android --locked
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/debug/libwipsnet_android.so android/app/src/main/jniLibs/arm64-v8a/
"$ndk_bin/llvm-strip" --strip-debug android/app/src/main/jniLibs/arm64-v8a/libwipsnet_android.so
cd android
./gradlew --no-daemon assembleDebug
