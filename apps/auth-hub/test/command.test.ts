import test from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, randomBytes, sign, createHash, type KeyObject } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { serializeSignedCookie } from "better-call";
import { CommandCenter, publishedHostname } from "../src/app.js";
import { validateConfig } from "../src/config.js";

interface Actor { id: string; cookie: string; }
async function fixture(t: test.TestContext) {
  const dir = mkdtempSync(join(tmpdir(), "wispnet-command-"));
  const baseURL = "http://127.0.0.1:43210";
  const app = await CommandCenter.open({ baseURL, stateDir: dir, publicDomain: "wispnet.test", secret: "command-test-secret-".repeat(4), testMode: true });
  t.after(() => { app.close(); rmSync(dir, { recursive: true, force: true }); });
  async function actor(name: string): Promise<Actor> {
    const ctx = await app.auth.$context;
    const user = await ctx.internalAdapter.createUser({ name, email: name.toLowerCase() + "@example.test", emailVerified: true }, { method: "oauth", oauth: { providerId: "github" } });
    const session = await ctx.internalAdapter.createSession(user.id);
    const cookie = await serializeSignedCookie(ctx.authCookies.sessionToken.name, session.token, app.config.secret);
    return { id: user.id, cookie: cookie.split(";")[0] };
  }
  async function request(path: string, body?: unknown, auth?: Actor | string, method?: string) {
    const headers = new Headers({ origin: baseURL });
    if (body !== undefined) headers.set("content-type", "application/json");
    if (typeof auth === "string") headers.set("authorization", "Bearer " + auth);
    else if (auth) headers.set("cookie", auth.cookie);
    return app.handle(new Request(baseURL + path, { method: method ?? (body === undefined ? "GET" : "POST"), headers, body: body === undefined ? undefined : JSON.stringify(body) }));
  }
  async function linked(owner: Actor, name = "j-desktop", previousKey?: { publicKey: KeyObject; privateKey: KeyObject }) {
    const key = previousKey ?? generateKeyPairSync("ed25519");
    const host = key.publicKey.export({ type: "spki", format: "der" }).subarray(-32).toString("hex");
    const nonce = randomBytes(32).toString("hex");
    const signature = sign(null, Buffer.from(`wispnet-command-link-v1\n${baseURL}\n${host}\n${name}\n${nonce}`), key.privateKey).toString("hex");
    const startBody = { host, name, nonce, signature };
    const started = await request("/api/command/links/start", startBody);
    assert.equal(started.status, 200, await started.clone().text());
    const codes = await started.json();
    assert.equal((await request("/api/command/links/poll", { device_code: codes.device_code })).status, 200);
    assert.equal((await request("/api/command/links/approve", { user_code: codes.user_code }, owner)).status, 200);
    const polled = await request("/api/command/links/poll", { device_code: codes.device_code });
    assert.equal(polled.status, 200);
    const { publisher_token } = await polled.json();
    return { host, name, publisher_token: publisher_token as string, startBody, codes, key };
  }
  async function service(host: Awaited<ReturnType<typeof linked>>, id = "gike-mcp") {
    const resource = `https://${publishedHostname(host.name, host.host, id, app.config.publicDomain)}/MCP`;
    const body = { host: host.host, service: id, incarnation: randomBytes(32).toString("hex"), name: host.name, resource };
    const response = await request("/api/command/host/register", body, host.publisher_token);
    assert.equal(response.status, 200, await response.clone().text());
    const data = await response.json();
    return { ...body, service_id: data.resource_id as string };
  }
  async function authorize(host: Awaited<ReturnType<typeof linked>>, target: Awaited<ReturnType<typeof service>>, token: string) {
    return request("/api/command/authorize", { ...target, token }, host.publisher_token);
  }
  async function oauth(user: Actor, target: Awaited<ReturnType<typeof service>>) {
    const clientResponse = await request("/api/auth/oauth2/register", {
      client_name: "Fixture MCP", redirect_uris: ["http://127.0.0.1:9911/callback"], application_type: "native",
      token_endpoint_auth_method: "none", grant_types: ["authorization_code", "refresh_token"], response_types: ["code"], scope: "mcp:use offline_access",
    });
    assert.equal(clientResponse.status, 201);
    const client = await clientResponse.json();
    const verifier = randomBytes(32).toString("base64url");
    const params = new URLSearchParams({ client_id: client.client_id, redirect_uri: "http://127.0.0.1:9911/callback", response_type: "code", resource: target.resource,
      code_challenge: createHash("sha256").update(verifier).digest("base64url"), code_challenge_method: "S256", scope: "mcp:use offline_access", state: "fixture-state" });
    const auth = await request("/api/auth/oauth2/authorize?" + params, undefined, user);
    assert.equal(auth.status, 302);
    const consentURL = new URL(auth.headers.get("location")!, baseURL);
    const consent = await request("/api/auth/oauth2/consent", { accept: true, oauth_query: consentURL.search.slice(1) }, user);
    assert.equal(consent.status, 200);
    const callback = new URL((await consent.json()).url);
    const exchange = async (body: URLSearchParams, endpoint = "token") => app.handle(new Request(baseURL + "/api/auth/oauth2/" + endpoint, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body }));
    const response = await exchange(new URLSearchParams({ client_id: client.client_id, grant_type: "authorization_code", code: callback.searchParams.get("code")!,
      redirect_uri: "http://127.0.0.1:9911/callback", code_verifier: verifier, resource: target.resource }));
    return { client, response, tokens: await response.json(), exchange };
  }
  return { app, dir, baseURL, actor, request, linked, service, authorize, oauth };
}

