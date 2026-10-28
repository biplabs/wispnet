import { createHash, createPublicKey, randomBytes, randomUUID, verify } from "node:crypto";
import { createLocalJWKSet, jwtVerify } from "jose";
import { oauthProviderAuthServerMetadata, oauthProviderOpenIdConfigMetadata } from "@better-auth/oauth-provider";
import { createAuth, ensureOAuthResource, migrateAuth, type CommandAuth } from "./auth.js";
import { validateConfig, type HubConfig } from "./config.js";
import { CommandStore, digest, now, openStore, secretToken, type LinkedHost, type Service } from "./store.js";
import { page, script, stylesheet } from "./ui.js";

class HttpError extends Error { constructor(readonly status: number, message: string) { super(message); } }
function required(value: unknown, max = 2048): string {
  if (typeof value !== "string" || !value.length || value.length > max || /[\u0000-\u001f\u007f]/.test(value)) throw new HttpError(400, "Invalid request");
  return value;
}
function hex(value: unknown): string { const text = required(value, 64); if (!/^[a-f0-9]{64}$/.test(text)) throw new HttpError(400, "Invalid device or service identity"); return text; }
function serviceId(value: unknown): string { const text = required(value, 128); if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(text)) throw new HttpError(400, "Invalid service"); return text; }
function json(value: unknown, status = 200): Response { return Response.json(value, { status }); }
async function objectBody(request: Request): Promise<Record<string, unknown>> {
  try {
    const value = await request.json();
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error();
    return value;
  } catch { throw new HttpError(400, "A JSON object is required"); }
}
const safeUser = (user: { id: string; name: string; email: string }) => ({ id: user.id, name: user.name, email: user.email });

export function publishedHostname(name: string, host: string, service: string, domain: string): string {
  const slug = (text: string, limit: number): string => {
    let result = "";
    for (const value of Buffer.from(text)) {
      if (result.length === limit) break;
      const character = String.fromCharCode(value);
      if (/[a-z0-9]/i.test(character)) result += character.toLowerCase();
      else if (result && !result.endsWith("-")) result += "-";
    }
    return result.replace(/^-+|-+$/g, "");
  };
  let serviceSlug = slug(service, 24);
  if (serviceSlug !== service) serviceSlug = `${slug(service, 15)}-${createHash("sha1").update(service).digest("hex").slice(0, 8)}`;
  return `${slug(name, 20) || "node"}-${host.slice(0, 12)}-${serviceSlug}.${domain}`;
}

interface PendingLink { id: string; host: string; name: string; owner_id: string | null; expires: number; consumed: number; }
interface StoredKey { id: string; user_id: string; service_id: string; service_generation: number; grant_generation: number; revoked: number; }
interface OAuthClaim { user_id: string; service_id: string; service_generation: number; grant_generation: number; connection_id: string; connection_generation: number; }

export class CommandCenter {
  readonly config: HubConfig;
  readonly store: CommandStore;
  auth: CommandAuth;
  private starts: number[] = [];
  private ownerWrites = new Map<string, number[]>();

  private constructor(config: HubConfig) {
    this.config = validateConfig(config);
    this.store = new CommandStore(openStore(config.stateDir));
    this.auth = createAuth(this.config, this.store, false);
  }
  static async open(config: HubConfig): Promise<CommandCenter> {
    const app = new CommandCenter(config);
    try {
      await migrateAuth(app.auth);
      app.auth = createAuth(app.config, app.store);
      const context = await app.auth.$context;
      await context.checkSchema?.();
      return app;
    }
    catch (error) { app.store.db.close(); throw error; }
  }
  close(): void { this.store.db.close(); }

