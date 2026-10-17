# ADR 0001: Browser access boundary

Status: browser-boundary direction accepted for the tested Linux/Android clients;
production cookie/HTTPS and network release gates are tracked
in the compatibility matrix. This records architecture for #2, not approval to
release the Python prototype or a completed #10 implementation.

## Product requirements

One shared directory names the owner's services. Either Linux computer must
access services across networks. Android must open them in its ordinary browser
without manual home-router forwarding. Application traffic goes directly to its
hosting agent (or through an encrypted transport relay), never through the registry.

## Sequence and ownership

    Directory selection on accessing device
      -> short-lived, single-use browser bootstrap
      -> service-specific loopback hostname and HTTP gateway
      -> authenticated Iroh connection to the registered hosting endpoint
      -> host authorizes authenticated peer + requested service ID
      -> host opens its fixed local backend
      -> streaming response returns along the same path

The experiment currently substitutes a fixed loopback HTTP upstream for Iroh.
It does not implement enrollment, registry policy, host grants, or Internet routing.
The production gateway must keep the HTTP security checks on the browser-facing
side even if #5 transports an opaque byte stream.

## Decisions established by the experiment

- Loopback binding alone is not authorization. Require a browser session before
  any backend connection. Keep administrative control separate and authenticated.
- Different ports are not cookie isolation. A real Chromium test showed two
  127.0.0.1 services overwriting the same application cookie. Use distinct service
  hostnames; separate ports can remain an implementation detail.
- Evaluate per-service random .localhost hostnames as the smallest initial naming
  scheme. Linux Chromium resolves these to loopback and treats the pages as secure
  contexts in this experiment. This does not prove Secure-cookie acceptance on all
  clients, encrypted HTTP, or cross-application protection for every cookie domain.
- Validate exact Host including assigned port, reject duplicate/unregistered
  authorities, and select upstreams only from host configuration. Never derive an
  upstream from Host, query parameters, or a client-supplied URL.
- Bootstrap uses a short-lived token in a fragment, removes the fragment before
  handing off to the application, and exchanges it once using a same-origin POST.
  Browser sessions use HttpOnly, host-only, SameSite=Strict cookies with server-side
  expiry. Session credentials are stripped before forwarding to the backend.
- Require exact Origin for unsafe methods and WebSocket upgrades. Validate Origin
  whenever supplied. Deny cross-site and sibling-site Fetch Metadata requests,
  including GETs; ordinary authenticated navigation without Origin remains valid.
  Cross-site top-level links to protected application routes are deliberately denied.
  The public bootstrap landing page alone permits external top-level document
  navigation: Android Chrome marks ADB-launched URLs cross-site. That page issues
  no authority and makes no backend request until a valid one-use token is exchanged
  in a same-origin POST. Embedding remains denied and covered by a regression test.
- Confine forwarded application cookies to the service hostname, reserve the
  gateway credential cookie, strip hop-by-hop and untrusted forwarding headers,
  and rewrite only absolute Location values that point to the exact fixed upstream.
  Embedded absolute URLs, third-party auth redirects and deliberate shared-domain
  cookies require application-specific compatibility validation, not broad rewriting.
- Stream responses with bounded chunks and backpressure. Close the upstream on
  client disconnect. Never retry side-effecting requests automatically.

The HTTP experiment does not set Secure/__Host- on its gateway cookie. Production
cookie-prefix, HTTPS and certificate decisions remain release gates in #10 after
the tests owned by #10. The tested applications currently work through the HTTP
prototype; other clients/cookie policies are not supported by this evidence. Do not equate a browser secure-context
flag with transport encryption or declare plaintext sessions production-safe.

## Trust boundaries

| Actor or surface | Trust and enforcement |
| --- | --- |
| Enrolled device | Cryptographic identity is not proof of current user intent; host grants are still required |
| Registry administrator | Can approve devices and change policy; must not proxy application payloads; clients validate controller identity and policy freshness |
| Hosting agent | Owns service bindings; authorizes the actual transport peer before connecting to any backend |
| Local user/processes | Same-UID code and root can inspect state/memory; this design does not isolate hostile local processes or backend scripts |
| Hosted application | May execute JavaScript in its own service origin; must not receive gateway credentials or authorize another service |
| Unrelated web page | Untrusted even if it can reach loopback; enforce session, Host, Origin and Fetch Metadata before backend access |
| Relay operator | Handles encrypted transport; reachability/timing metadata may be visible; relay availability is a dependency |
| Android browser/app handoff | Requires explicit bootstrap and a lifecycle strategy; USB tests do not prove a persistent background agent |

## Release gaps and owning issues

| Gap | Owner |
| --- | --- |
| Native Android gateway, ordinary-browser handoff and lifecycle/network transitions | #12; compatibility evidence remains #2 |
| Linux/Android Secure cookie, prefix, hostname and HTTPS decisions | #10, validated under #2 |
| Per-service isolation against malicious cookie Domain settings, XSS, service workers and port/hostname reuse | #10 |
| Protected administrative API and authenticated nonbrowser API access | #10 |
| Native gateway integration with authenticated transport and host-side authorization | #5 foundation landed; integration #10/#14 |
| Atomic policy changes, active-stream revocation and expiry | #9 |
| Direct/relay, LAN/separate Wi-Fi/cellular and no-router-forwarding proof | #13/#14 |
| Resource/session limits, long streams, slow peers, malformed headers and lifecycle stress | #10/#14 |
| Application-specific login, Secure/domain cookies, absolute URLs and external redirects | #2/#10 |

## Reproduce and references

See [browser compatibility matrix](../browser-compatibility.md) and the disposable
[spike](../../experiments/browser/README.md). Normative browser behavior is informed by
[HTTP cookies](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Cookies),
[Origin](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Origin)
and [Fetch Metadata](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-Fetch-Site).
Runtime browser observations are distinguished from these references in the matrix.
