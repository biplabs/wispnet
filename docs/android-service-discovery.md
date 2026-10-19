# Android service discovery API (version 1)

Other Android apps can resolve a paired host's HTTP service without configuring
or predicting a local port. Wispnet must be installed on that phone and the
phone must already be paired with, and granted access by, the owning host.
The discovery API cannot approve a phone on a remote host, change grants, pair
arbitrary hosts, or execute native management commands.

## Connect and resolve

Bind to the explicit component:

```java
Intent intent = new Intent().setComponent(new ComponentName(
    "app.wispnet.android", "com.biplabs.wispnet.ServiceDiscoveryService"));
bindService(intent, connection, Context.BIND_AUTO_CREATE);
```

Declare package visibility and Internet access in the caller's manifest:

```xml
<uses-permission android:name="android.permission.INTERNET"/>
<queries><package android:name="app.wispnet.android"/></queries>
```

The caller also needs cleartext access to the loopback HTTP gateway in its own
network security configuration (the example uses `usesCleartextTraffic="true"`).
Do not enable cleartext for remote Internet endpoints merely to use this API.

In `onServiceConnected`, construct `new Messenger(binder)`. Send a `Message`
with `what = 1`, a `replyTo` Messenger, and these Bundle fields:

| Field | Type | Value |
| --- | --- | --- |
| `api_version` | int | `1` |
| `request_id` | String | Caller correlation value, at most 128 characters |
| `package_name` | String | The caller's own package name |
| `host_id` | String | Full 64-character lowercase hexadecimal host ID |
| `service_id` | String | Service ID, such as `llama` |

```java
Message message = Message.obtain(null, 1);
Bundle request = new Bundle();
request.putInt("api_version", 1);
request.putString("request_id", "llama-connection");
request.putString("package_name", getPackageName());
request.putString("host_id", fullHostId);
request.putString("service_id", "llama");
message.setData(request);
message.replyTo = replies;
discovery.send(message);
```

Replies use `what = 1`, `api_version = 1`, the original `request_id`, and a
`status`. Service work runs outside the main thread and the request queue is
bounded; clients must handle `busy` with bounded backoff. Unbind when finished.
No signature-level Android permission is required: separately signed apps may
request access, but cannot bypass the user approval below.

## Approval and endpoint response

The first resolve returns `permission_required` and a `consent` PendingIntent.
Launch its IntentSender from a foreground activity using
`startIntentSenderForResult`. Android 14+ requires the sender's explicit
activity-launch opt-in, including when using an IntentSender:

```java
ActivityOptions options = ActivityOptions.makeBasic();
options.setPendingIntentBackgroundActivityStartMode(
    Build.VERSION.SDK_INT >= 36
        ? ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_IF_VISIBLE
        : ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED);
startIntentSenderForResult(consent.getIntentSender(), 1, null,
    0, 0, 0, options.toBundle());
```