  async handle(input: Request): Promise<Response> {
    try {
      const request = await boundedRequest(input);
      const response = await this.route(request);
      response.headers.set("cache-control", "no-store");
      response.headers.set("x-content-type-options", "nosniff");
      response.headers.set("referrer-policy", "no-referrer");
      response.headers.set("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' https://avatars.githubusercontent.com; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
      return response;
    } catch (error) {
      const status = error instanceof HttpError ? error.status : 500;
      return json({ error: status < 500 ? (error as Error).message : "Command center request failed" }, status);
    }
  }

  private async session(request: Request) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    if (!session) throw new HttpError(401, "Sign in with GitHub first");
    return session;
  }
  private sameOrigin(request: Request): void {
    if (request.headers.get("origin") !== this.config.baseURL || !request.headers.get("content-type")?.startsWith("application/json")) {
      throw new HttpError(403, "Same-origin JSON request required");
    }
  }
  private throttleOwner(userId: string): void {
    const cutoff = Date.now() - 60_000;
    const attempts = (this.ownerWrites.get(userId) ?? []).filter((value) => value > cutoff);
    if (attempts.length >= 30) throw new HttpError(429, "Try again later");
    attempts.push(Date.now()); this.ownerWrites.set(userId, attempts);
    if (this.ownerWrites.size > 4096) for (const [id, rows] of this.ownerWrites) if (!rows.some((value) => value > cutoff)) this.ownerWrites.delete(id);
  }
  private publisher(request: Request, host: string): LinkedHost {
    const match = /^Bearer (wph_[A-Za-z0-9_-]{43})$/.exec(request.headers.get("authorization") ?? "");
    const record = this.store.host(host);
    if (!match || !record || record.revoked || record.publisher_hash !== digest(match[1])) throw new HttpError(403, "Publisher not authorized");
    return record;
  }
  private ownedService(id: unknown, userId: string): Service {
    const service = this.store.service(required(id, 64));
    if (!service || service.owner_id !== userId || this.store.host(service.host)?.revoked) throw new HttpError(403, "Service access denied");
    return service;
  }

