#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../apps/flutter"
../../scripts/flutter.sh pub get --enforce-lockfile
../../scripts/flutter.sh analyze --no-pub
../../scripts/flutter.sh test --no-pub
