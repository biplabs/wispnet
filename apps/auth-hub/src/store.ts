import Database from "better-sqlite3";
import { chmodSync, lstatSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createHash, randomBytes, randomUUID } from "node:crypto";

export const digest = (value: string): string => createHash("sha256").update(value).digest("hex");
export const secretToken = (prefix: string): string => prefix + randomBytes(32).toString("base64url");
export const now = (): number => Math.floor(Date.now() / 1000);
export interface LinkedHost { host: string; owner_id: string; name: string; publisher_hash: string | null; revoked: number; }
export interface Service {
  id: string; host: string; owner_id: string; service: string; incarnation: string;
  name: string; resource: string; generation: number; auth_mode: string; enabled: number;
}
export interface Grant { service_id: string; user_id: string; generation: number; active: number; }

export function openStore(stateDir: string): Database.Database {
  mkdirSync(stateDir, { recursive: true, mode: 0o700 });
  const dir = lstatSync(stateDir);
  if (!dir.isDirectory() || dir.isSymbolicLink() || (dir.mode & 0o077) || dir.uid !== process.getuid?.()) {
    throw new Error("Command center state directory must be owned by this user and mode 700");
  }
  const path = join(stateDir, "command.sqlite");
  try {
    const file = lstatSync(path);
    if (!file.isFile() || file.isSymbolicLink() || file.nlink !== 1 || (file.mode & 0o077) || file.uid !== process.getuid?.()) {
      throw new Error("Command center database must be an owner-only regular file");
    }
  } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
  const oldMask = process.umask(0o077);
  let db: Database.Database;
  try { db = new Database(path); chmodSync(path, 0o600); } finally { process.umask(oldMask); }
  db.pragma("journal_mode = WAL");
  db.pragma("foreign_keys = ON");
  db.exec(`
    CREATE TABLE IF NOT EXISTS command_links (
      id TEXT PRIMARY KEY, host TEXT NOT NULL, name TEXT NOT NULL, nonce TEXT NOT NULL UNIQUE,
      device_hash TEXT NOT NULL UNIQUE, user_code TEXT NOT NULL UNIQUE, expires INTEGER NOT NULL,
      owner_id TEXT, consumed INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS command_hosts (
      host TEXT PRIMARY KEY, owner_id TEXT NOT NULL, name TEXT NOT NULL,
      publisher_hash TEXT UNIQUE, revoked INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS command_services (
      id TEXT PRIMARY KEY, host TEXT NOT NULL REFERENCES command_hosts(host), owner_id TEXT NOT NULL,
      service TEXT NOT NULL, incarnation TEXT NOT NULL, name TEXT NOT NULL, resource TEXT NOT NULL UNIQUE,
      generation INTEGER NOT NULL DEFAULT 1, auth_mode TEXT NOT NULL DEFAULT 'both', enabled INTEGER NOT NULL DEFAULT 1,
      UNIQUE(host, service)
    );
    CREATE TABLE IF NOT EXISTS command_grants (
      service_id TEXT NOT NULL REFERENCES command_services(id), user_id TEXT NOT NULL,
      generation INTEGER NOT NULL DEFAULT 1, active INTEGER NOT NULL DEFAULT 1,
      PRIMARY KEY(service_id, user_id)
    );
    CREATE TABLE IF NOT EXISTS command_keys (
      id TEXT PRIMARY KEY, user_id TEXT NOT NULL, service_id TEXT NOT NULL REFERENCES command_services(id),
      service_generation INTEGER NOT NULL, grant_generation INTEGER NOT NULL,
      name TEXT NOT NULL, prefix TEXT NOT NULL, expires INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS command_oauth_connections (
      id TEXT PRIMARY KEY, user_id TEXT NOT NULL, service_id TEXT NOT NULL REFERENCES command_services(id),
      client_id TEXT NOT NULL, generation INTEGER NOT NULL DEFAULT 1, active INTEGER NOT NULL DEFAULT 1,
      UNIQUE(user_id,service_id,client_id)
    );
  `);
  return db;
}