test("production is GitHub-only and exposes OAuth discovery without a password login", async (t) => {
  const f = await fixture(t);
  assert.throws(() => validateConfig({ ...f.app.config, testMode: false, baseURL: "https://cmd.wispnet.app" }), /GITHUB/);
  const discovery = await f.request("/.well-known/oauth-authorization-server");
  assert.equal(discovery.status, 200);
  const metadata = await discovery.json();
  assert.equal(metadata.issuer, f.baseURL);
  assert.equal(metadata.authorization_endpoint, f.baseURL + "/api/auth/oauth2/authorize");
  assert.equal(metadata.client_id_metadata_document_supported, true);
  assert.deepEqual(metadata.code_challenge_methods_supported, ["S256"]);
  const password = await f.request("/api/auth/sign-up/email", { name: "No", email: "no@example.test", password: "long-password" });
  assert.notEqual(password.status, 200);
  assert.equal((await f.request("/api/command/dashboard")).status, 401);
});

test("host linking proves the endpoint key, requires approval, consumes codes, and stores no publisher token", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner");
  const host = await f.linked(owner);
  assert.equal((await f.request("/api/command/links/start", host.startBody)).status, 409);
  assert.equal((await f.request("/api/command/links/start", { ...host.startBody, nonce: randomBytes(32).toString("hex") })).status, 403);
  assert.equal((await f.request("/api/command/links/poll", { device_code: host.codes.device_code })).status, 404);
  const stored = JSON.stringify(f.app.store.db.prepare("SELECT * FROM command_hosts").all());
  assert(!stored.includes(host.publisher_token));
  const info = await f.request("/api/command/dashboard", undefined, owner);
  assert.equal(info.status, 200);
  assert(!(await info.text()).includes(host.publisher_token));
});

test("publisher registration is host-bound and resource URLs match the gateway namespace", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner");
  const host = await f.linked(owner), other = await f.linked(owner, "j-strix");
  const service = await f.service(host);
  assert.equal((await f.request("/api/command/host/register", service, other.publisher_token)).status, 403);
  assert.equal((await f.request("/api/command/host/register", { ...service, resource: "https://attacker.example/MCP" }, host.publisher_token)).status, 400);
  assert.equal((await f.request("/api/command/authorize", { ...service, token: "bad" }, host.publisher_token)).status, 403);
});

test("bearer keys require a live service grant, remain service-bound, and stay revoked after regrant", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner"), alice = await f.actor("Alice");
  const host = await f.linked(owner), service = await f.service(host), other = await f.service(host, "other");
  assert.equal((await f.request("/api/command/keys/create", { service_id: service.service_id, name: "Alice" }, alice)).status, 403);
  assert.equal((await f.request("/api/command/grants", { service_id: service.service_id, user_id: alice.id, active: true }, owner)).status, 200);
  const created = await f.request("/api/command/keys/create", { service_id: service.service_id, name: "Alice MCP" }, alice);
  assert.equal(created.status, 200, await created.clone().text());
  const key = await created.json();
  assert.match(key.key, /^wpk_/);
  assert.equal((await f.authorize(host, service, key.key)).status, 200);
  assert.equal((await f.authorize(host, other, key.key)).status, 403);
  assert.equal((await f.request("/api/command/keys/create", { service_id: other.service_id, name: "Escalate" }, alice)).status, 403);
  assert.equal((await f.request("/api/auth/api-key/create", { permissions: { service: [other.service_id] } }, alice)).status, 404);
  assert(!(await (await f.request("/api/command/dashboard", undefined, alice)).text()).includes(key.key));
  assert(!readFileSync(join(f.dir, "command.sqlite")).includes(Buffer.from(key.key)));
  assert.equal((await f.request("/api/command/grants", { service_id: service.service_id, user_id: alice.id, active: false }, owner)).status, 200);
  assert.equal((await f.authorize(host, service, key.key)).status, 403);
  assert.equal((await f.request("/api/command/grants", { service_id: service.service_id, user_id: alice.id, active: true }, owner)).status, 200);
  assert.equal((await f.authorize(host, service, key.key)).status, 403);
});

