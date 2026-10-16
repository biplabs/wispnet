#!/bin/sh
set -eu
cd "$(git rev-parse --show-toplevel)"
git config --local core.hooksPath .githooks
git config --local push.default simple
echo "Installed local pre-push tests. Develop on main and push with git push."
