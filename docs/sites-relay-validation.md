# Codex Sites relay compatibility experiment (#13 / #18)

On 2026-10-16 a private Worker ESM Site was deployed at
https://wispnet-relay-compatibility.chirpy-tiger-6433.chatgpt.site.
Its source is `experiments/relay-compatibility`; its opaque project identity is tracked
in that directory's `.openai/hosting.json`. The platform service token is not
tracked or written to a file. The production Wispnet relay configuration is
unchanged.

Final Sites source commit: `e2962ec250aa2f1a6e601c8c8da88fbb875a7936`.
Saved version: `appgprj_6ad28d7003288191839174fde279e54a~appgver_ed31ab2e59508191ad28f2ec5878e7da`.
Private deployment: `appgdep_6ad28f2e0be48191a218b23c59ad4ee5` (succeeded).

## Observed result

Two physical x86-64 Linux computers (this computer and `desktop.home`) and the
arm64 Android tablet connected directly to the hosted HTTPS/WSS endpoint. SSH
and ADB only orchestrated processes/copied probe binaries: no SSH forwarding,
USB reverse, local relay, direct UDP or peer discovery participated.

- Authenticated HTTPS `/ping` succeeded.
- All three devices negotiated `iroh-relay-v2` and completed the Ed25519 signed
  challenge, using the domain-separated BLAKE3 derivation from pinned Iroh 1.3.0.
- Datagram round trips failed for local computer/desktop, desktop/tablet and
  tablet/local computer. Each sender received `EndpointGone` (destination
  unavailable) while its listener was authenticated and waiting.
- The Worker keeps its routing table in an isolate-local `Map`. These observations
  demonstrate that this deployment does not provide a shared live connection
  router. A JavaScript global is insufficient; successful WebSocket authentication
  alone is not a successful relay.
- A prototype binary-message bug was repaired before the final measurements:
  binary messages can arrive as Blobs, so the handler normalizes them to buffers.

The probe tests the relay wire protocol with native Iroh keys/signatures. It does
**not** test stock Iroh QUIC transport, pairing, catalog, service HTTP, an Android
APK, cellular, or whole-computer power-off acceptance through Sites. Those cannot
be claimed from a failed packet-routing experiment. #13 remains open.

## Additional compatibility requirement

Owner-private Sites requires `OAI-Sites-Authorization: Bearer …`. The existing
Iroh 1.3.0 native `RelayConfig` / `ClientBuilder` token option sends
`Authorization: Bearer …`, and exposes no arbitrary relay-request header setter.
The isolated `sites_relay_probe` uses the underlying WebSocket client to send the
Sites header. No product transport modification or secret-bearing configuration
was introduced. A private Sites relay would need an upstream/client integration
for this access gate in addition to a supported live socket coordinator.

The installed Sites hosting contract documents D1/R2 bindings, not a Durable
Object binding. This experiment does not establish whether Sites can provision
such a coordinator. A supported coordinator or a conventional relay service is
still required. D1/R2 must not become an application authority or persisted
packet mailbox to conceal the routing failure.

## Reproduce

Build the Linux example and cross-compile the same example for the configured
Android NDK/target (see the Android build procedure in the native validation
notes):

```sh
cargo build -p wispnet-cli --example sites_relay_probe --locked
# With the NDK linker/CC/AR variables configured:
cargo build -p wispnet-cli --example sites_relay_probe --target aarch64-linux-android --locked
python3 scripts/sites_relay_smoke.py --ssh-host desktop.home --serial AUTHORIZED_SERIAL \
  --url https://wispnet-relay-compatibility.chirpy-tiger-6433.chatgpt.site
```

Read the existing service token through Sites `get_site`, then provide it at the
hidden terminal prompt. Do not generate/rotate a token just to inspect access.
The runner expects three successful round trips and exits nonzero on the observed
compatibility failure. It does not convert that failure into a passing acceptance
check. This live test is opt-in, not part of the pre-push hook. The hook does run
the local bundled-Worker proof checks plus the existing full Wispnet suite.

