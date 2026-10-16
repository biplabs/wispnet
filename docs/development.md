# Development foundation (#3)

The Rust workspace provides configuration checking and Linux agent/registry
lifecycle shells. Networking, device identity, enrollment, persistence, and service
access remain unimplemented.

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| wipsnet-core | Platform-neutral configuration and future protocol/policy types |
| wipsnet-agent | Agent lifecycle and future transport/gateway capabilities |
| wipsnet-registry | Registry lifecycle and future authenticated API/persistence |
| wipsnet-cli | Linux CLI, structured logging, and OS signals |

## Build and verify

Install rustup and a Linux C toolchain. Rust 1.95.0, rustfmt and Clippy are pinned
in rust-toolchain.toml. Direct dependencies use exact versions; Cargo.lock pins
the resolved graph. The supported development host is x86_64 Linux.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke.py
cargo build --workspace --release --locked
rustup target add aarch64-linux-android
cargo check -p wipsnet-core --target aarch64-linux-android --locked
```

The Android check type-checks the current shared core and dependencies. Rust
supports this [Android cross-compilation target](https://doc.rust-lang.org/rustc/platform-support/android.html).
This does not link an APK or validate Iroh/JNI, NDK/API versions, key storage,
or background execution. Those remain #4/#5/#12 work.

## Run

Copy config/example.toml to an absolute path and edit state_dir to a private
absolute directory, for example /home/YOUR_USER/.local/state/wipsnet/agent.
Use distinct config/state paths for agent and registry.

```sh
mkdir -p "$HOME/.config/wipsnet"
cp config/example.toml "$HOME/.config/wipsnet/agent.toml"
# Edit state_dir before running.
cargo run -p wipsnet-cli -- check --config "$HOME/.config/wipsnet/agent.toml"
cargo run -p wipsnet-cli -- agent --config "$HOME/.config/wipsnet/agent.toml"
```

Ctrl-C or SIGTERM stops the process. Use registry instead of agent with its own
configuration to run the registry shell. Neither mode opens network listeners.
There is no installed systemd service yet.

## Configuration and logging

All four TOML fields are required. schema_version must be 1; device_name must
be nonblank, at most 128 bytes and contain no control characters; state_dir
must be absolute, non-root and contain no parent traversal; log_level accepts
error, warn, info or debug. Unknown fields fail validation. Files are limited to
64 KiB. No implicit config/state lookup occurs. The check command is read-only.
Invalid configuration fails before state preparation or component startup.

Linux state directories are created as 0700. Existing shared directories and
final symlinks are rejected; parent directories must be under your control.
No key store exists yet, and same-UID malicious processes are not isolated.

JSON logs go to stderr and contain only fixed lifecycle component/event fields
and networking status. Configuration, arguments, device names, raw parser errors,
keys, enrollment/browser tokens and HTTP payloads are not logged. Configuration
errors describe the expected schema without echoing input. RUST_LOG is ignored.

The smoke test starts/stops both shells with SIGINT and SIGTERM, checks private
state permissions and sensitive-input errors, and verifies no sockets are owned.
Future networking must extend it to verify actual listener/task cleanup. Current
tests cannot prove shutdown of future tunnels. CI repeats Linux checks and checks
the Android core; local verification does not mean hosted CI has run.