  private async route(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;
    if (request.method === "GET" && path === "/healthz") return json({ ok: true, service: "wispnet-command-center", login: "github" });
    if (request.method === "GET" && ["/.well-known/oauth-authorization-server", "/.well-known/oauth-authorization-server/api/auth"].includes(path)) {
      return oauthProviderAuthServerMetadata(this.auth)(request);
    }
    if (request.method === "GET" && path === "/.well-known/openid-configuration") return oauthProviderOpenIdConfigMetadata(this.auth)(request);
    if (path.startsWith("/api/auth/")) {
      // Only our wrappers can create scoped keys. Browser-controlled plugin metadata must never establish permissions.
      if (path.startsWith("/api/auth/api-key/") || path.startsWith("/api/auth/admin/") || path === "/api/auth/token") throw new HttpError(404, "Not found");
      const revocation = path === "/api/auth/oauth2/revoke" && request.method === "POST" ? await this.revocationCandidate(request) : undefined;
      const response = await this.auth.handler(request);
      if (revocation) {
        const details = response.status === 400 ? await response.clone().json().catch(() => null) : undefined;
        if (response.ok || (revocation.jwt && details?.error === "unsupported_token_type")) {
          for (const id of revocation.connections) this.revokeConnection(id);
          // Better Auth cannot invalidate self-contained JWTs. Our durable connection generation can.
          if (!response.ok) return json({});
        }
      }
      return response;
    }
    if (request.method === "GET" && path === "/assets/command.js") return new Response(script, { headers: { "content-type": "text/javascript; charset=utf-8" } });
    if (request.method === "GET" && path === "/assets/command.css") return new Response(stylesheet, { headers: { "content-type": "text/css; charset=utf-8" } });
    if (request.method === "GET" && ["/", "/sign-in", "/connect", "/consent"].includes(path)) return new Response(page(path), { headers: { "content-type": "text/html; charset=utf-8" } });
    if (request.method === "POST" && path === "/api/command/links/start") return this.startLink(await objectBody(request));
    if (request.method === "POST" && path === "/api/command/links/poll") return this.pollLink(await objectBody(request));
    if (request.method === "POST" && path === "/api/command/host/register") return this.register(request, await objectBody(request));
    if (request.method === "POST" && path === "/api/command/host/unlink") {
      const body = await objectBody(request); const host = hex(body.host); this.publisher(request, host); this.store.unlink(host); return json({ ok: true });
    }
    if (request.method === "POST" && path === "/api/command/authorize") return this.authorize(request, await objectBody(request));
    if (!path.startsWith("/api/command/")) throw new HttpError(404, "Not found");
    const session = await this.session(request);
    const userId = session.user.id;
    if (request.method === "GET" && path === "/api/command/dashboard") {
      const services = this.store.db.prepare(`SELECT s.*,h.name AS host_name FROM command_services s JOIN command_hosts h ON h.host=s.host
        WHERE h.revoked=0 AND (s.owner_id=? OR EXISTS(SELECT 1 FROM command_grants g WHERE g.service_id=s.id AND g.user_id=? AND g.active=1)) ORDER BY s.name,s.service`).all(userId, userId);
      const hosts = this.store.db.prepare("SELECT host,name,revoked FROM command_hosts WHERE owner_id=?").all(userId);
      const keys = this.store.db.prepare("SELECT id,service_id,name,prefix,expires,revoked FROM command_keys WHERE user_id=? ORDER BY expires DESC").all(userId);
      const connections = this.store.db.prepare("SELECT id,service_id,client_id,generation,active FROM command_oauth_connections WHERE user_id=?").all(userId);
      const grants = this.store.db.prepare(`SELECT g.service_id,g.user_id,g.active,u.name,u.email FROM command_grants g JOIN user u ON u.id=g.user_id
        JOIN command_services s ON s.id=g.service_id WHERE s.owner_id=?`).all(userId);
      return json({ user: safeUser(session.user), services, hosts, keys, connections, grants });
    }
    if (request.method === "GET" && path === "/api/command/link") {
      const code = required(url.searchParams.get("code"), 32).replace(/-/g, "").toUpperCase();
      const link = this.store.db.prepare("SELECT host,name,expires,owner_id FROM command_links WHERE user_code=? AND consumed=0 AND expires>?").get(code, now());
      if (!link) throw new HttpError(404, "Link code expired or invalid");
      return json(link);
    }
    if (request.method !== "POST") throw new HttpError(405, "Method not allowed");
    this.sameOrigin(request); this.throttleOwner(userId);
    const body = await objectBody(request);
    if (path === "/api/command/links/approve") {
      const code = required(body.user_code, 32).replace(/-/g, "").toUpperCase();
      const link = this.store.db.prepare("SELECT * FROM command_links WHERE user_code=? AND expires>? AND consumed=0").get(code, now()) as PendingLink | undefined;
      if (!link) throw new HttpError(404, "Link code expired or invalid");
      const existing = this.store.host(link.host);
      if ((existing && !existing.revoked && existing.owner_id !== userId) || (link.owner_id && link.owner_id !== userId)) throw new HttpError(403, "Host already linked");
      this.store.db.prepare("UPDATE command_links SET owner_id=? WHERE id=?").run(userId, link.id);
      return json({ ok: true, host: link.host, name: link.name });
    }
    if (path === "/api/command/hosts/unlink") {
      const host = hex(body.host); if (this.store.host(host)?.owner_id !== userId) throw new HttpError(403, "Host access denied");
      this.store.unlink(host); return json({ ok: true });
    }
    if (path === "/api/command/grants") {
      const service = this.ownedService(body.service_id, userId);
      const user = this.store.db.prepare("SELECT id FROM user WHERE id=?").get(required(body.user_id, 128)) as { id: string } | undefined;
      if (!user || typeof body.active !== "boolean" || user.id === service.owner_id) throw new HttpError(400, "Select a registered user's account ID");
      this.store.setGrant(service, user.id, body.active); return json({ ok: true });
    }
    if (path === "/api/command/services/mode") {
      const service = this.ownedService(body.service_id, userId);
      if (typeof body.auth_mode !== "string" || !["oauth", "bearer", "both"].includes(body.auth_mode)) throw new HttpError(400, "Invalid public authentication mode");
      if (service.auth_mode !== body.auth_mode) {
        this.store.db.prepare("UPDATE command_services SET auth_mode=?,generation=generation+1 WHERE id=?").run(body.auth_mode, service.id);
        this.store.db.prepare("UPDATE command_keys SET revoked=1 WHERE service_id=?").run(service.id);
        this.store.db.prepare("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id=?").run(service.id);
      }
      return json({ ok: true });
    }
    if (path === "/api/command/keys/create") {
      const service = this.store.service(required(body.service_id, 64));
      const generation = service && this.store.grant(service, userId);
      if (!service || generation === undefined || service.auth_mode === "oauth") throw new HttpError(403, "Service access denied");
      const name = required(body.name, 80);
      const expiresIn = Number(body.expires_in ?? 60 * 60 * 24 * 30);
      if (!Number.isSafeInteger(expiresIn) || expiresIn < 3600 || expiresIn > 60 * 60 * 24 * 365) throw new HttpError(400, "Choose an expiry between one hour and one year");
      const key = await this.auth.api.createApiKey({ body: { userId, name, prefix: "wpk_", expiresIn,
        permissions: { service: [service.id] } } });
      this.store.db.prepare("INSERT INTO command_keys(id,user_id,service_id,service_generation,grant_generation,name,prefix,expires) VALUES(?,?,?,?,?,?,?,?)")
        .run(key.id, userId, service.id, service.generation, generation, name, key.key.slice(0, 10), now() + expiresIn);
      return json({ id: key.id, key: key.key, service_id: service.id, expires: now() + expiresIn, note: "Save this key now. It is shown only once." });
    }
    if (path === "/api/command/keys/revoke") {
      const key = this.store.db.prepare("SELECT * FROM command_keys WHERE id=? AND user_id=?").get(required(body.key_id, 128), userId) as StoredKey | undefined;
      if (!key) throw new HttpError(404, "Key not found");
      if (key.revoked) return json({ ok: true });
      this.store.db.prepare("UPDATE command_keys SET revoked=1 WHERE id=?").run(key.id);
      await this.auth.api.deleteApiKey({ body: { keyId: key.id }, headers: request.headers });
      return json({ ok: true });
    }
    if (path === "/api/command/connections/revoke") {
      const connection = this.store.db.prepare("SELECT * FROM command_oauth_connections WHERE id=? AND user_id=?").get(required(body.connection_id, 128), userId) as { id: string; client_id: string } | undefined;
      if (!connection) throw new HttpError(404, "Connection not found");
      this.revokeConnection(connection.id);
      return json({ ok: true });
    }
    throw new HttpError(404, "Not found");
  }