Only launch from a visible, user-initiated flow. See Android's
[activity security guidance](https://developer.android.com/guide/components/activities/secure-bal).
 Wispnet shows the app's installed label/package,
the requested host and service, and explicit Allow/Deny buttons. Denial or
cancellation grants nothing. Do not repeatedly prompt following denial.

If the activity returns `RESULT_OK`, resolve again when Wispnet is online.
Approval may start Wispnet's normal foreground access service; startup is
asynchronous, so handle `offline` until the user-visible access service is ready.
The consent Activity is not exported and the PendingIntent is immutable.

An `ok` response contains:

| Field | Type | Meaning |
| --- | --- | --- |
| `url` | String | Current loopback root URL, including its assigned port |
| `header_name` | String | `X-Wispnet-Access` |
| `access_token` | String | Secret credential for this app grant and service route |
| `expires_in_seconds` | int | `900` seconds |
| `host_id`, `service_id` | String | Resolved target |

Make ordinary HTTP requests relative to `url`, adding the returned header on
**every** request, including streaming/SSE requests. The gateway removes this
header before forwarding to the backend. An upstream `Authorization` header
remains available for services that have their own API key. Never put the token
in a query string, logs, analytics, clipboard or exported Intent.

Some Java networking stacks do not resolve wildcard `.localhost` names. In that
case, connect to `127.0.0.1` at the returned port and send the returned URL's
**exact authority (hostname and port)** in the HTTP `Host` header. Never substitute
`Host: 127.0.0.1`. The independent example demonstrates this without configuring
any service port. Preserve this Host/credential pairing on every request and do
not follow redirects to another origin with the credential attached.

Save the full host ID and service ID. Keep endpoint credentials in memory and
resolve again after HTTP 401, connection failure, network changes, or Wispnet
restarts. Renewing a grant invalidates its previous credential, so serialize
refreshes within the consuming app. Do not automatically replay writes or
partially consumed streams. Discovery does not currently push lifecycle events;
clients detect failed requests and resolve again. Stopping Wispnet closes routes
and invalidates all issued credentials. Credentials are never persisted.

## Permissions and revocation

Wispnet obtains the sender UID from Binder, verifies the package belongs to that
UID, and binds approval to the package's current signing certificate, full host
ID, service ID, and service incarnation. Shared-UID/multiple-signer callers are
currently rejected. A replacement service requires fresh approval. Every
resolve fetches a live catalog and every service request remains subject to the
owning host's current permissions. Cached listings never grant access.

In Wispnet, tap the **gear on the service card** to open its **App permissions**
sheet, styled like the machine-pairing sheet, then revoke the specific app. The list includes only grants for that host
and service, including grants for older service instances. Revocation persists denial first and cancels the
route's requests and credentials. Other clients of that same route may need to
resolve or reopen it. Other routes are unaffected. If native revocation fails,
Wispnet stops access instead of leaving that grant usable.

The caller's credential is a bearer capability. Approval controls issuance via
this API; it is not OS isolation from a hostile app with arbitrary local TCP
access. The existing browser gateway accepts browser navigation to establish
browser sessions, and native apps can imitate browser headers. Strong isolation
between hostile local applications would require an additional transport/OS
boundary. The API does not strengthen that existing loopback trust boundary.

## Statuses

| Status | Meaning/action |
| --- | --- |
| `ok` | Use returned endpoint and credential |
| `permission_required` | Offer the supplied consent UI |
| `offline` | Ask the user to bring Wispnet online, then retry |
| `host_not_paired` | Pair this phone with the host in Wispnet first |
| `host_denied` | The host has not approved this phone or revoked it |
| `service_unavailable` | Service absent, withdrawn, unreachable, or unverifiable |
| `unsupported_protocol` | Only HTTP services are supported in version 1 |
| `caller_rejected` | Package/UID/signature identity could not be established |
| `invalid_request` | Malformed IDs or request fields |
| `unsupported_version` | Wrong protocol version or message operation |
| `busy` | Queue full; retry with bounded backoff |

## Working independent app and tests

`apps/android/discovery-example` is a separate Android application with its own
UID and no Wispnet implementation dependency. It demonstrates Binder binding,
consent, resolving and authenticated HTTP. Build it with:

```sh
cd apps/android
JAVA_HOME=/opt/android-studio/jbr ./gradlew :discovery-example:assembleDebug
```

The example defaults to the production package; tests pass the isolated
`app.wispnet.android.peercheck` package explicitly. It is an example/development
artifact and is not included in Wispnet's production APK or Play bundle.

Run native gateway tests and build the current Android native library first:

```sh
cargo test -p wispnet-client --locked
scripts/build_android.sh
cd apps/android
JAVA_HOME=/opt/android-studio/jbr ./gradlew :app:assemblePeercheck \
  :app:assemblePeercheckAndroidTest :discovery-example:assembleDebug \
  -PwispnetTestBuildType=peercheck
cd ../..
python3 scripts/test_android_discovery.py --serial AUTHORIZED_ADB_SERIAL
```

The device test uses the real Binder boundary from the independent sample UID,
checks package spoof rejection, malformed/absent services, consent denial and
approval, HTTP access, UI revocation/old credential rejection, and host grant
withdrawal. It creates a temporary host service and a separate phone identity,
then removes the service, revokes the fixture peer, and uninstalls the test apps.
It does not replace the user's installed app or stop either live host/relay.
The native tests additionally verify wrong-route credentials, invalid and expired
tokens, renewal, cross-origin rejection and upstream credential stripping.
