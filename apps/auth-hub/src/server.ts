import { createServer, type Server } from "node:http";
import { toNodeHandler } from "better-auth/node";
import { pathToFileURL } from "node:url";
import { CommandCenter } from "./app.js";
import { configFromEnv } from "./config.js";

export function createCommandServer(app: CommandCenter): Server {
  const handler = toNodeHandler((request) => app.handle(request));
  return createServer({ maxHeaderSize: 32768, headersTimeout: 10_000, requestTimeout: 30_000 }, (request, response) => {
    const authorizationHeaders = request.rawHeaders.filter((_, index) => index % 2 === 0 && request.rawHeaders[index].toLowerCase() === "authorization").length;
    if (authorizationHeaders > 1) { response.writeHead(400); response.end("Invalid request"); return; }
    void handler(request, response).catch(() => {
      if (!response.headersSent) response.writeHead(500, { "content-type": "application/json" });
      response.end('{"error":"Command center request failed"}');
    });
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const app = await CommandCenter.open(configFromEnv());
    const server = createCommandServer(app);
    const port = Number(process.env.WISPNET_COMMAND_PORT ?? "8325");
    if (!Number.isSafeInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid command center port");
    server.listen(port, "127.0.0.1", () => process.stdout.write(`Wispnet command center listening on loopback port ${port}\n`));
    const stop = () => server.close(() => { app.close(); process.exit(0); });
    process.once("SIGINT", stop); process.once("SIGTERM", stop);
  } catch (error) {
    process.stderr.write((error instanceof Error ? error.message : "Command center startup failed") + "\n");
    process.exitCode = 1;
  }
}