test("owners are isolated and account mutations reject cross-origin requests", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner"), alice = await f.actor("Alice");
  const host = await f.linked(owner), service = await f.service(host);
  assert.equal((await f.request("/api/command/grants", { service_id: service.service_id, user_id: alice.id, active: true }, alice)).status, 403);
  assert.equal((await f.request("/api/command/services/mode", { service_id: service.service_id, auth_mode: "bearer" }, alice)).status, 403);
  const response = await f.app.handle(new Request(f.baseURL + "/api/command/hosts/unlink", { method: "POST", headers: { cookie: owner.cookie, origin: "https://attacker.example", "content-type": "application/json" }, body: JSON.stringify({ host: host.host }) }));
  assert.equal(response.status, 403);
});

test("malformed and oversized command requests fail before processing", async (t) => {
  const f = await fixture(t);
  assert.equal((await f.request("/api/command/links/start", null)).status, 400);
  assert.equal((await f.request("/api/command/links/start", { value: "x".repeat(65537) })).status, 413);
});

test("standard OAuth clients complete DCR, PKCE, consent, resource-bound access and renewal", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner");
  const host = await f.linked(owner), service = await f.service(host), other = await f.service(host, "other");
  const clientResponse = await f.request("/api/auth/oauth2/register", {
    client_name: "Fixture MCP", redirect_uris: ["http://127.0.0.1:9911/callback"],
    application_type: "native", token_endpoint_auth_method: "none", grant_types: ["authorization_code", "refresh_token"],
    response_types: ["code"], scope: "mcp:use offline_access",
  });
  assert.equal(clientResponse.status, 201, await clientResponse.clone().text());
  const client = await clientResponse.json();
  const verifier = randomBytes(32).toString("base64url");
  const params = new URLSearchParams({ client_id: client.client_id, redirect_uri: "http://127.0.0.1:9911/callback", response_type: "code", resource: service.resource,
    code_challenge: createHash("sha256").update(verifier).digest("base64url"), code_challenge_method: "S256", scope: "mcp:use offline_access", state: "fixture-state" });
  const authorization = await f.request("/api/auth/oauth2/authorize?" + params, undefined, owner);
  assert.equal(authorization.status, 302, await authorization.clone().text());
  const consentURL = new URL(authorization.headers.get("location")!, f.baseURL);
  assert.equal(consentURL.pathname, "/consent");
  const consent = await f.request("/api/auth/oauth2/consent", { accept: true, oauth_query: consentURL.search.slice(1) }, owner);
  assert.equal(consent.status, 200, await consent.clone().text());
  const callback = new URL((await consent.json()).url);
  assert.equal(callback.searchParams.get("state"), "fixture-state");
  const tokenParams = new URLSearchParams({ client_id: client.client_id, grant_type: "authorization_code", code: callback.searchParams.get("code")!,
    redirect_uri: "http://127.0.0.1:9911/callback", code_verifier: verifier, resource: service.resource });
  const exchange = async (body: URLSearchParams) => f.app.handle(new Request(f.baseURL + "/api/auth/oauth2/token", { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body }));
  const tokenResponse = await exchange(tokenParams);
  assert.equal(tokenResponse.status, 200, await tokenResponse.clone().text());
  const token = await tokenResponse.json();
  assert.equal(token.token_type.toLowerCase(), "bearer");
  assert.equal((await f.authorize(host, service, token.access_token)).status, 200);
  assert.equal((await f.authorize(host, other, token.access_token)).status, 403);
  const renewal = await exchange(new URLSearchParams({ grant_type: "refresh_token", client_id: client.client_id, refresh_token: token.refresh_token, resource: service.resource }));
  assert.equal(renewal.status, 200, await renewal.clone().text());
  const refreshed = await renewal.json();
  assert.equal((await f.authorize(host, service, refreshed.access_token)).status, 200);
  const connections = (await (await f.request("/api/command/dashboard", undefined, owner)).json()).connections;
  assert.equal((await f.request("/api/command/connections/revoke", { connection_id: connections[0].id }, owner)).status, 200);
  assert.equal((await f.authorize(host, service, token.access_token)).status, 403);
  assert.equal((await f.authorize(host, service, refreshed.access_token)).status, 403);
  assert.notEqual((await exchange(new URLSearchParams({ grant_type: "refresh_token", client_id: client.client_id, refresh_token: refreshed.refresh_token, resource: service.resource }))).status, 200);
  assert.notEqual((await exchange(tokenParams)).status, 200, "authorization code is single-use");
});

