# Browser gateway security and compatibility (#10)

## v1 decision

The shared Rust Android/desktop gateway serves **HTTP only on IPv4 loopback**,
using a fresh random `w<128-bit>.localhost:<port>` origin for every Open. There is
no LAN listener, installed certificate authority, shared service domain, or TLS
termination in the browser hop. Iroh separately encrypts the peer transport.
The trust boundary excludes hostile same-UID/root processes and application
scripts accessing their own origin. A browser secure-context flag does not
establish transport encryption.

The gateway cookie is host-only, HttpOnly, SameSite=Strict, Path=/ and expires
at 600 seconds. It deliberately has no Secure flag or cookie prefix. A one-use
256-bit bootstrap token expires after 60 seconds; the landing page removes its
fragment before exchanging it with a same-origin POST. Desktop capability URLs
travel through private handoff files/stdin, never browser command arguments.
The route's monotonic ten-minute deadline closes its listener and active
connections even if browser cookie expiry differs. Reauthorization requires
an explicit new Open and a new origin/token/cookie; there is no silent renewal.

Applications requiring HTTPS-specific cookie semantics are unsupported in v1.
A response setting Secure, Partitioned, SameSite=None or `__Host-`, `__Secure-`,
`__Http-` (including `__Host-Http-`) cookies is rejected with 502 **before any
upstream headers are emitted**. Those flags/prefixes are never stripped to make
a login appear to work. Ordinary application cookies retain their Path,
HttpOnly and SameSite settings, lose Domain to become host-only, and cannot set
or receive `_wipsnet_session`. This is a bounded compatibility decision, not a
claim that arbitrary web apps work over HTTP.

Service workers are unsupported: script requests with `Service-Worker` or
`Sec-Fetch-Dest: serviceworker` get 403 before connecting to the backend.
`Service-Worker-Allowed` is removed from upstream responses. Random origins are
never intentionally reused. Storage/cookies from an old origin cannot authorize
a later route; expiry/Stop do not erase already rendered pages, downloaded data,
or an application's own localStorage. Own-origin scripts remain application
code trusted with that application's access.

Relative and same-gateway-origin redirects work. Owner-local absolute redirects
to the **exact current backend origin** are rewritten to the gateway origin.
Remote private backend addresses are intentionally not disclosed in catalogs;
such absolute redirects cannot be safely rewritten and are rejected. Other
origins/schemes, userinfo, backslashes, control characters and Refresh redirects
are rejected with 502. External OAuth/SSO and application-embedded absolute URLs
need an explicit future compatibility design. Applications should use relative
URLs. Ordinary relative fixture login passes in Chromium and Firefox.

## HTTP boundary

- Exact Host, single Origin and Fetch Metadata are checked before backend access.
  Cross-site/sibling/null-origin requests are rejected, including GET and
  WebSocket. Non-GET/HEAD requests and WebSocket require exact same-origin Origin.
  Only the public bootstrap landing page permits an external top-level document
  navigation; it grants no backend access. Bootstrap replay/expiry fail closed.
- Browser-facing requests must be HTTP/1.1 origin-form. GET/HEAD/POST/PUT/PATCH/
  DELETE/OPTIONS are supported; TRACE/CONNECT and other methods get 405.
  Uploads require a single decimal Content-Length (zero if absent), at most
  16 MiB, within 30 seconds. Chunked uploads, Expect/TE/Trailer and ambiguous
  framing get 400; oversize declared uploads get 413.
- Headers are bounded to 32 KiB/96 fields and a **total** ten-second deadline,
  not a per-byte idle timeout. The gateway allows 16 routes and 32 concurrent
  accepted connections across routes; excess connections close immediately.
  Stop/expiry releases capacity. A slow reader applies bounded-copy backpressure;
  there is no whole-response buffer and no total download-size cap.
- HTTP/1.0 and 1.1 backend responses support exact Content-Length,
  close-delimited bodies and plain chunked transfer. Chunk lines are at most
  128 bytes; each chunk is at most 16 MiB; extensions/trailers are unsupported.
  Duplicate/conflicting Content-Length/Transfer-Encoding and unsupported 1xx
  responses get 502. Fixed-length/chunked termination prevents copying a second
  response. Malformed bodies after headers abort the connection; they cannot
  be converted retrospectively into a 502.
