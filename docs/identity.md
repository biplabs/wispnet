# Persistent device identity (#4)

Linux installations use independent Iroh Ed25519 keys. Agent startup only loads
an existing key: it never generates a replacement for missing or damaged state.
First use is an explicit operation:

```sh
cargo run -p wipsnet-cli -- identity-init --config "$HOME/.config/wipsnet/agent.toml"
cargo run -p wipsnet-cli -- status --config "$HOME/.config/wipsnet/agent.toml"
cargo run -p wipsnet-cli -- agent --config "$HOME/.config/wipsnet/agent.toml"
```

Initialization returns an existing valid identity unchanged. Status prints the
configured device name and public endpoint ID, including while the agent runs.
Device name is a display label, not an authenticated human identity. Target-host
pairing, approvals and service grants are implemented; see
[pairing](pairing-bootstrap.md) and [host control](per-host-control.md).

## Linux storage

The platform-neutral IdentityStore contract lives in wipsnet-core; Linux's
FileIdentityStore lives in wipsnet-agent. The state directory must be owned by the
current UID, mode 0700, and not a symlink. Parent directories must be trusted.
Use a local filesystem supporting file locks, atomic file creation and fsync.

identity.key is a versioned 72-byte binary record: eight-byte WIPSKEY1 marker,
32-byte secret seed, and 32-byte public key. Loading checks the length, version,
and seed/public-key consistency. This catches accidental corruption; it is not
tamper protection against someone who can replace both fields.

Key and lock files must be singly linked regular files owned by this UID, mode
0600. Opens reject final symlinks and nonregular files. Mutations use an advisory
file lock; a competing operation fails with a retry message. Initialization writes
a private temporary file, syncs it, commits without replacing any existing key,
then syncs the directory. Raw storage buffers are zeroized on drop. No key export
or credential input is exposed by the CLI, and identities have no Debug/Serialize
implementation. Diagnostics never print raw file contents or underlying errors.

The key is plaintext at rest on Linux, protected by filesystem permissions.
Same-UID processes, root, filesystem backups and process memory are within the
local trust boundary; this is not a hardware-protected key store. Protect any
identity backups as secrets. Never copy a key to enroll another device.

An agent holds a separate identity-use lease until shutdown. Reset and a second
agent using the same state fail while that lease is held; status remains usable.
Identity authentication alone does not authorize services. Schema-2 hosts enforce
current durable approvals/grants, including active-stream cancellation. The
[static tunnel walkthrough](service-tunnel.md) retains its historical scope;
see [current host control](per-host-control.md).

## Recovery and reset

Automated protected host recovery (#19) is deferred. If a host identity is lost
and no usable protected original exists, a new identity requires explicit
re-pairing by every client that trusted that host. Do not copy another host's
identity or assume the old registry can be reused with a replacement key.
Other hosts retain their own identities and permissions.

Missing, truncated, unreadable, wrong-permission, or inconsistent key state stops
startup. Restore the original identity from a protected backup when possible;
initialization will not overwrite corrupt state. Incomplete .identity-write-*
files are never loaded as keys. After a crash during a link/unlink commit, extra
hard links cause a safe failure. Stop the agent and inspect ownership, record
integrity and links before removing a verified orphan. No automatic cleanup
chooses an identity on your behalf.

For deliberate replacement of a client identity, stop the agent and revoke its
old public identity independently at every host that approved it. Host-authority
identity replacement additionally requires explicit migration/re-pairing; these
key-only commands do not migrate a host registry. Copy its public ID from status:

```sh
cargo run -p wipsnet-cli -- identity-reset --config "$HOME/.config/wipsnet/agent.toml" --expect-id PUBLIC_ID
cargo run -p wipsnet-cli -- identity-init --config "$HOME/.config/wipsnet/agent.toml"
```

Reset deletes the key only if its ID matches. It neither revokes remote access
nor creates a new identity. It does not securely erase filesystem backups or
recoverable blocks. A new key must be independently enrolled and granted access;
it cannot inherit the old key's authority. Reset refuses corrupt state; deliberate
manual recovery requires stopping the agent and protecting or removing the bad
record after recording/revoking the old public identity. Never delete the entire
state directory merely to repair one key: future policy/state may live there.

## Android app protection

The native app wraps its Iroh seed using an Android Keystore AES-GCM key and
stores the envelope in private app storage, with backup disabled. Missing or
corrupt envelopes fail closed; startup does not silently replace the identity.
Signing still uses software key material in memory. Hardware backing is
device-dependent and is not claimed by this implementation. See the
[Android app guide](android-app.md) for implementation and physical test scope.
The shell-UID native diagnostic harness below uses separate filesystem keys;
it does not establish app Keystore or Play-install acceptance.

## Local verification

The pre-push suite covers persistence, independent installations, file modes,
corruption, interrupted writes, hard links, symlinks, competing locks, guarded
reset, and running-agent reset refusal. CLI smoke tests check public status,
restart loading, missing/corrupt-state failure, reset and secret-safe diagnostics.
A real loopback Iroh integration test reloads two stored keys and checks each
authenticated remote ID; dialing a real peer with a different expected ID fails
or times out within the test bound. Discovery and relay services are disabled.
This does not establish internet/relay reachability or service authorization.

The native Android enrollment harness uses this same private filesystem format
under the shell UID in a temporary private directory. Its exclusive lease uses
OS `flock` because Rust standard file locking is unsupported on Android. The
[physical harness](enrollment.md) verifies on-device key creation and cross-process
reset protection. App-scoped Keystore wrapping is implemented separately;
see [Android app](android-app.md).
