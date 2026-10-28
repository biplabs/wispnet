import { isAbsolute } from "node:path";

export interface HubConfig {
  baseURL: string;
  stateDir: string;
  secret: string;
  publicDomain: string;
  githubClientId?: string;
  githubClientSecret?: string;
  /** Only injected by local tests. Never read from a production environment variable. */
  testMode?: boolean;
}

export function validateConfig(input: HubConfig): HubConfig {
  const url = new URL(input.baseURL);
  if (url.username || url.password || url.search || url.hash || url.pathname !== "/") {
    throw new Error("Command center URL must be a plain HTTPS origin");
  }
  if (url.protocol !== "https:" && !(input.testMode && ["localhost", "127.0.0.1"].includes(url.hostname))) {
    throw new Error("Command center requires HTTPS");
  }
  if (!isAbsolute(input.stateDir) || input.secret.length < 32 || !/^[a-z0-9.-]+\.[a-z]{2,}$/.test(input.publicDomain)) {
    throw new Error("Invalid command center state, secret, or public domain configuration");
  }
  if (!input.testMode && (!input.githubClientId || !input.githubClientSecret)) {
    throw new Error("Configure WISPNET_GITHUB_CLIENT_ID and WISPNET_GITHUB_CLIENT_SECRET before starting GitHub registration");
  }
  return { ...input, baseURL: url.origin };
}

export function configFromEnv(): HubConfig {
  return validateConfig({
    baseURL: process.env.WISPNET_COMMAND_URL ?? "https://cmd.wispnet.app",
    stateDir: process.env.WISPNET_COMMAND_STATE ?? "",
    secret: process.env.WISPNET_COMMAND_SECRET ?? "",
    publicDomain: process.env.WISPNET_PUBLIC_DOMAIN ?? "wispnet.app",
    githubClientId: process.env.WISPNET_GITHUB_CLIENT_ID,
    githubClientSecret: process.env.WISPNET_GITHUB_CLIENT_SECRET,
  });
}
