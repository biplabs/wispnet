# v1 acceptance evidence — 2026-10-17

This matrix tracks the release criteria in #1 and acceptance work in #14. A
passing fixture does not establish the corresponding physical release scenario.
The exact published release build/device/network combination must be recorded
before a row with unverified release behavior can be accepted. No row claims a
full power-off, cloud VM/disk replacement, or protected recovery test.

| Release criterion / issues | Recorded implementation and evidence | Device/build scope | Still required |
| --- | --- | --- | --- |
| List/use website and Llama across networks (#11/#12/#13) | [Physical desktop browsing](linux-desktop.md#physical-guiheadless-checkpoint--2026-10-17), [Android app](android-app.md), [native public-egress evidence](external-relay-validation.md) | `d0595ad` / `58dc318`: CachyOS/KDE Chromium → headless Ubuntu WispShell; Ubuntu headless Firefox → GIKE/Llama, Llama health HTTP 200. Android guide distinguishes tablet app from native CLI. | Complete Play-phone normal-browser, unrelated-Wi-Fi/cellular/forced-relay matrix on the final build. |
| Symmetric physical power-off, surviving-host cold-start/local changes/fresh pairing (#16/#17) | [Node-offline/restart](linux-desktop.md), [independent tablet pairing](android-app.md#pairing-without-a-camera) | Two physical Linux computers with the opposite **node process stopped**; Android 14 tablet isolated sandbox. Relay continued on desktop.home. | Independent relay, actual opposite-machine power-off and cold starts in both directions. |
| Long unrelated-host outage preserves durable grants (#9/#14) | [Host control](per-host-control.md), local host/relay fixtures | Rust tests and stopped-node checks; no global permission lease. | Declared long-duration physical outage acceptance; explicit expiry/clock boundaries. |
| Composite service IDs, hostile catalogs and owner isolation (#8/#9) | [Host control](per-host-control.md), [desktop cache validation](linux-desktop.md) | Per-host Rust authority and Qt/cache regressions, same local service IDs on distinct hosts. | Service incarnations now reject stale listings/sessions at the identical backend, including physical headless tests; see [lifecycle evidence](service-lifecycle.md). Versioned complete snapshots, typed metadata, durable native high-water and the malformed/partial matrix are now implemented/tested locally and on headless Ubuntu; see [catalog evidence](catalog-snapshots.md). Physical power-off/rebuild acceptance remains. |
| Stale/offline/removal isolation and current host-local enforcement (#9/#10/#11) | [Desktop cache/Open behavior](linux-desktop.md), [Android permission checks](android-app.md) | `4638fd6`, `58dc318`: persistent cache; cached Open blocked; physical offline catalog failure about 5–5.6 s, surviving owner-local browsing works. | Complete snapshot/removal, rollback/generation and cache-deletion checks now have native fixture/headless evidence; see [catalog contract](catalog-snapshots.md). Final measured cancellation/rebuild acceptance remains. |
| No transitive admin; per-target multi-host results (#17) | [Combined owner operations](linux-desktop.md#combined-owner-operations) | `4b1ec0e`: both physical Linux hosts; desktop.home headless; installed Qt and headless CLI. Offline target pending, persistence/retry, rejected/wrong-target and lost-ack replay after later revoke. | Final release-build security/error consolidation. Full power-off/rebuild remains in the rows above/below. |
| Disposable relay/disk and changed-IP rebuild (#18) | [Relay fixtures](native-relay-validation.md), [operated relay](desktop-relay.md) | Local/native changed-address and relay-process restart; desktop-hosted public relay. | Independent relay deployment; destroy/recreate isolated VM and writable disk, same/new IP; verify pins/revocations and reconnect bounds. |
| Independent host recovery and stale backup (#19) | [Identity fail-closed behavior](identity.md), [SQLite foundations](registry-storage.md) | Local protected storage, consistent SQLite backup, restart/missing-state rejection. | Protected host-bound bundle, single-writer fencing, wrong-host/corrupt/key-loss rejection and stale-grant quarantine/reconciliation. |
| Browser/session/security isolation (#10) | [Browser decision](decisions/0001-browser-access.md), local Rust gateway tests and [physical browser paths](linux-desktop.md) | Loopback bootstrap/cookie/origin checks, host-local/remote routing, Stop/restart/removal fixtures; physical Chromium/Firefox and tablet Chrome. | Complete CSRF/WebSocket/origin/port-reuse matrix; explicit HTTPS/secure-cookie/service-worker/login compatibility decisions. |
| Streaming/resources/reconnect without write replay (#10/#13/#14) | [Native relay validation](native-relay-validation.md), Rust service-tunnel tests; combined-operation receipts documented above | Local streaming, backpressure, stream/connection bounds, malformed/timeouts and relay lifecycle. Combined retries replay receipts, not policy mutations. | Consolidate final measured limits on supported release/device paths; no arbitrary application-write retries. |
| Android background/lifecycle (#12/#14) | [Android lifecycle evidence](android-app.md), dated UI/emulator limitations | Physical Android 14 tablet; screen-off 60 s including 15 s forced deep Doze; post-wake browser reopen and Stop. Optical Play-phone scan recorded in #17. | Final Play-phone real-service/lifecycle checks, broader OEM restrictions, declared long-duration tests and applicable Android 15/16 acceptance. |
| Fresh install, recovery/rebuild guides and dependencies (#15) | [Linux distribution](linux-distribution.md), [Android internal publishing](android-release.md) | Release x86_64 bundle tested in isolated local/Ubuntu homes: fresh init, state-preserving reinstall, checksum rejection, missing-authority refusal; no Qt on headless Ubuntu. | Clean-OS GUI distribution verification, store declarations/release notes; evidence-backed protected backup and independent cloud rebuild guides (#18/#19). |

## Reproduction and evidence rules

`scripts/test.sh` and the enabled local pre-push hook run the native, Python,
Qt-offscreen, gateway and relay fixture suites. `scripts/test_linux_distribution.py`
checks an extracted bundle in a temporary home; set `WIPSNET_PACKAGE_TEST_BINARY`
to an explicitly built release executable to exercise that binary. Build bundles
with `scripts/package_linux.py`; inspect their manifest for source commit, dirty
checkout flag, payload checksums and glibc requirement.

Physical guides retain dated device/build details and limitations. The 2026-10-17
Ubuntu bundle check used a temporary home on desktop.home, staged files with
`--headless --no-start`, initialized only fixture authority and compared state
after reinstall. It did not restart the live node, relay or any desktop GUI.
Future evidence should record commit/version code, OS/device, network/path,
scenario, observed timing, cleanup and remaining uncertainty in its owning issue
and link it here. Deployment/account provisioning, machine shutdown, and live
authority recovery must not be inferred from a fixture's success.
