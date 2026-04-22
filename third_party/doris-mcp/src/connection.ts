/**
 * Doris connection helper.
 * Author: kejiqing
 */

import mysql from "mysql";
import type { ClusterConfig } from "./configLoader.js";

export interface DorisConnection {
  query(sql: string): Promise<[unknown[], { name?: string }[]]>;
  end(): Promise<void>;
}

function wrapConnection(raw: mysql.Connection): DorisConnection {
  return {
    query(sql: string): Promise<[unknown[], { name?: string }[]]> {
      return new Promise((resolve, reject) => {
        raw.query(sql, (err?: Error, results?: unknown, fields?: unknown) => {
          if (err) reject(err);
          else {
            let rows: unknown[] = [];
            let outFields: { name?: string }[] = [];
            if (results != null && typeof results === "object" && !Array.isArray(results)) {
              rows = [results];
              outFields = Object.keys(results as object).map((name) => ({ name }));
            } else if (Array.isArray(results) && results.length > 0) {
              const first = results[0];
              if (results.length === 1) {
                rows = Array.isArray(first) ? (first as unknown[]) : (results as unknown[]);
              } else {
                for (let i = results.length - 1; i >= 0; i--) {
                  if (Array.isArray(results[i])) {
                    rows = results[i] as unknown[];
                    break;
                  }
                }
                if (rows.length === 0 && results.length > 0 && typeof first === "object" && first !== null)
                  rows = results as unknown[];
              }
              if (rows.length > 0 && typeof rows[0] === "object" && rows[0] !== null) {
                outFields = Object.keys(rows[0] as object).map((name) => ({ name }));
              } else if (Array.isArray(fields) && fields.length > 0) {
                const lastFields = fields.length > 1 && Array.isArray(fields[fields.length - 1])
                  ? (fields[fields.length - 1] as { name?: string }[])
                  : (fields as { name?: string }[]);
                outFields = lastFields.map((f) => ({ name: f?.name ?? "" }));
              }
            }
            resolve([rows, outFields]);
          }
        });
      });
    },
    end(): Promise<void> {
      return new Promise((resolve, reject) => {
        raw.end((err?: Error) => (err ? reject(err) : resolve()));
      });
    },
  };
}

export async function getConnection(
  _clusterId: string,
  config: ClusterConfig,
  database?: string
): Promise<DorisConnection> {
  const useDb =
    database === ""
      ? undefined
      : (database ?? config.default_database ?? undefined);
  const raw = mysql.createConnection({
    host: config.host,
    port: config.port,
    user: config.user,
    password: config.password,
    database: useDb,
    charset: "utf8",
    connectTimeout: 15000,
    multipleStatements: true,
    ...(config.ssl ? { ssl: { rejectUnauthorized: false } } : {}),
  });
  await new Promise<void>((resolve, reject) => {
    raw.connect((err?: Error) => (err ? reject(err) : resolve()));
  });
  return wrapConnection(raw);
}

export function evictConnection(_clusterId: string, _database: string): void {
  // no-op
}

export function touchConnection(_clusterId: string, _database: string): void {
  // no-op
}

export function releaseConnection(
  _clusterId: string,
  _database: string,
  _conn: DorisConnection
): void {
  // no-op
}
