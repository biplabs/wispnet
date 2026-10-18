# Browser compatibility evidence (#2)

The current shared Rust gateway contract, HTTP-only compatibility decisions and
2026-10-17 Chromium/headless Firefox security matrix are in
[gateway security](gateway-security.md). The dated prototype evidence below is
retained with its original scope; it does not describe current unsupported-feature
handling or substitute for final Play-phone acceptance.

Observation date: 2026-10-15 (America/Vancouver). This matrix describes a local
Python HTTP prototype, not the shipping Rust application or an Iroh end-to-end test.

## Tested environment

Linux x86_64; Codex in-app Chromium user agent Chrome/155.0.0.0; Python 3.13.12;
aiohttp 3.14.5; Rust toolchain 1.95.0. Two disposable fixture websites, the live GIKE
Insight Explorer on loopback port 8310, and the existing llama.cpp UI/API on port
8201 were used. The llama.cpp checkout revision was 434ddbbc0; this is a source
checkout observation, not verification that the running binary matches it.

GIKE was located via user-systemd gike-insight-explorer.service, initially stopped,
then started for testing. Its directory and evidence detail loaded through the
gateway. No GIKE records, service definitions, model configuration, or permissions
were changed. Private application data, device IDs and capability URLs are omitted.

## Linux browser results

| Scenario | Result and evidence boundary |
| --- | --- |
| Service-specific .localhost resolution | Pass: both fixture services and actual GIKE/llama loaded |
| Ordinary navigation without Origin | Pass: initial page, root-relative CSS/JS and GIKE directory/detail |
| Fixture login and application cookies | Pass: two simultaneous service hostnames retain session=website and session=second |
| Same IP, different ports | Confirmed failure of isolation: second fixture overwrote first fixture's session cookie in real Chromium |
| Absolute redirect | Pass for exact configured-upstream redirect back to service root |
| Fixture SSE | Pass: first event arrived before completion, in browser and automated stream test |
| Fixture WebSocket | Pass: browser text echo; automated text/binary echo and session-expiry close |
| Unrelated-site attacks | Pass: fetch GET, POST, form POST and WebSocket probes caused zero fixture backend requests |
| Host/rebinding/duplicate Host | Automated pass: denied before backend request; no real hostile-DNS deployment was used |
| Origin null, sibling origin and missing Origin on writes/upgrades | Automated pass: denied before backend request |
| Bootstrap replay and expired session | Automated pass; expired WebSocket is closed |
| Browser disconnect | Automated pass: backend SSE connection observes cancellation/close |
| Live GIKE directory and evidence navigation | Pass: connected live-data state, directory cards, detail and source links rendered |
| Live llama.cpp UI and short chat | Pass: UI loaded and a short chat completed through the gateway |
| Live llama incremental token latency, long requests, backend login/OAuth | Not established; fixture streaming is not a live-model latency benchmark |
| Secure/domain cookies, service workers, malicious hosted app | Not established; production isolation/prefix/HTTPS review required |

## Android evidence

The connected device reports Android 14 / API 34 and installed Chrome
152.0.7977.82. USB debugging was explicitly authorized. Temporary adb reverse
mappings connect device-local gateway ports to host-local experimental gateways.
This validates a browser-facing candidate independently of the absent Android
companion app; it cannot establish native lifecycle or across-network behavior.

Actual browser interaction was performed with explicitly requested ADB launch and
touch input, screenshots, and fixture self-reports. Chrome's reduced user agent
reports Android 10/K and Chrome/152.0.0.0; OS/API and package version above were
read separately, not inferred from that reduced user agent.

| Physical Android scenario | Result and evidence boundary |
| --- | --- |
| External app/ADB to browser handoff | Initial 403 exposed cross-site Fetch Metadata on top-level navigation; fixed only for the public bootstrap landing page and regression-tested |
| Per-service .localhost resolution | Pass: both fixture hostnames and live GIKE/llama loaded |
| Two fixture sessions | Pass: reports show session=website and session=second independently |
| Fixture assets, login, early SSE and WebSocket echo | Pass on both fixture pages; status 200, first event before completion, hello echo |
| GIKE actual site | Pass: connected live-data directory rendered on physical device |
| llama.cpp actual UI/chat | Pass: UI rendered and short prompt received OK on physical device |
| Unrelated-site GET/POST/form/WebSocket probes | Pass: physical browser displayed backendRequestDelta=0 |
| Secure-context flag | True in fixture reports; not proof of HTTP encryption or Secure-cookie acceptance |
| Native Android app, background lifetime, screen-off, Wi-Fi/cellular transitions | Not tested; #12/#13/#14 |
| Iroh/direct/relay end-to-end transport | Not tested; USB reverse is an independent compatibility fixture |

No Android system/browser security setting was weakened, no APK was installed,
and backend application configuration was unchanged.

## Remaining validation

The decision in [ADR 0001](decisions/0001-browser-access.md) selects per-service
hostname isolation and an HTTP-aware authorization boundary as the direction.
HTTPS/cookie-prefix behavior and Android support are still conditional.

GIKE is a read-only explorer with no login in the tested configuration. The tested
llama UI has no backend login configured. Fixture login/cookie tests supply those
compatibility cases; OAuth and authentication configurations not exercised here
remain explicit #10 compatibility gates. Before release, run the real Iroh/direct/relay and native-agent lifecycle tests under
#5/#10/#12/#13/#14. USB loopback forwarding is not the product's network access path.

Reproduction commands and fixture controls are in the
[spike guide](../experiments/browser/README.md). The nine automated boundary tests are
part of the local pre-push hook and do not depend on live GIKE, llama or a device.