  private revokeConnection(id: string): void {
    const row = this.store.db.prepare(`SELECT c.user_id,c.client_id,s.resource FROM command_oauth_connections c
      JOIN command_services s ON s.id=c.service_id WHERE c.id=?`).get(id) as { user_id: string; client_id: string; resource: string } | undefined;
    if (!row) return;
    this.store.db.transaction(() => {
      this.store.db.prepare("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE id=?").run(id);
      this.store.db.prepare("UPDATE oauthRefreshToken SET revoked=? WHERE userId=? AND clientId=? AND (resources IS NULL OR resources LIKE ?)")
        .run(Date.now(), row.user_id, row.client_id, `%${row.resource}%`);
    })();
  }

  private async revocationCandidate(request: Request): Promise<{ connections: string[]; jwt: boolean } | undefined> {
    try {
      const clone = request.clone();
      const body = request.headers.get("content-type")?.startsWith("application/json") ? await clone.json() : Object.fromEntries(new URLSearchParams(await clone.text()));
      let clientId = body.client_id;
      const basic = /^Basic ([A-Za-z0-9+/=]+)$/.exec(request.headers.get("authorization") ?? "");
      if (basic) clientId = Buffer.from(basic[1], "base64").toString().split(":")[0];
      if (typeof clientId !== "string" || typeof body.token !== "string") return;
      const token = body.token.replace(/^Bearer /, "");
      const refresh = this.store.db.prepare("SELECT userId,clientId,resources FROM oauthRefreshToken WHERE token=?").get(digest(token)) as { userId: string; clientId: string; resources: string } | undefined;
      if (refresh?.clientId === clientId) {
        const resources = JSON.parse(refresh.resources ?? "[]") as string[];
        const rows = this.store.db.prepare(`SELECT c.id,s.resource FROM command_oauth_connections c JOIN command_services s ON s.id=c.service_id
          WHERE c.user_id=? AND c.client_id=?`).all(refresh.userId, clientId) as { id: string; resource: string }[];
        return { jwt: false, connections: rows.filter((row) => resources.includes(row.resource)).map((row) => row.id) };
      }
      const { payload } = await jwtVerify(token, createLocalJWKSet(await this.auth.api.getJwks()), { issuer: this.config.baseURL, algorithms: ["EdDSA", "ES256", "RS256", "PS256", "ES512"] });
      const claim = payload.wispnet as OAuthClaim | undefined;
      if (payload.client_id !== clientId || !claim) return;
      const connection = this.store.db.prepare("SELECT id FROM command_oauth_connections WHERE id=? AND client_id=? AND user_id=? AND service_id=?")
        .get(claim.connection_id, clientId, claim.user_id, claim.service_id) as { id: string } | undefined;
      if (connection) return { jwt: true, connections: [connection.id] };
    } catch { /* Invalid credentials remain the OAuth provider's responsibility. */ }
  }