Probe identities are ephemeral and generated independently on each device. The
runner copies only binaries into unique private temporary directories and removes
them afterwards. Existing application state/services/keys are untouched. Tablet
Wi-Fi must be connected for the direct hosted test.

## Durable Object follow-up

The current Sites interface was checked on 2026-10-16 rather than assuming
Cloudflare Workers features are automatically available in Sites:

1. [Official Sites documentation](https://learn.chatgpt.com/docs/sites?surface=app#understand-limits-and-unsupported-uses)
   confirms HTTP/HTTPS/WebSockets and D1/R2, but does not document Durable Objects.
   The installed Sites skill/tools likewise provide no namespace provisioning or
   binding operation. This alone is not proof of runtime absence.
2. A standard Cloudflare `durable_objects.bindings` declaration naming
   `RELAY_ROOMS` / `RelayRoom`, with the required new-class registration, was
   submitted in the deployment manifest. The exact extra fields are retained in
   `experiments/relay-compatibility/durable-objects.probe.json` as a **rejected probe**,
   not active Sites configuration. Sites rejected the archive before deployment:
   `Invalid dist/.openai/hosting.json: Extra inputs are not permitted`
   (`INVALID_ARGUMENT`, `invalid_archive`, HTTP 400). No deployment or saved
   version ID was returned for this attempt. Its Sites source commit was
   `327afed455e5e9b59b8d3d62aac203e105ec5e41`.
3. Those unsupported fields were removed, and the same Worker with an
   authenticated `/capabilities` endpoint deployed successfully. It returned
   HTTP 200 with `durable_object_binding: false` and `durable_storage: false`.
   The endpoint exposes only fixed labels/booleans, never environment names,
   values, keys, peer identifiers or access credentials.
4. The new `/relay-durable` route requires a genuine namespace binding and fails
   with HTTP 503 / `durable_object_binding_unavailable` if it is absent. It never
   falls back to the broken isolate-local router. Both physical computers and
   the tablet independently received HTTP 503 directly from that hosted route.

The accepted capability deployment is
`appgdep_6ad29101dca881918fbfa48904da2938`, saved version
`appgprj_6ad28d7003288191839174fde279e54a~appgver_472214d536a8819192ce5a3724415ad6`,
Sites source commit `192820e48ca213df84e3a28504f3f97964db32df`.

The code now includes an exported `RelayRoom` class with per-object, ephemeral
connection state and a fixed object name for every relay request. Local tests
verify canonical object selection across requests, header preservation, rejection
of missing/malformed bindings, and redacted capability output. These are routing
contract tests with a fake namespace; they do not establish real Durable Object
provisioning or successful coordinated WebSockets in Sites.

[Cloudflare's own deployment contract](https://developers.cloudflare.com/durable-objects/get-started/#4-configure-durable-object-bindings)
requires an upload-time namespace binding in addition to the exported class.
Adding that class or a JSON/string environment variable cannot manufacture a
binding. A direct Cloudflare deployment would be a separate hosting integration;
none was attempted with unprovided account credentials.

**Conclusion:** the currently exposed Sites deployment schema rejects the
standard Durable Object declaration, and the accepted deployment has no
`RELAY_ROOMS` namespace. This route cannot fix #13 through the available Sites
interface. This does not assert that Cloudflare lacks Durable Objects or that a
future Sites interface cannot support them. #13 remains open; no whole-machine
off, application pairing/service, or external relay replacement acceptance is
claimed from these failures.

Repeat the required-coordinator check with the existing command plus `--durable`:

```sh
python3 scripts/sites_relay_smoke.py --durable --ssh-host desktop.home \
  --serial AUTHORIZED_SERIAL \
  --url https://wispnet-relay-compatibility.chirpy-tiger-6433.chatgpt.site
```

A missing coordinator deliberately makes this live acceptance check exit nonzero.
The baseline `/relay` route is retained only as the earlier comparison experiment.