- WebSocket requires GET, version 13, a base64 16-byte key, Connection/Upgrade,
  exact Origin and a valid 101/Sec-WebSocket-Accept response. A returned
  subprotocol must have been offered. Non-WebSocket protocol upgrades are
  unsupported. Validated connections stream bidirectionally with backpressure.
- Request forwarding credentials/proxy headers and response hop-by-hop headers
  (including Connection-nominated fields) are removed. Upstream CORS permission
  headers cannot relax the gateway boundary. Responses force no-store,
  no-referrer, frame-ancestors 'none', X-Frame-Options DENY, same-origin CORP/COOP.
  Application CSP headers are retained alongside the frame restriction.
- Peer identity/service incarnation and current host-local grants select the
  backend; catalog display text never selects a URL/backend. Revocation/removal
  cancels active native streams. Owner-local binding changes are polled every
  second with a one-second storage-read deadline. No HTTP write is retried or
  replayed automatically after interruption/reconnect.

## Recorded 2026-10-17 validation

`scripts/test_physical_gateway_security.py` exercises the installed shared Rust
node on CachyOS/KDE and Ubuntu 24.04 `desktop.home` in **both directions**, with
two temporary same-ID services and malicious literal display text. Chromium
154.0.8037.97 locally and existing Firefox 158.0 in headless mode on Ubuntu each
open simultaneous owner-local and remote origins. Checks cover application and
HttpOnly credential cookies, localStorage, relative login, chunked SSE,
WebSocket echo, denied cross-origin fetch/WebSocket/iframe GET/form POST with **zero backend requests**, rejected Secure/prefix
cookies/external redirects and blocked service-worker registration. Raw checks
cover CSRF, null/cross-site Origin, Host mismatch, methods, framing, upload
bounds, bootstrap replay and actual non-loopback interface connection refusal.
Host-local grant withdrawal denies an existing route, remains denied after
client-node restart, and permits a fresh explicit Open after regrant.
An interrupted application POST commits exactly once; a fresh Open does not
replay it. Fixtures restore services/grants/pins/access settings; the relay PID
is unchanged and the remote Qt/tray unit remains absent.

The seven `wipsnet-client` gateway unit tests also run as a native executable on
headless Ubuntu: strict request/response/cookie/redirect/WebSocket cases;
single-use bootstrap and Stop; short **private test-only** expiry limits and
forced exact port reuse by a different peer; 16-route/32-socket limits and a
continuously trickled header stopped by the ten-second deadline. Expired tokens
and old cookies/Host cannot substitute the newly bound host; fresh bootstrap
gets a different credential. No production TTL control is exposed. This is
same-code expiry validation, not a claim of waiting out a physical ten-minute
browser session. Existing [service recreation](service-lifecycle.md) and
[offline/restart](linux-desktop.md) evidence remains independently scoped.

Run the opt-in physical test only on those approved paired hosts:

```sh
python3 scripts/test_physical_gateway_security.py
python3 scripts/test_physical_service_recreation.py
cargo test -p wipsnet-client --lib --locked
```

The enabled `scripts/test.sh` pre-push suite also checks native tunnel SSE early
arrival, slow-reader backpressure/cancellation, bounded QUIC/connection/stream
credit, malformed frames/timeouts, host policy isolation, and no POST replay
(`crates/agent/tests/service_tunnel.rs`, `scripts/tunnel_smoke.py`). These are
consolidated with remaining final-device work in [release acceptance](release-acceptance.md)
and #14. Actual independent relay VM/disk replacement remains #18; this
checkpoint neither destroys the live relay nor claims that deployment test.
The native `explicit_tls_relay_without_udp_pins_identity_enforces_grants_and_recovers`
fixture runs locally and as an executable on headless Ubuntu: rebuilds its own
stateless TLS relay at the same address, recovers an approved client within the
30-second test bound without replaying a service request, restarts a previously
revoked client with the same key, and denies both its catalog and bound service
request. This checks the authority boundary across relay replacement; it does
not establish cloud provisioning or changed-IP VM/disk replacement acceptance.
Fresh Play-phone browser acceptance is separate from Android app/instrumentation
APK build success.

Normative references: [HTTP/1.1 message framing, RFC 9112](https://www.rfc-editor.org/rfc/rfc9112.html),
[Set-Cookie semantics](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie),
and [service-worker script requests](https://w3c.github.io/ServiceWorker/#service-worker-script-request).
