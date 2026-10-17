#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
check_args=()
case "${1:-}" in
  '') ;;
  --signed)
    for signing_var in WIPSNET_UPLOAD_KEYSTORE WIPSNET_UPLOAD_STORE_PASSWORD WIPSNET_UPLOAD_KEY_ALIAS WIPSNET_UPLOAD_KEY_PASSWORD; do
      if [[ -z "${!signing_var:-}" ]]; then echo 'Upload signing requires all four documented environment variables.' >&2; exit 1; fi
    done
    check_args+=(--require-signed)
    ;;
  *) echo 'Usage: scripts/build_android_release.sh [--signed]' >&2; exit 1 ;;
esac
export WIPSNET_ANDROID_PROFILE=release WIPSNET_ANDROID_ARTIFACT=bundle
scripts/build_android.sh
python3 scripts/check_android_release.py apps/android/app/build/outputs/bundle/release/app-release.aab --abi "${WIPSNET_ANDROID_ABI:-arm64-v8a}" "${check_args[@]}"
