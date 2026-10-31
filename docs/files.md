# Built-in Files service

Every Wispnet host includes the permanent `files` service alongside `shell`.
Files uses binary frames on the authenticated native Iroh/QUIC service tunnel;
file bytes never pass through MCP JSON, Shell output, or the HTTP gateway.

The host owner chooses receiving folders and the complete list of paired
clients that may send files into them. Pairing and Shell grants confer no Files
access. Exports begin empty, and remote access begins disabled.

Choose at least one folder before granting Files access. A grant is ongoing
permission: that device can send files into every configured folder without
asking you to confirm each transfer. This is a one-way receiving service: the
grant does not let the sender read or download files from your computer.
Transfers cannot overwrite existing files or escape the receiving folders.
Revoke the device's Files grant to stop active access and prevent new transfers.

## Configure a host

Create a directory for the files, then configure it on the host:

```sh
mkdir -p /srv/training
wispnet files-access --export training=/srv/training --allow FULL_CLIENT_PUBLIC_ID
```

Use `--config /absolute/path/host.toml` when managing a nondefault node.
Desktop settings provide **Files access** with a native folder picker and device
grants. Choose the receiving folder there; the path is shown only on that device.
Use the gear beside the selected folder to change it. Removing the folder
disables incoming transfers and revokes Files grants when saved.
MCP agents can use the owner-only `wispnet_set_files_access` tool to set the
receiving folder and its complete device allow list in one revision-checked
operation; see [MCP configuration](service-registry.md).
Export roots must be existing absolute directories without symlink components;
they cannot overlap Wispnet's private state directory. The grant applies to all
configured exports. Remote paths are relative to the selected export.

Repeated `--export` options replace the entire export map. Omitting exports
preserves the map; omitting `--allow` preserves the grants. To revoke all remote
Files access, run `wispnet files-access --allow none`. To remove all exports and
revoke their grants, use `wispnet files-access --clear-exports`. Sharing is
disabled until a folder is configured and devices are granted access again.

Export changes rotate the service incarnation and invalidate previous resume
bindings. Grant revocation closes active remote streams through the same
current-policy checks used by Shell. Files cannot be renamed or removed as an
ordinary HTTP/TCP service; an old custom service named `files` is migrated to an
available `files-legacy` identifier with its existing grants preserved.

## Transfer a file

The node must be running. Device names must resolve unambiguously; a full public
host ID always works. The sender chooses a local file and its destination path
inside one of the receiving host's configured folders:

```sh
wispnet files put GPU_HOST training ./dataset.npz jobs/run-1/dataset.npz
```

Transfers resume by default. Repeat the same command after a disconnect, client
restart, or node restart. Ctrl+C pauses a transfer and retains its resumable
bytes. `--no-resume` starts with fresh client resume state. Existing destinations
are never overwritten, including files with identical content. Retrying your own
completed transfer can recover its receipt while its client journal and the
receiving host's retained transfer state remain available (24 hours by default).

Progress is JSON on stderr, and completion or a typed transfer error is JSON on
stdout. The owner-local API starts bounded background jobs with paths and target
metadata only (`file_put`), then exposes `file_status` and
`file_cancel`. The CLI also supports `files status TRANSFER` and
`files cancel TRANSFER`. Background job handles are ephemeral; repeat put to
recover durable resume state after a node restart.

On desktop, drop one or more files into the Wispnet pop-up or onto a remote
device's **Files** service, then select **Send files**. The receiving device
chooses the destination; the sender neither selects nor sees its receiving
folder. The app does not discover receiving folder names.
The transfer panel shows progress, cancellation and resume actions. Hiding the
pop-up or closing the panel keeps submitted transfers running. Files are queued
one at a time and retain their original names; folders must first be archived.
The recipient must already have granted Files access.

On Android, tap a device's **Files** service and use **Choose files** to select
one or more documents in Android's system picker.
Select **Send files** to queue the batch. Two files can run concurrently; each
has its own progress, cancel and resume controls. The network service retains
the queue when the panel closes or the UI is recreated. Going offline cancels
its active and queued transfers. After a network-process restart, select the
original documents again to recover their durable resume state.

