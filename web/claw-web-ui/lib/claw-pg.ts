/** PostgreSQL pool for Claw Web UI (server-only). Author: kejiqing */

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Pool, type PoolClient } from "pg";

let pool: Pool | null = null;
let migrated = false;
let migratePromise: Promise<void> | null = null;

/** Session advisory lock (two int4 keys — works on all PG versions). */
const LOCK_KEY1 = 0x434c; // "CL"
const LOCK_KEY2 = 0x4157; // "AW"

const MIGRATIONS_TABLE = "claw_schema_migrations";

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

function sqlMigrationsDir(): string {
  const candidates = [join(process.cwd(), "sql"), join(process.cwd(), "web/claw-web-ui/sql")];
  for (const dir of candidates) {
    if (existsSync(dir)) return dir;
  }
  throw new Error(`sql migrations directory not found (tried: ${candidates.join(", ")})`);
}

function listMigrationFiles(): string[] {
  const dir = sqlMigrationsDir();
  return readdirSync(dir)
    .filter((f) => f.endsWith(".sql"))
    .sort();
}

function migrationChecksum(sql: string): string {
  return createHash("sha256").update(sql).digest("hex");
}

async function appliedMigrations(client: PoolClient): Promise<Map<string, string>> {
  const map = new Map<string, string>();
  const res = await client.query<{ name: string; checksum: string }>(
    `SELECT name, checksum FROM ${MIGRATIONS_TABLE}`,
  );
  for (const row of res.rows) {
    map.set(row.name, row.checksum);
  }
  return map;
}

async function schemaMarkedCurrent(client: PoolClient): Promise<boolean> {
  const files = listMigrationFiles();
  if (files.length === 0) return true;

  const reg = await client.query<{ reg: string | null }>(
    `SELECT to_regclass($1) AS reg`,
    [`public.${MIGRATIONS_TABLE}`],
  );
  if (!reg.rows[0]?.reg) return false;

  const applied = await appliedMigrations(client);
  for (const file of files) {
    const sql = readFileSync(join(sqlMigrationsDir(), file), "utf8");
    if (applied.get(file) !== migrationChecksum(sql)) return false;
  }
  return true;
}

async function runMigrations(client: PoolClient): Promise<void> {
  const dir = sqlMigrationsDir();
  const files = listMigrationFiles();

  await client.query("SELECT pg_advisory_lock($1, $2)", [LOCK_KEY1, LOCK_KEY2]);
  try {
    await client.query(`
      CREATE TABLE IF NOT EXISTS ${MIGRATIONS_TABLE} (
        name TEXT PRIMARY KEY,
        checksum TEXT NOT NULL,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
      )
    `);

    const applied = await appliedMigrations(client);

    for (const file of files) {
      const sql = readFileSync(join(dir, file), "utf8");
      const checksum = migrationChecksum(sql);
      if (applied.get(file) === checksum) continue;

      await client.query("BEGIN");
      try {
        await client.query(sql);
        await client.query(
          `INSERT INTO ${MIGRATIONS_TABLE} (name, checksum)
           VALUES ($1, $2)
           ON CONFLICT (name) DO UPDATE SET checksum = EXCLUDED.checksum, applied_at = now()`,
          [file, checksum],
        );
        await client.query("COMMIT");
      } catch (e) {
        await client.query("ROLLBACK");
        throw e;
      }
    }
  } finally {
    await client.query("SELECT pg_advisory_unlock($1, $2)", [LOCK_KEY1, LOCK_KEY2]);
  }
}

async function runEnsureMigrationsPipeline(): Promise<void> {
  const p = getPool();
  const probe = await p.connect();
  try {
    if (await schemaMarkedCurrent(probe)) {
      migrated = true;
      return;
    }
  } catch {
    /* first boot or partial schema — run migrations below */
  } finally {
    probe.release();
  }

  const client = await p.connect();
  try {
    await runMigrations(client);
    migrated = true;
  } finally {
    client.release();
  }
}

async function ensureMigrated(): Promise<void> {
  if (migrated) return;
  if (!migratePromise) {
    migratePromise = runEnsureMigrationsPipeline().catch((err) => {
      migratePromise = null;
      throw err;
    });
  }
  await migratePromise;
}

export async function withPg<T>(fn: (client: PoolClient) => Promise<T>): Promise<T> {
  await ensureMigrated();
  const p = getPool();
  const client = await p.connect();
  try {
    return await fn(client);
  } finally {
    client.release();
  }
}
