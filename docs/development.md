# Development foundation (#3)

The Rust workspace provides configuration checking and Linux agent/registry
lifecycle shells with persistent Linux device identity and an opt-in authenticated
Iroh service host/raw client. Enrollment, registry persistence, the directory and
production browser gateway remain unimplemented. See the
[identity guide](identity.md) for storage, recovery, and Android strategy.

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| wipsnet-core | Platform-neutral configuration, service protocol and future policy types |
| wipsnet-agent | Agent lifecycle, identity and Iroh tunnel; future browser gateway |
| wipsnet-registry | Registry lifecycle and future authenticated API/persistence |
| wipsnet-cli | Linux CLI, structured logging, and OS signals |

## Build and verify

Develop directly on main and push to origin/main. Install the local pre-push
hook once per checkout:

```sh
./scripts/install-hooks.sh
./scripts/test.sh
git push
```

The hook runs formatting, Clippy, unit tests, a debug build, CLI lifecycle/tunnel
smoke tests, and the disposable browser-boundary suite. Install uv and Python 3.11+ for that suite.
It blocks a push on failure, a dirty working tree, or a branch other than main,
so checks cover the commit being pushed. Fresh clones must install the hook;
Git does not enable versioned hooks automatically. GitHub Actions is not used.
Release builds and Android checks below remain available as manual checks.

Install rustup and a Linux C toolchain. Rust 1.95.0, rustfmt and Clippy are pinned
in rust-toolchain.toml. Direct dependencies use exact versions; Cargo.lock pins
the resolved graph. The supported development host is x86_64 Linux.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke.py
python3 scripts/tunnel_smoke.py
cargo build --workspace --release --locked
rustup target add aarch64-linux-android
cargo check -p wipsnet-core --target aarch64-linux-android --locked
```

The Android check type-checks the current shared core and dependencies. Rust
supports this [Android cross-compilation target](https://doc.rust-lang.org/rustc/platform-support/android.html).
This does not link an APK or validate Iroh/JNI, NDK/API versions, key storage,
or background execution. Android integration and physical-device validation remain
#12 work.

## Run

Copy config/example.toml to an absolute path and edit state_dir to a private
absolute directory, for example /home/YOUR_USER/.local/state/wipsnet/agent.
Use distinct config/state paths for agent and registry.

```sh
mkdir -p "$HOME/.config/wipsnet"
cp config/example.toml "$HOME/.config/wipsnet/agent.toml"
# Edit state_dir before running.
cargo run -p wipsnet-cli -- check --config "$HOME/.config/wipsnet/agent.toml"
cargo run -p wipsnet-cli -- identity-init --config "$HOME/.config/wipsnet/agent.toml"
cargo run -p wipsnet-cli -- status --config "$HOME/.config/wipsnet/agent.toml"
cargo run -p wipsnet-cli -- agent --config "$HOME/.config/wipsnet/agent.toml"
```

Ctrl-C or SIGTERM stops the process. Use registry instead of agent with its own
configuration to run the registry shell. Agent networking is opt-in through a
validated transport section; the default example stays offline. See the
[service tunnel guide](service-tunnel.md) and config/tunnel-host.toml.
There is no installed systemd service yet.

## Configuration and logging

All four TOML fields are required. schema_version must be 1; device_name must
be nonblank, at most 128 bytes and contain no control characters; state_dir
must be absolute, non-root and contain no parent traversal; log_level accepts
error, warn, info or debug. Optional transport/services define static service
bindings/grants; invalid limits, non-loopback backends, bad IDs and duplicate
registrations/grants fail before startup. Unknown fields fail validation. Files
are limited to 64 KiB. No implicit config/state lookup occurs. The check command is read-only.
Invalid configuration fails before state preparation or component startup.

Linux state directories are created as 0700. Existing shared directories and
final symlinks are rejected; parent directories must be under your control.
The Linux identity store also checks ownership; same-UID malicious processes are
not isolated. Agent startup requires an explicitly initialized identity.

JSON logs go to stderr and contain only fixed lifecycle component/event fields
and networking status; a networked agent also reports its public endpoint ID and
actual UDP bind address. Third-party protocol trace targets are excluded.
Configuration, arguments, device names, raw parser errors, keys, enrollment/browser
tokens and HTTP payloads are not logged. Configuration
errors describe the expected schema without echoing input. RUST_LOG is ignored.

The smoke test starts/stops both shells with SIGINT and SIGTERM, checks private
state permissions and sensitive-input errors, and verifies no sockets are owned.
The separate tunnel suite checks actual Iroh listener/task cleanup, socket reuse,
stream cancellation and two-process streaming. Internet/relay, native Android and
production browser gateway checks remain future release work. The pre-push hook
runs the local Linux checks; Android and release builds remain explicit manual checks.