Regular Android documents stream directly through their read-only descriptors.
Providers that expose a stream or a file slice are prepared in private temporary
storage with bounded buffers and cancellation, then sent through the same native
protocol. Temporary copies are removed when the transfer finishes or is cancelled.
File contents never cross Flutter or the UI's Binder connection.

App uploads use an empty workspace selector, which the receiving host resolves
to its configured receiving folder without returning its name or filesystem
path. Existing configurations with multiple exports use the first export in
name order for these uploads. Explicit named exports remain available to the
CLI and SDK; authenticated legacy folder discovery returns export aliases only.

The Rust SDK exposes `Client::put_file`, `FileTarget`,
`TransferOptions`, progress callbacks, and `FileCancellation` in
`wispnet_client::files`.

There is no `files get` command or `file_get` API. To send files in the other
direction, the other computer's owner must configure a receiving folder and
grant the first computer permission to send there.

## Reliability and limits

Uploads use a persistent manifest bound to the authenticated client, service
incarnation, workspace, destination, size, SHA-256, and idempotency key. A chunk
acknowledgement reports a contiguous durable prefix. Duplicate acknowledged
chunks are verified against stored bytes. Upload commit verifies the complete
file and publishes it atomically without replacing an existing destination.
Retries recover lost acknowledgements and commit responses.

Pipelined upload chunks can share a durable flush in groups of up to eight.
Each chunk still receives a response; its acknowledged offset may include later
chunks in the same group. Single-request callers remain supported.

Only regular files are supported. Absolute remote paths, traversal, symlinks,
and reserved partial-file names are rejected. Disk/hash work runs outside the
async I/O executor. Limits bound chunk size, pending bytes, upload count, bulk
operations, client background jobs, and retained upload state. Expiry/abort only
remove owned partial data; completed artifacts remain.

Defaults allow files up to 1 TiB, 2 TiB of declared pending uploads, and 1,024
retained upload manifests, with a 24-hour retention period. Four disk-heavy
operations can run concurrently; the client allows two background transfers.
Idle maintenance checks expiry every minute. Each frame read/write has a
300-second deadline; the complete transfer has no fixed session lifetime.

## Throughput target

The initial target is **1 Gbit/s sustained file payload** on a direct path with
sufficient source upload, destination download, CPU, and disk capacity. Report
steady-state payload throughput and total time through verification/publication
separately. Relay and WAN results require their own measurements.

Frames carry up to 4 MiB of binary data. Helpers pipeline eight chunks, allowing
32 MiB of application data in flight while preserving ordered writes. Transport
windows are 64 MiB per stream, 128 MiB receive per connection, and 64 MiB send per
connection. These are bounded ceilings rather than eager allocations. At
100 ms RTT, 1 Gbit/s needs approximately 12.5 MB in flight; at 200 ms, 25 MB.

Linux hosts use a high-resolution monotonic timer for shared Iroh/QUIC protocol
deadlines, reducing late pacing wakeups during sustained transfers. This applies
to Shell, control and other native services as well as Files. Timer failures fall
back to the existing Tokio timer, which other platforms continue to use. Files
still runs inside the existing Wispnet daemon.

Run the optional 1 GiB native upload/restart/resume benchmark:

```sh
cargo test -p wispnet-agent --release --test files native_files_bulk -- --ignored --nocapture
cargo test -p wispnet-client --release --test files sdk_large_file -- --ignored --nocapture
```

This transfers real binary payload over authenticated Iroh/QUIC, checks a durable
offset after restarting the host, and verifies the final SHA-256. Loopback results
demonstrate local implementation capacity, not a guaranteed Internet rate.

The SDK benchmark uses two native peers, pauses an upload, restarts both nodes,
and resumes the upload through verification and commit.
Set `TMPDIR` to a directory on the desired filesystem to include that storage
device in the measurement; `/tmp` may be backed by memory.

An initial 1 GiB run on the development machine's normal filesystem measured
3.77 Gbit/s of resumed native upload payload, with verification/publication
complete in 3.00 seconds. The same run checked status responsiveness during the
transfer. These are local loopback measurements; Internet and relay paths still
need separate validation.

The full SDK run on that filesystem measured 3.27 Gbit/s for upload including
source hashing and commit.
