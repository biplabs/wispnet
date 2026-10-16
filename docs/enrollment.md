# Invitation enrollment (v1)

Enrollment adds an installation without copying another installation's private
key. Linux and the native Android harness use the same portable JSON payload,
Ed25519 proof protocol, HTTPS client, and persisted identity. The Android
companion UI and Android Keystore wrapping remain #12; the shell harness below
is a protocol/device check, not a shipped Android application.

## Owner and device walkthrough

First initialize the controller using `wipsnet registry-init` and configure its
HTTPS proxy as described in [registry API](registry-api.md). Keep the owner token
on the controller/owner workstation. A normal device never receives it.

1. Create an invitation request file containing:

   ```json
   {"op":"invite","ttl_seconds":600}
   ```

   Then create a private invitation file:

   ```sh
   python3 scripts/registry_admin.py --origin https://controller.example.com \
     --owner-token /private/controller/owner.token \
     --mutation /private/invite-request.json --invitation-out /private/invitation.json
   ```

   The output file is new and mode 0600. The tool does not print its code. Send
   the file/code to the intended installation through a trusted channel. A code
   is a short-lived secret; it authorizes a pairing request, not access to private
   services. A QR transport can encode exactly this JSON; no QR generator or
   camera dependency is required by this implementation.

2. On the device, copy `config/controller.toml`, set its local state path, a
   human-readable name, and the exact controller HTTPS origin. The invitation
   must match that configured origin. Save the received JSON in a 0600 file:

   ```sh
   target/debug/wipsnet enroll --config /absolute/path/device.toml \
     --invitation /private/invitation.json
   ```

   This explicit command initializes the device's own identity if missing and
   preserves an existing identity. It signs a fresh server challenge and prints
   `Pending owner approval`, the device name, and its full public endpoint ID.
   It does not print any private key, invitation code, or device credential.

3. The owner lists pending requests:

   ```sh
   python3 scripts/registry_admin.py --origin https://controller.example.com \
     --owner-token /private/controller/owner.token --enrollments
   ```

   Compare the **full endpoint ID and exact name** with the intended device,
   through a trusted channel, before approving. Names alone are not identities.
   Write an approval request using the current listing revision:

   ```json
   {"op":"approve","invitation":"INVITATION_ID","device":"PUBLIC_ENDPOINT_ID","name":"Linux laptop","expected_revision":0}
   ```

   ```sh
   python3 scripts/registry_admin.py --origin https://controller.example.com \
     --owner-token /private/controller/owner.token --mutation /private/approve.json
   ```

   The controller accepts approval only after successful proof and only for the
   exact bound key/name. To refuse a request instead, submit
   `{"op":"reject","invitation":"INVITATION_ID"}` to the same owner tool.

4. Rerun the same `wipsnet enroll` command on the device. After approval it signs
   a **new completion challenge** and receives its own credential, committed to
   `controller.credential_file` atomically as mode 0600. The containing directory
   must be current-user-owned and mode 0700. No owner distributes device tokens.

   ```sh
   target/debug/wipsnet registry-directory --config /absolute/path/device.toml
   target/debug/wipsnet agent --config /absolute/path/device.toml
   ```

   Restart uses the stored identity/credential and does not repeat pairing. If
   `enroll` is rerun with a valid existing credential it verifies the device's
   directory and reports `Already enrolled`, without submitting the invitation.
   A revoked/corrupt credential fails rather than being silently overwritten.

## Payload and proof contract

The copyable invitation is UTF-8 JSON with strict unknown-field rejection:

```json
{"version":1,"controller":"https://controller.example.com/","mesh":"personal","invitation":"64_LOWERCASE_HEX_DIGITS","code":"64_LOWERCASE_HEX_DIGITS","expires_at":1792180000}
```

`invitation` is a public random identifier; `code` is an independent 256-bit
capability whose SHA-256 hash is stored on the controller. `expires_at` is UTC
Unix seconds. Owner-selected TTL must be 30–900 seconds; the walkthrough uses
600. A device name must be nonblank, at most 128 UTF-8 bytes, and contain no
control characters. Core types/parser/canonical encoding are in
`wipsnet_core::enrollment`, shared by native Linux/Android builds. Payloads never
contain an existing installation's private key, owner credential or TLS root.

Private HTTPS controllers may use an independently provisioned PEM trust root:

```toml
[controller]
origin = "https://controller.example.com"
credential_file = "/private/device/device.token"
ca_certificate_file = "/private/controller-root.pem"
```

This explicit local configuration adds a trusted CA; hostname/certificate
validation and HTTPS requirements remain enabled. Root files must be regular,
singly linked, not writable by group/others, owned by the current user or root,
and at most 16 KiB. A root supplied inside an invitation is rejected as an
unknown field. Do not trust a root obtained from an unauthenticated endpoint.
Public HTTPS deployments use the built-in trust roots and omit this field.

| Endpoint | Credential | Request / result |
|---|---|---|
| `GET /v1/enrollment` | Owner | Revision, invitation states, named enrolled/revoked devices; no codes, hashes or private keys |
| `POST /v1/enrollment/admin` | Owner | `invite`, `approve`, `reject` |
| `POST /v1/enrollment/challenge` | Invitation code bearer | `{invitation,device,name}` → bound challenge or pending state |
| `POST /v1/enrollment/proof` | Invitation code bearer | `{invitation,device,name,nonce,signature}` → pending state or own device token after approval |

