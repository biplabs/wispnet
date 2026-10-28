#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(python3 scripts/build_identity.py)
case "$(uname -s)" in
 Linux) platform=linux; bundle=apps/flutter/build/linux/x64/release/bundle ;;
 Darwin) platform=macos; bundle=apps/flutter/build/macos/Build/Products/Release/Wispnet.app/Contents/MacOS ;;
 *) echo 'Build on Linux or macOS; Windows node integration is not implemented.' >&2; exit 1 ;;
esac
(cd apps/flutter; ../../scripts/flutter.sh build "$platform" --release --dart-define="WISPNET_BUILD_ID=$version")
cp apps/flutter/desktop/bridge.py apps/flutter/desktop/desktop_data.py apps/flutter/desktop/command_center.py "$bundle/"
cp apps/node/desktop/icon.png "$bundle/"
mkdir -p "$bundle/wispnet_desktop"
for name in __init__.py api.py combined.py catalog_cache.py; do
 cp "apps/node/desktop/wispnet_desktop/$name" "$bundle/wispnet_desktop/"
done
printf '%s\n' "$version" > "$bundle/build-identity"
python3 -c 'import pathlib,sys; print(pathlib.Path(sys.executable).resolve())' > "$bundle/python-path"
if [ "$platform" = macos ]; then
 codesign --force --deep --sign - apps/flutter/build/macos/Build/Products/Release/Wispnet.app
fi
printf 'Flutter desktop bundle: %s\n' "$bundle"