test("OAuth token issuance denies users without a service grant", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner"), alice = await f.actor("Alice");
  const host = await f.linked(owner), service = await f.service(host);
  const denied = await f.oauth(alice, service);
  assert.notEqual(denied.response.status, 200);
  assert.equal((await f.request("/api/command/grants", { service_id: service.service_id, user_id: alice.id, active: true }, owner)).status, 200);
  const allowed = await f.oauth(alice, service);
  assert.equal(allowed.response.status, 200);
  assert.equal((await f.authorize(host, service, allowed.tokens.access_token)).status, 200);
  await f.request("/api/command/grants", { service_id: service.service_id, user_id: alice.id, active: false }, owner);
  assert.equal((await f.authorize(host, service, allowed.tokens.access_token)).status, 403);
});

test("OAuth refresh and access-token revocation invalidate durable connections", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner"), host = await f.linked(owner), service = await f.service(host);
  const flow = await f.oauth(owner, service);
  assert.equal(flow.response.status, 200);
  const revoked = await flow.exchange(new URLSearchParams({ client_id: flow.client.client_id, token: flow.tokens.refresh_token, token_type_hint: "refresh_token" }), "revoke");
  assert.equal(revoked.status, 200, await revoked.clone().text());
  assert.equal((await f.authorize(host, service, flow.tokens.access_token)).status, 403);
  const fresh = await f.oauth(owner, service), wrong = await f.oauth(owner, service);
  const wrongRevocation = await fresh.exchange(new URLSearchParams({ client_id: wrong.client.client_id, token: fresh.tokens.access_token, token_type_hint: "access_token" }), "revoke");
  assert.notEqual(wrongRevocation.status, 200);
  assert.equal((await f.authorize(host, service, fresh.tokens.access_token)).status, 200);
  const jwtRevoke = await fresh.exchange(new URLSearchParams({ client_id: fresh.client.client_id, token: fresh.tokens.access_token, token_type_hint: "access_token" }), "revoke");
  assert.equal(jwtRevoke.status, 200);
  assert.equal((await f.authorize(host, service, fresh.tokens.access_token)).status, 403);
});

test("unlink and same-owner relink never revive old OAuth credentials", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner"), host = await f.linked(owner), service = await f.service(host);
  const flow = await f.oauth(owner, service);
  assert.equal(flow.response.status, 200);
  assert.equal((await f.request("/api/command/host/unlink", { host: host.host }, host.publisher_token)).status, 200);
  const relinked = await f.linked(owner, host.name, host.key);
  assert.equal((await f.request("/api/command/host/register", service, relinked.publisher_token)).status, 200);
  assert.equal((await f.authorize(relinked, service, flow.tokens.access_token)).status, 403);
  const refresh = await flow.exchange(new URLSearchParams({ client_id: flow.client.client_id, grant_type: "refresh_token", refresh_token: flow.tokens.refresh_token, resource: service.resource }));
  assert.notEqual(refresh.status, 200);
  const fresh = await f.oauth(owner, service);
  assert.equal(fresh.response.status, 200);
  assert.equal((await f.authorize(relinked, service, fresh.tokens.access_token)).status, 200);
});

test("backend replacement, key revocation and host unlink fail closed", async (t) => {
  const f = await fixture(t); const owner = await f.actor("Owner"), host = await f.linked(owner), service = await f.service(host);
  const key = await (await f.request("/api/command/keys/create", { service_id: service.service_id, name: "Test" }, owner)).json();
  assert.equal((await f.authorize(host, service, key.key)).status, 200);
  assert.equal((await f.request("/api/command/keys/revoke", { key_id: key.id }, owner)).status, 200);
  assert.equal((await f.authorize(host, service, key.key)).status, 403);
  const nextKey = await (await f.request("/api/command/keys/create", { service_id: service.service_id, name: "Replacement" }, owner)).json();
  const replacement = { ...service, incarnation: randomBytes(32).toString("hex") };
  assert.equal((await f.request("/api/command/host/register", replacement, host.publisher_token)).status, 200);
  assert.equal((await f.authorize(host, replacement, nextKey.key)).status, 403);
  assert.equal((await f.request("/api/command/host/unlink", { host: host.host }, host.publisher_token)).status, 200);
  assert.equal((await f.request("/api/command/host/register", replacement, host.publisher_token)).status, 403);
});