There is no direct `enroll` mutation in `/v1/mutations`. This removes the earlier
manual provisioning bypass. Existing schema-2 devices/tokens are preserved and
listed as `legacy`; all new enrollment requires the proof/approval protocol.

Each challenge expires after at most 120 seconds and never outlives its
invitation. Its Ed25519 signed bytes are, in exact order:

- ASCII `wipsnet/enrollment/proof/1` followed by a NUL byte;
- version as an unsigned 16-bit big-endian integer;
- controller origin, mesh, invitation ID, name, nonce, each encoded as a 32-bit
  big-endian byte length followed by UTF-8 bytes;
- the raw 32-byte public endpoint key;
- challenge expiry as an unsigned 64-bit big-endian integer;
- ASCII `request` or `complete`, according to challenge purpose.

Signatures use Iroh's strict Ed25519 verification and serialize as 128 lowercase
hex digits. The client checks the challenge against its local invitation, key,
name and expiry before signing. Server state determines the permitted purpose.
A fresh challenge invalidates the previous nonce; a successful proof consumes
it. Request proofs cannot be reused to complete an approved enrollment.

## States, limits and recovery

An invitation progresses `issued` → `pending` (proof verified) → `approved` →
`enrolled` (fresh completion proof). The first challenge binds its key/name;
subsequent attempts cannot attach a different key, including after approval.
The owner may reject before enrollment. Active invitations become `expired` at
their deadline. Revoking an enrolled device also marks its invitation `revoked`.
The owner listing shows these states; only the intended installation can receive
its completion credential.

Unauthenticated enrollment is globally limited to 60 challenge/proof requests
per minute per registry process. Each valid invitation additionally has a
persistent budget of 12 attempts per 60 seconds, including failed proofs and
pending checks. Restart cannot reset this per-invitation budget. Owner operations
remain separate so rate-limited public traffic does not consume that budget.
The API's existing concurrency/body/time limits also apply. Rate limits return
429 `rate_limited`; invalid, expired, reused, mismatched and unapproved proofs
fail closed. The global limiter can cause temporary denial of pairing under
attack; it is not a per-IP Internet abuse service. The controller's clock is
trusted for expiry and should be kept synchronized.

At most 128 invitations may be active. The controller retains up to 4,096 recent
invitation records, pruning oldest terminal records as new invitations are
created. Named device/revocation history remains separate. A device count of
1,024 is enforced. Completion atomically creates the device/credential and bumps
the policy revision once. Approval alone does not grant directory access or
create a policy revision. Concurrent completion cannot mint two credentials.

Revocation uses the existing owner mutation:

```json
{"op":"revoke","expected_revision":1,"device":"PUBLIC_ENDPOINT_ID"}
```

It records the revoked state and a new revision, invalidates the credential and
removes the device's grants/hosted services atomically. Enforcement in existing
service tunnels and cached policies remains #9. For a replacement key, create a
new invitation and explicitly approve the new endpoint ID. Never copy an old
installation's identity key or implicitly rotate it on a read failure.

Completion credentials are returned once. If the response is lost or local
credential commit fails, the owner must revoke the installation, issue a new
invitation, and approve again. Remove the unusable credential file only after
revocation; the CLI never overwrites it. Corrupt/missing identity storage follows
[identity recovery](identity.md), not automatic key rotation. Backups include
invitation states/nonces/rate budgets and device metadata; the existing restore
warning about resurrecting pre-revocation authority still applies.

## Verification and physical Android harness

`./scripts/test.sh` runs protocol/state-machine tests plus the native HTTPS CLI
smoke. That smoke uses a short-lived local certificate with an explicitly
configured test trust root, a TLS proxy and a real Rust registry process. It
checks owner-tool code secrecy, Linux request/approval/completion, identity and
credential persistence across agent restarts, backup/restore and revocation.
OpenSSL must be installed to generate this disposable certificate.

For the connected authorized aarch64 Android device, build with your Android NDK
linker/CC/AR configuration and run:

```sh
export WIPSNET_NDK="$HOME/Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_aarch64_linux_android="$WIPSNET_NDK/aarch64-linux-android34-clang"
export AR_aarch64_linux_android="$WIPSNET_NDK/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
cargo build -p wipsnet-cli --target aarch64-linux-android --locked
python3 scripts/registry_smoke.py --android
```

The optional harness currently expects NDK `27.2.12479018` beneath
`~/Android/Sdk/ndk` for stripping the test binary. It creates a fresh private
`/data/local/tmp/wipsnet-enrollment-*` directory, copies only the binary,
invitation, config and public test certificate, and installs one temporary USB
reverse mapping. Android generates its own identity key on-device, enrolls over
HTTPS, then rereads its persistent credential without pairing again. The harness
also starts the Android agent and verifies that its exclusive identity lease
blocks a simultaneous identity reset, then shuts the agent down cleanly. All
harness files and its reverse mapping are removed afterward. It never pulls a
private identity key off the device. On Android, the native identity store uses
nonblocking OS `flock` because Rust's standard file-lock API is unsupported.

Physical verification on 2026-10-16 passed on the connected Android 14/API 34
aarch64 device. This verifies the native payload/enrollment path over USB, not
camera pairing, the companion UI/Keystore, or cellular/relay networking.