export class CommandStore {
  constructor(readonly db: Database.Database) {}
  host(host: string): LinkedHost | undefined { return this.db.prepare("SELECT * FROM command_hosts WHERE host=?").get(host) as LinkedHost | undefined; }
  service(id: string): Service | undefined { return this.db.prepare("SELECT * FROM command_services WHERE id=?").get(id) as Service | undefined; }
  resource(uri: string): Service | undefined { return this.db.prepare("SELECT * FROM command_services WHERE resource=?").get(uri) as Service | undefined; }
  grant(service: Service, userId: string): number | undefined {
    if (!service.enabled || this.host(service.host)?.revoked !== 0) return;
    if (service.owner_id === userId) return 0;
    const row = this.db.prepare("SELECT * FROM command_grants WHERE service_id=? AND user_id=?").get(service.id, userId) as Grant | undefined;
    return row?.active ? row.generation : undefined;
  }
  setGrant(service: Service, userId: string, active: boolean): void {
    this.db.transaction(() => {
      this.db.prepare(`INSERT INTO command_grants(service_id,user_id,active) VALUES(?,?,?)
        ON CONFLICT(service_id,user_id) DO UPDATE SET active=excluded.active,generation=generation+1`).run(service.id, userId, Number(active));
      this.db.prepare("UPDATE command_keys SET revoked=1 WHERE service_id=? AND user_id=?").run(service.id, userId);
      this.db.prepare("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id=? AND user_id=?").run(service.id, userId);
      // Revoke refresh credentials as well: regranting access must not resurrect an old client.
      // The resource-specific JWT claims still deny old access tokens immediately.
      this.db.prepare('UPDATE oauthRefreshToken SET revoked=? WHERE userId=? AND (resources IS NULL OR resources LIKE ?)').run(Date.now(), userId, `%${service.resource}%`);
    })();
  }
  register(host: LinkedHost, data: { service: string; incarnation: string; name: string; resource: string }): Service {
    return this.db.transaction(() => {
      const previous = this.db.prepare("SELECT * FROM command_services WHERE host=? AND service=?").get(host.host, data.service) as Service | undefined;
      if (previous) {
        const replaced = previous.incarnation !== data.incarnation || previous.resource !== data.resource || previous.owner_id !== host.owner_id;
        this.db.prepare(`UPDATE command_services SET owner_id=?,incarnation=?,name=?,resource=?,enabled=1,
          generation=generation+? WHERE id=?`).run(host.owner_id, data.incarnation, data.name, data.resource, Number(replaced), previous.id);
        if (replaced) {
          this.db.prepare("UPDATE command_grants SET active=0,generation=generation+1 WHERE service_id=?").run(previous.id);
          this.db.prepare("UPDATE command_keys SET revoked=1 WHERE service_id=?").run(previous.id);
          this.db.prepare("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id=?").run(previous.id);
        }
        return this.service(previous.id)!;
      }
      const id = randomUUID();
      this.db.prepare("INSERT INTO command_services(id,host,owner_id,service,incarnation,name,resource) VALUES(?,?,?,?,?,?,?)")
        .run(id, host.host, host.owner_id, data.service, data.incarnation, data.name, data.resource);
      return this.service(id)!;
    })();
  }
  unlink(host: string): void {
    this.db.transaction(() => {
      const resources = this.db.prepare("SELECT resource FROM command_services WHERE host=?").all(host) as { resource: string }[];
      this.db.prepare("UPDATE command_hosts SET revoked=1,publisher_hash=NULL WHERE host=?").run(host);
      this.db.prepare("UPDATE command_services SET enabled=0,generation=generation+1 WHERE host=?").run(host);
      this.db.prepare("UPDATE command_keys SET revoked=1 WHERE service_id IN (SELECT id FROM command_services WHERE host=?)").run(host);
      this.db.prepare("UPDATE command_oauth_connections SET active=0,generation=generation+1 WHERE service_id IN (SELECT id FROM command_services WHERE host=?)").run(host);
      for (const { resource } of resources) {
        this.db.prepare("UPDATE oauthRefreshToken SET revoked=? WHERE resources LIKE ?").run(Date.now(), `%${resource}%`);
      }
    })();
  }
}
