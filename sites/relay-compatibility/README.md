# Private Sites / Iroh 1.3 compatibility spike

This is an experiment, not Wispnet's configured relay. The Worker implements the
Iroh 1.3 v2 signed-challenge handshake and transient datagram forwarding using an
isolate-local socket map. It has no D1/R2, catalog, permissions or stored keys.
It intentionally does not pretend that a JavaScript global provides shared
routing across Workers. Do not use this as a production relay.

See [the results](../../docs/sites-relay-validation.md). The current native Iroh
client also cannot supply Sites' required private service-access header; the
isolated Rust probe supplies it to test wire compatibility independently.

Rebuild and verify with Node 22+:

```sh
npm ci --ignore-scripts
npm run bundle
npm test
bash scripts/build.sh
node scripts/validate-artifact.mjs
```

`src/relay.js` is the authored source; `worker/index.js` is its committed standalone
bundle, including the pinned Noble BLAKE3 implementation. Tests run against the
bundle, including nonce mismatch and tampered-signature rejection, without npm
installation. Rebundle after source/dependency changes. The Worker ESM starter's
build copies that bundle and `.openai/hosting.json` into `dist/`.

Publish through the Sites skill's source helper from the separate Sites checkout,
using the persisted project ID and owner-private deployment operation. Do not
initialize another Git repository inside this Wispnet checkout. Credentials go
through hidden stdin only, never files, source, arguments or logs. Retain the
private audience and existing service token. No token is needed for local tests.

The Durable Object follow-up adds `/capabilities` and `/relay-durable`. The latter
requires `env.RELAY_ROOMS` and returns HTTP 503 when absent; every request selects
the same named object. `RelayRoom` owns only in-memory live connection state.
The existing `/relay` remains the isolate-local comparison experiment.

`durable-objects.probe.json` records standard Cloudflare declaration fields that
Sites **rejected** as extra manifest inputs. They are not active hosting fields.
Do not treat the exported class or a successful Worker deployment as proof that
Sites created a namespace. Local tests use a fake namespace and test routing and
failure behavior; the physical-device check is required for hosted acceptance.