  private startLink(body: Record<string, unknown>): Response {
    this.starts = this.starts.filter((value) => value > Date.now() - 60_000);
    if (this.starts.length >= 60) throw new HttpError(429, "Try linking later");
    this.starts.push(Date.now());
    const host = hex(body.host); const nonce = hex(body.nonce); const name = required(body.name, 80);
    const signature = required(body.signature, 128);
    if (!/^[a-f0-9]{128}$/.test(signature)) throw new HttpError(400, "Invalid host proof");
    const key = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(host, "hex")]), type: "spki", format: "der" });
    const bytes = `wispnet-command-link-v1\n${this.config.baseURL}\n${host}\n${name}\n${nonce}`;
    if (!verify(null, Buffer.from(bytes), key, Buffer.from(signature, "hex"))) throw new HttpError(403, "Invalid host proof");
    if (this.store.db.prepare("SELECT 1 FROM command_links WHERE nonce=?").get(nonce)) throw new HttpError(409, "Link challenge already used");
    const active = this.store.db.prepare("SELECT COUNT(*) AS n FROM command_links WHERE host=? AND expires>? AND consumed=0").get(host, now()) as { n: number };
    if (active.n >= 5) throw new HttpError(429, "Finish an existing link request first");
    const deviceCode = secretToken("wpd_"); const userCode = randomBytes(6).toString("hex").toUpperCase();
    this.store.db.prepare("INSERT INTO command_links(id,host,name,nonce,device_hash,user_code,expires) VALUES(?,?,?,?,?,?,?)")
      .run(randomUUID(), host, name, nonce, digest(deviceCode), userCode, now() + 600);
    const shownCode = userCode.match(/.{4}/g)!.join("-");
    return json({ device_code: deviceCode, user_code: shownCode, verification_url: `${this.config.baseURL}/connect?code=${shownCode}`, expires_in: 600, interval: 5 });
  }
  private pollLink(body: Record<string, unknown>): Response {
    const deviceCode = required(body.device_code, 128);
    return this.store.db.transaction(() => {
      const link = this.store.db.prepare("SELECT * FROM command_links WHERE device_hash=? AND expires>? AND consumed=0").get(digest(deviceCode), now()) as PendingLink | undefined;
      if (!link) throw new HttpError(404, "Link request expired or completed");
      if (!link.owner_id) return json({ state: "pending" });
      const existing = this.store.host(link.host);
      if (existing && !existing.revoked && existing.owner_id !== link.owner_id) throw new HttpError(403, "Host already linked");
      const account = this.store.db.prepare("SELECT id,name FROM user WHERE id=?").get(link.owner_id);
      if (!account) throw new HttpError(403, "Account no longer available");
      const token = secretToken("wph_");
      this.store.db.prepare(`INSERT INTO command_hosts(host,owner_id,name,publisher_hash,revoked) VALUES(?,?,?,?,0)
        ON CONFLICT(host) DO UPDATE SET owner_id=excluded.owner_id,name=excluded.name,publisher_hash=excluded.publisher_hash,revoked=0`)
        .run(link.host, link.owner_id, link.name, digest(token));
      this.store.db.prepare("UPDATE command_links SET consumed=1 WHERE id=?").run(link.id);
      return json({ state: "linked", publisher_token: token, account });
    })();
  }
  private async register(request: Request, body: Record<string, unknown>): Promise<Response> {
    const hostId = hex(body.host); const host = this.publisher(request, hostId);
    const id = serviceId(body.service); const incarnation = hex(body.incarnation); const name = required(body.name, 80);
    const resource = required(body.resource);
    const uri = new URL(resource);
    if (uri.protocol !== "https:" || uri.username || uri.password || uri.port || uri.hash || uri.search || uri.hostname !== publishedHostname(name, hostId, id, this.config.publicDomain) || uri.href !== resource) throw new HttpError(400, "Invalid public resource URL");
    const record = this.store.register(host, { service: id, incarnation, name, resource });
    await ensureOAuthResource(this.auth, resource, `${name} / ${id}`);
    return json({ ok: true, resource, resource_id: record.id, auth_mode: record.auth_mode });
  }
  private async authorize(request: Request, body: Record<string, unknown>): Promise<Response> {
    const host = hex(body.host); this.publisher(request, host);
    const service = this.store.resource(required(body.resource));
    if (!service || service.host !== host || service.service !== serviceId(body.service) || service.incarnation !== hex(body.incarnation)) throw new HttpError(403, "Service access denied");
    const token = required(body.token, 16384);
    let userId: string;
    let grantGeneration: number;
    if (token.startsWith("wpk_")) {
      if (service.auth_mode === "oauth") throw new HttpError(403, "Service access denied");
      const verified = await this.auth.api.verifyApiKey({ body: { key: token, permissions: { service: [service.id] } } });
      if (!verified.valid || !verified.key) throw new HttpError(403, "Service access denied");
      const record = this.store.db.prepare("SELECT * FROM command_keys WHERE id=?").get(verified.key.id) as StoredKey | undefined;
      if (!record || record.revoked || record.service_id !== service.id || record.service_generation !== service.generation || verified.key.referenceId !== record.user_id) throw new HttpError(403, "Service access denied");
      userId = record.user_id; grantGeneration = record.grant_generation;
    } else {
      if (service.auth_mode === "bearer") throw new HttpError(403, "Service access denied");
      try {
        const jwks = await this.auth.api.getJwks();
        const { payload } = await jwtVerify(token, createLocalJWKSet(jwks), { issuer: this.config.baseURL, audience: service.resource, algorithms: ["EdDSA", "ES256", "RS256", "PS256", "ES512"] });
        const claim = payload.wispnet as OAuthClaim | undefined;
        if (!claim || payload.aud !== service.resource || !payload.exp || !payload.client_id || !String(payload.scope ?? "").split(" ").includes("mcp:use") || claim.service_id !== service.id || claim.service_generation !== service.generation) throw new Error("Invalid claim");
        const connection = this.store.db.prepare("SELECT generation,active FROM command_oauth_connections WHERE id=? AND user_id=? AND service_id=? AND client_id=?")
          .get(claim.connection_id, claim.user_id, service.id, String(payload.client_id)) as { generation: number; active: number } | undefined;
        if (!connection?.active || connection.generation !== claim.connection_generation) throw new Error("Revoked connection");
        userId = claim.user_id; grantGeneration = claim.grant_generation;
      } catch { throw new HttpError(403, "Service access denied"); }
    }
    if (this.store.grant(service, userId) !== grantGeneration || !this.store.db.prepare("SELECT 1 FROM user WHERE id=?").get(userId)) throw new HttpError(403, "Service access denied");
    return json({ allowed: true, subject: userId, lease_seconds: 5 });
  }
}

async function boundedRequest(request: Request): Promise<Request> {
  if (!request.body) return request;
  const reader = request.body.getReader(); const chunks: Uint8Array[] = []; let length = 0;
  try {
    for (;;) {
      const next = await reader.read(); if (next.done) break;
      length += next.value.byteLength;
      if (length > 65536) { await reader.cancel(); throw new HttpError(413, "Request too large"); }
      chunks.push(next.value);
    }
  } finally { reader.releaseLock(); }
  return new Request(request.url, { method: request.method, headers: request.headers, body: Buffer.concat(chunks) });
}
