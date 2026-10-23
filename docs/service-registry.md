# Local service registry (API version 1)

Android and Linux expose the same loopback HTTP API while Wispnet access is
online. Native apps on that device can use services granted to the device,
without separate per-app approval. Owning-host pairing and service grants still
apply to every request. This replaces Android's Messenger discovery/consent API.
No Android package queries, Binder connection or app permission prompt is needed.

## Configure ports

Open **Settings** in either app. The registry defaults to port **18400**:

```
http://registry.wispnet.localhost:18400
```

`http://127.0.0.1:18400` and `http://localhost:18400` are also accepted. It binds
only to IPv4 loopback, never the LAN. All website origins are allowed without
an origin list. Native clients without browser Origin/Fetch Metadata headers use
device-level access. This is discovery/access only; the
registry never exposes pairing, policy changes or settings commands.

On Linux, Settings lists each known HTTP service's **gateway port**.
On Android, use the service card's gear to set that device's gateway port.
These ports are separate from the owning machine's backend port. For example,
llama.cpp can run at backend port 8201 while the phone and Linux gateway both use
18401. Leave the gateway port blank to use automatic allocation; an existing
saved automatic port remains stable. Chosen ports must be unique and in
1024–65535. Port conflicts produce an error; no silent fallback occurs.
Changing settings reconnects active sessions and resets browser sessions.
Failed live listener changes restore the previous settings and routes; if that
recovery fails, access stops with an error. Taking Wispnet offline closes the
registry and service connections. Settings and saved addresses survive restart.

Configured service ports open when access starts or settings are saved while
online; previously opened automatic routes are restored too. A newly configured
service must be reachable to establish its first route. Configure/copy the
endpoint on each device; host identity is part of the gateway hostname, so the
complete URL also depends on the chosen host and service, not just its port.

Headless Linux uses the same settings through the owner-only `wispnet-node`
helper. `{"op":"gateway_settings"}` returns `settings`, saved `routes` and
`enabled`. Send `{"op":"configure_access","settings":...}` to replace settings:

```json
{
  "registry_port": 18400,
  "service_ports": [
    {"host":"FULL_HOST_ID", "service":"llama", "port":18401}
  ]
}
```

Settings are saved privately in `state_dir/access-settings.json`. Use the owner
API rather than editing this file under a running node. These settings affect
only this device; they never change the remote host's backend or permissions.

## Discover and resolve

`GET /v1/services` returns fresh permitted HTTP service records:

```json
{
  "api_version": 1,
  "services": [
    {"host_id":"FULL_HOST_ID", "service_id":"llama", "display_name":"llama",
     "kind":"http", "incarnation":"SERVICE_INCARNATION", "url":"http://HOST_SERVICE.localhost:PORT/"}
  ],
  "errors": []
}
```

Listing creates a local browser route for each newly discovered permitted HTTP
service, so `url` is usable without first opening it in the app or calling resolve.
Existing saved addresses remain stable. If route creation fails (for example,
all saved browser-service slots are used), `url` is null and the record includes
an `error` explaining why. Discovery and route creation use fresh owner checks;
listing never changes service grants or publishes a service to the internet. URLs never expose backend addresses, browser login links, or access tokens.
Offline or denied remote hosts are reported separately in `errors`; cached
catalogs do not authorize access. `POST /v1/resolve` remains available to recheck
authority and obtain a route explicitly:

```js
const registry = 'http://127.0.0.1:18400';
const directory = await fetch(`${registry}/v1/services`).then(r => r.json());
const service = directory.services.find(s => s.service_id === 'llama');
if (!service) throw new Error('Llama is unavailable');
const response = await fetch(`${registry}/v1/resolve`, {
  method: 'POST',
  headers: {'Content-Type': 'application/json'},
  body: JSON.stringify({host_id: service.host_id, service_id: service.service_id})
});
if (!response.ok) throw new Error('Cannot open service');
const endpoint = await response.json();
const models = await fetch(new URL('/v1/models', endpoint.url)).then(r => r.json());
```

Resolve returns `url`, `host_id`, `service_id` and `api_version`. No access token,
credential header, expiry or renewal is required. Call the URL directly, including
for POST and SSE requests. A service's own `Authorization` API key remains required
if that service enforces one. Re-resolve after connection failures or port changes.
For a known fixed service URL, call it directly without contacting the registry.
The obsolete `/_wispnet/access` token endpoint is removed (404).

Some Android Java clients cannot resolve wildcard `.localhost`. Connect to
127.0.0.1 at the returned port and send the URL's exact authority in `Host`.
Do not substitute `Host: 127.0.0.1` on service routes. The independent Android
`discovery-example` uses the registry over HTTP and demonstrates this fallback.

Browser callers from any origin (including opaque `null` origins) can use both
the registry and service routes, without a token or cookie for API calls.
Duplicate or malformed Origin headers and cross-site requests that omit Origin
are rejected. Native clients send no browser Origin/Fetch Metadata headers and use
the device's host/service grants directly. Retired `X-Wispnet-Access` headers are
ignored and stripped upstream. Legacy saved `allowed_origins` lists are ignored.

Browser localhost/network permissions and secure-context rules can impose
additional restrictions. Existing browser navigation retains its single-use
handoff and same-origin session-cookie mechanism; these browser session cookies
are separate from the removed API access tokens. Local native apps share the
device's trust boundary; browser headers do not authenticate native processes.
