#!/bin/sh
# Command Line Tools ships Swift Testing outside the default framework search path.
set -eu
cd "$(git rev-parse --show-toplevel)"
frameworks="$(xcode-select -p)/Library/Developer/Frameworks"
if [ -d "$frameworks/Testing.framework" ]; then
    exec swift test --package-path apps/macos --disable-xctest \
        -Xswiftc -F -Xswiftc "$frameworks" \
        -Xlinker -F -Xlinker "$frameworks" \
        -Xlinker -rpath -Xlinker "$frameworks" \
        -Xlinker -rpath -Xlinker "$(xcode-select -p)/Library/Developer/usr/lib"
else
    exec swift test --package-path apps/macos --disable-xctest
fi
