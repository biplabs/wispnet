import { betterAuth } from "better-auth";
import { jwt } from "better-auth/plugins";
import { APIError } from "better-auth/api";
import { getMigrations } from "better-auth/db/migration";
import { oauthProvider } from "@better-auth/oauth-provider";
import { cimd } from "@better-auth/cimd";
import { fetchClientMetadataResource } from "@better-auth/cimd/node";
import { apiKey } from "@better-auth/api-key";
import { randomUUID } from "node:crypto";
import type { HubConfig } from "./config.js";
import { CommandStore, digest } from "./store.js";

export const SCOPES = ["openid", "profile", "offline_access", "mcp:use"];
interface Connection { id: string; generation: number; active: number; }

export function createAuth(config: HubConfig, store: CommandStore, validateSchema = true) {
  return betterAuth({
    appName: "Wispnet Command Center",
    baseURL: config.baseURL,
    basePath: "/api/auth",
    secret: config.secret,
    database: store.db,
    trustedOrigins: [config.baseURL],
    socialProviders: config.githubClientId && config.githubClientSecret ? {
      github: { clientId: config.githubClientId, clientSecret: config.githubClientSecret },
    } : {},
    // Accounts are created by the GitHub callback. There are no password or test login endpoints.
    emailAndPassword: { enabled: false },
    account: { accountLinking: { enabled: false } },
    session: { expiresIn: 60 * 60 * 24 * 7, updateAge: 60 * 60 * 24 },
    rateLimit: { enabled: !config.testMode, window: 60, max: 60, storage: "database" },
    advanced: { useSecureCookies: !config.testMode, disableCSRFCheck: false, disableOriginCheck: false,
      database: { validateSchema } },
    logger: { level: "error", log: (level) => {
      // The auth library can include credential-bearing context in diagnostics. Keep logs structural.
      if (level === "error" && !config.testMode) process.stderr.write("Command center authentication error\n");
    } },
    plugins: [
      jwt({ jwt: { issuer: config.baseURL }, disableSettingJwtHeader: true }),
      oauthProvider({
        loginPage: "/sign-in",
        consentPage: "/consent",
        scopes: SCOPES,
        resources: [],
        // A client may request a registered service; only an owner-approved user grant can authorize it.
        // This supports resources added after a client registered, without treating client registration as access.
        enforcePerClientResources: false,
        grantTypes: ["authorization_code", "refresh_token"],
        accessTokenExpiresIn: 300,
        refreshTokenExpiresIn: 60 * 60 * 24 * 30,
        refreshTokenReuseInterval: 0,
        storeTokens: { hash: async (token) => digest(token) },
        allowDynamicClientRegistration: true,
        allowUnauthenticatedClientRegistration: true,
        clientRegistrationDefaultApplicationType: "infer",
        clientPrivileges: () => false,
        resourcePrivileges: () => false,
        extensions: [{ claims: { accessToken: async ({ user, client, resources, scopes, grantType }) => {
          if (!user || resources?.length !== 1 || !scopes.includes("mcp:use")) {
            throw new APIError("FORBIDDEN", { message: "A single permitted MCP service is required" });
          }
          const service = store.resource(resources[0]);
          const generation = service && store.grant(service, user.id);
          if (!service || generation === undefined || service.auth_mode === "bearer") {
            throw new APIError("FORBIDDEN", { message: "Service access denied" });
          }
          let connection = store.db.prepare("SELECT * FROM command_oauth_connections WHERE user_id=? AND service_id=? AND client_id=?")
            .get(user.id, service.id, client.clientId) as Connection | undefined;
          if (grantType === "refresh_token" && !connection?.active) {
            throw new APIError("FORBIDDEN", { message: "Connect this client again" });
          }
          if (!connection) {
            const id = randomUUID();
            store.db.prepare("INSERT INTO command_oauth_connections(id,user_id,service_id,client_id) VALUES(?,?,?,?)")
              .run(id, user.id, service.id, client.clientId);
            connection = { id, generation: 1, active: 1 };
          } else if (!connection.active) {
            store.db.prepare("UPDATE command_oauth_connections SET active=1,generation=generation+1 WHERE id=?").run(connection.id);
            connection = { ...connection, active: 1, generation: connection.generation + 1 };
          }
          return { wispnet: { user_id: user.id, service_id: service.id, service_generation: service.generation,
            grant_generation: generation, connection_id: connection.id, connection_generation: connection.generation } };
        } } }],
      }),
      cimd({ fetchClientMetadataResource, metadataProfile: "mcp-2026-07-28" }),
      apiKey({
        defaultPrefix: "wpk_",
        enableMetadata: false,
        disableKeyHashing: false,
        enableSessionForAPIKeys: false,
        keyExpiration: { defaultExpiresIn: 60 * 60 * 24 * 30, minExpiresIn: 1 / 24, maxExpiresIn: 365 },
        rateLimit: { enabled: true, timeWindow: 60_000, maxRequests: 1200 },
      }),
    ],
  });
}

export type CommandAuth = ReturnType<typeof createAuth>;
export async function migrateAuth(auth: CommandAuth): Promise<void> {
  const migrations = await getMigrations(auth.options);
  await migrations.runMigrations();
}

export async function ensureOAuthResource(auth: CommandAuth, identifier: string, name: string): Promise<void> {
  const { adapter } = await auth.$context;
  // This is trusted server-side provisioning after the host's publisher credential was checked.
  // Public resource CRUD is disabled; the pinned Better Auth adapter owns type conversion and IDs.
  const existing = await adapter.findOne({ model: "oauthResource", where: [{ field: "identifier", value: identifier }] });
  if (!existing) await adapter.create({ model: "oauthResource", data: {
    identifier, name, allowedScopes: SCOPES, accessTokenTtl: 300, refreshTokenTtl: 60 * 60 * 24 * 30,
    disabled: false, policyVersion: 1, dpopBoundAccessTokensRequired: false, createdAt: new Date(), updatedAt: new Date(),
  } });
}
