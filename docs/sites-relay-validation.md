# Codex Sites relay compatibility experiment (#13 / #18)

On 2026-10-16 a private Worker ESM Site was deployed at
https://wispnet-relay-compatibility.chirpy-tiger-6433.chatgpt.site.
Its source is `sites/relay-compatibility`; its opaque project identity is tracked
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
cargo build -p wipsnet-cli --example sites_relay_probe --locked
# With the NDK linker/CC/AR variables configured:
cargo build -p wipsnet-cli --example sites_relay_probe --target aarch64-linux-android --locked
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
