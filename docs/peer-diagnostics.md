# Peer path diagnostics

`peer-probe` authenticates the expected host, fetches its permitted catalog, and
prints one JSON snapshot. `peer-watch` repeats that read-only check using one
persistent identity lease and Iroh endpoint until SIGTERM or Ctrl-C.

```sh
target/debug/wipsnet peer-probe --config /absolute/path/client.toml --peer PINNED_HOST_ID
target/debug/wipsnet peer-watch --config /absolute/path/client.toml --peer PINNED_HOST_ID
```

Both accept one optional `--address IP:UDP_PORT` hint after the peer ID. Without
that hint, configure `transport.relay_url`; no directory lookup is needed. The
host must independently approve the client before returning a catalog. Probes
never open a backend, mutate policy, enroll a client, or replay a service operation.

Successful reports contain the expected host and own client ID, observed
`selected_path` (`direct`, `relay`, or `unknown`), `relay_status` (`disabled`,
`connected`, `denied`, or `unavailable`), catalog generation/revision, permitted
service count, and elapsed milliseconds. No backend IPs, service payloads, grants,
owner tokens, certificate contents, or raw relay error strings are printed.
The transport pin and catalog host are verified before success is reported.
An API caller supplying a target address for a different ID fails before dialing.

A probe allows the existing five-second connection and five-second control-RPC
deadlines, then up to one second for a direct path to replace an initial relay
path. A path snapshot reflects the time of observation, not an availability or
future-route guarantee. Relay reachability and peer reachability are separate:
a direct peer can work while the configured relay is unavailable.

Watch emits one compact JSON line per completed probe. Successful probes pause
one second; consecutive failures pause 1, 2, 4, 8, 16, then at most 30 seconds
after each completed attempt, in addition to the bounded probe deadlines above.
Each watch owns its retry state independently. Success or a network-change
notification resets the failure backoff; notifications interrupt the wait and
start a fresh probe. Watch reports include `retry_after_ms` for this scheduled
pause (one-shot `peer-probe` does not retry or include that field).
Failures emit fixed redacted errors and omit private catalog
metadata. Pending probes are canceled on shutdown, their connections are closed,
and the endpoint/key lease is released. An unrelated host is not consulted.
Catalog observations are diagnostic snapshots, not service authorization tickets
or a persisted service cache; #9 owns cache/revocation integration.

## Explicit network-change notification

The pinned Iroh 1.3.0 API exposes `Endpoint::network_change()` for platforms whose
native route monitor cannot observe changes. Its Android native monitor is a
stub; a future Android app must forward its platform network callback to the
same owned endpoint. Replacing its key or restarting the endpoint is unnecessary.

`peer-watch` provides a local SIGUSR1 bridge for native testing:

```sh
kill -USR1 WATCH_PID
```

This cancels an in-flight read-only diagnostic if needed, notifies the existing
endpoint, and probes again with the same pin and key. It does not accept remote
administrative instructions. The signal bridge demonstrates the transport API;
it does not implement Android app/JNI callback registration (#12).

The physical test in [native relay validation](native-relay-validation.md) keeps
one Android watch process alive across Wi-Fi off/on, sends notifications after
interface changes, and verifies direct LAN → USB-reachable TLS relay → direct
LAN with unchanged identity and catalog. No seamless application-stream migration
or automatic retry of a potentially state-changing request is promised.
