/** PostgreSQL pool for Claw Web UI (server-only). Author: kejiqing */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Pool, type PoolClient } from "pg";

let pool: Pool | null = null;
let migrated = false;

export function databaseUrl(): string | null {
  const url = process.env.CLAW_WEB_DATABASE_URL ?? process.env.DATABASE_URL;
  return url && url.trim() ? url.trim() : null;
}

export function pgConfigured(): boolean {
  return databaseUrl() != null;
}

export function getPool(): Pool {
  const url = databaseUrl();
  if (!url) {
    throw new Error("CLAW_WEB_DATABASE_URL (or DATABASE_URL) is not set");
  }
  if (!pool) {
    pool = new Pool({ connectionString: url, max: 8 });
  }
  return pool;
}

async function runMigrations(client: PoolClient): Promise<void> {
  const dir = join(process.cwd(), "sql");
  const files = readdirSync(dir)
    .filter((f) => f.endsWith(".sql"))
    .sort();
  for (const file of files) {
    const sql = readFileSync(join(dir, file), "utf8");
    await client.query(sql);
  }
}

export async function withPg<T>(fn: (client: PoolClient) => Promise<T>): Promise<T> {
  const p = getPool();
  const client = await p.connect();
  try {
    if (!migrated) {
      await runMigrations(client);
      migrated = true;
    }
    return await fn(client);
  } finally {
    client.release();
  }
}
