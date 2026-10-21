#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -m unittest discover -s apps/flutter/desktop -p "test_*.py"
cd apps/flutter
../../scripts/flutter.sh pub get --enforce-lockfile
../../scripts/flutter.sh analyze --no-pub
../../scripts/flutter.sh test --no-pub
