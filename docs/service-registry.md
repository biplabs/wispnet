# Local service registry (API version 1)

Android and Linux expose the same loopback HTTP API while Wispnet access is
online. Native apps on that device can use services granted to the device,
without separate per-app approval. Owning-host pairing and service grants still
apply to every request. This replaces Android's Messenger discovery/consent API.
No Android package queries, Binder connection or app permission prompt is needed.

## Shared HTTP port

Open **Settings** in either app. The registry and every HTTP service share port **7842** by default:

```
http://wispnet.localhost:7842/registry
```

The previous `http://registry.wispnet.localhost:7842`, `http://127.0.0.1:7842`
and `http://localhost:7842` addresses remain accepted with their `/v1/...` paths.
`GET /registry` returns the service directory. The new base also supports
`/registry/v1/services` and `/registry/v1/resolve`. It binds
only to IPv4 loopback, never the LAN. All website origins are allowed without
an origin list. Native clients without browser Origin/Fetch Metadata headers use
device-level access. This is discovery/access only; the
registry never exposes pairing, policy changes or settings commands.

**Wispnet port** in device Settings changes the shared port on that device.
Changing it changes every service URL and the registry URL. Service cards have
no individual Wispnet port setting. For example, both of these use one loopback listener:

```
http://j-strix-0123456789ab-llama.localhost:7842/
http://j-strix-0123456789ab-gike.localhost:7842/
```

The listener dispatches by the exact HTTP Host authority, never by URL paths or
backend addresses supplied by callers. Unknown or duplicate Host headers are
rejected. Local-owner routes connect directly to their registered backend;
remote routes use the authenticated P2P tunnel and current host grants. HTTP,
REST, SSE and WebSocket services are supported; raw TCP is not multiplexed here.

This is a TCP listener. Iroh's local UDP transport port remains automatically
assigned; the relay's UDP discovery port is separately configured as 7842.

Choose a port in 1024–65535. An occupied port produces an error without silently
selecting another port. Failed live changes restore previous settings and routes;
if recovery fails, access stops with an error. Port changes interrupt streams;
requests are never replayed. Taking Wispnet offline closes the shared listener.

On upgrade, the saved registry port becomes the shared port, preserving explicit
user configuration. Legacy per-service ports are ignored; old bookmarks to those
ports must be updated. Newly installed devices default to 7842. Devices with
the same port and owning-host name derive the same HTTP addresses, including
service IDs that need sanitization or truncation. The host's canonical name is
read from that host, rather than derived from local display aliases.

Known routes are restored at access startup. Wispnet also discovers permitted
services when first going online, so copied HTTP addresses can work without
opening each service tile. Offline hosts may require a later registry refresh.
Saved catalog entries never authorize a request.

Headless Linux uses the same settings through the owner-only `wispnet-node`
helper. `{"op":"gateway_settings"}` returns `settings`, saved `routes` and
`enabled`. Send `{"op":"configure_access","settings":...}` to replace settings:

```json
{"registry_port":7842,"service_ports":[]}
```

Settings are saved privately in `state_dir/access-settings.json`. Use the owner
API rather than editing this file under a running node. The owner-API field remains named `registry_port` for compatibility; it now
controls the shared HTTP listener. These settings affect only this device; they never change the remote host's backend or permissions.

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
const registry = 'http://wispnet.localhost:7842/registry';
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
additional restrictions. Browser pages also use the stable service URL directly,
without a Wispnet session cookie or launch token. Going online restores saved
listeners before the desktop reports online, including after a node restart.
Local native apps share the device's trust boundary; browser headers do not
authenticate native processes. Application login cookies remain application-owned.

## Stable Wispnet links

A service also has a port-independent identity link:

```
wispnet://FULL_HOST_ID/gike
```

HTTP service cards and service settings offer **Copy Wispnet link**. Android, Linux desktop and macOS
register the scheme with the OS. Opening a link brings up Wispnet, enables local
access, checks current pairing/service authority, resolves the latest service
incarnation and opens its HTTP URL in the browser. An unknown host is not paired
automatically. Links carry no credentials or permissions. Renames, local port
changes and service recreation do not change the identity link; removed or denied
services cannot resolve.

Native apps can POST `{"link":"wispnet://FULL_HOST_ID/gike"}` to `/v1/resolve`
instead of `host_id` and `service_id`. Directory and resolve records include a
`link` field alongside the device-local `url`. Owner-only node callers can send
`{"op":"resolve_link","link":"wispnet://FULL_HOST_ID/gike"}`. Ordinary HTTP
clients still call the resolved HTTP address; a custom-scheme link is not itself
a REST transport. Linux registration is installed through the updated desktop
launcher (`MimeType=x-scheme-handler/wispnet`).

## Local MCP configuration

The same loopback listener serves **`http://wispnet.localhost:7842/mcp`** for
Codex and other local MCP clients. Service hostnames remain isolated. MCP is
stateless Streamable HTTP (JSON responses; no server event stream). It supports
initialize, ping, tools/list and tools/call. GET and DELETE return 405. Send
`Accept: application/json, text/event-stream` and `Content-Type: application/json`.

Every MCP request needs `Authorization: Bearer <token>`. The token is generated
once in private `state_dir/mcp-token` (0600) and survives restarts and port changes.
It is separate from Wispnet host identity, pairings and service grants. Never
publish the token, put it in a URL, or share it with websites. MCP rejects all
browser Origin/Fetch Metadata headers, sends no CORS permission, and rejects
missing/duplicate credentials. The discovery API retains its existing CORS policy.

After installing the updated desktop/node and bringing local access online, add
the following to your private Codex config (`~/.codex/config.toml`). Replace
`/home/YOU` with your actual home directory:

```toml
[mcp_servers.wispnet]
url = "http://wispnet.localhost:7842/mcp"
http_headers_helper = "python3 /home/YOU/.local/libexec/wispnet-node/mcp_headers.py"
startup_timeout_sec = 15
tool_timeout_sec = 35
```

The header helper reads the private token on demand; the Codex config contains
no token. It accepts `--config /absolute/path/host.toml` for another local node.
Restart/reconnect the Codex MCP client after registration. Other MCP clients can
read the same private token and send the bearer header themselves. See the
[Codex MCP configuration documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

Tools include service discovery/link resolution, gateway settings, port changes
and local access online/offline. On desktop, the running owner-only `node.sock`
also enables host status, transport diagnostics, service configuration/removal,
pairing invitation approval/rejection and client revocation. Policy writes require
the current `expected_revision` from `wispnet_status` and use the same owner API
and validation as the app. An explicit service `allow` list replaces the entire
list; it never implicitly grants every client. Pairing approval requires an
existing pending invitation. Android exposes discovery and device-local access
settings through this shared implementation; desktop owner tools are not listed
without the owner socket.

Changing the shared port moves both registry and MCP: update the client URL
and reconnect. Going offline closes both endpoints; use the app or owner socket
to go online again. In-flight MCP port/offline changes finish their bounded reply
even as the listener closes. An uncertain transport failure is never an instruction
to repeat a write: inspect status/current revision first. No requests are replayed.
