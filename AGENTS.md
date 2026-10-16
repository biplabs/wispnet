# Development workflow

This is a solo development project. Work on main and push directly to origin/main.
Do not create feature branches or pull requests unless explicitly requested.
Use scripts/install-hooks.sh in fresh checkouts to enable the pre-push hook.
Tests run locally through scripts/test.sh; keep the hook enabled and fix failures
before pushing. GitHub Actions is not used for this project's tests.
