/**
 * Doris query-only MCP server. Tools: list_clusters, list_databases, list_tables,
 * table_information, query (SELECT/SET only). Author: kejiqing
 */
import { z } from "zod";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { loadConfig, listClusterIds } from "./configLoader.js";
import mysql from "mysql";
import { getConnection, evictConnection } from "./connection.js";
import { validateDorisSql } from "./sqlValidator.js";
import { formatResult } from "./formatter.js";
import { getTableMeta, getColumnMeta, buildTableInformationText } from "./tableInfo.js";
import { wrapExecutionError, ONLY_READONLY_MCP, isConnectionError } from "./errors.js";
function getConfig() {
    return loadConfig();
}
function escapeSqlStringLiteral(input) {
    return input
        .replace(/\\/g, "\\\\")
        .replace(/\r/g, "\\r")
        .replace(/\n/g, "\\n")
        .replace(/'/g, "\\'");
}
function toSqlLiteral(value) {
    if (value == null)
        return "NULL";
    if (typeof value === "number" && Number.isFinite(value))
        return String(value);
    if (typeof value === "boolean")
        return value ? "1" : "0";
    return `'${escapeSqlStringLiteral(String(value))}'`;
}
function buildEnvSetPrefix(env) {
    if (!env)
        return "";
    const entries = Object.entries(env);
    if (entries.length === 0)
        return "";
    const sets = entries.map(([rawKey, v]) => {
        const key = rawKey.replace(/^@+/, "");
        return `SET @${key} = ${toSqlLiteral(v)}`;
    });
    return sets.join("; ") + ";";
}
function parseEnvSummary(envSummary) {
    if (!envSummary)
        return undefined;
    const trimmed = envSummary.trim();
    if (!trimmed || trimmed === "(none)")
        return undefined;
    const out = {};
    const parts = trimmed.split(";").map((s) => s.trim()).filter(Boolean);
    for (const p of parts) {
        const idx = p.indexOf("=");
        if (idx <= 0)
            continue;
        const key = p.slice(0, idx).trim().replace(/^@+/, "");
        const value = p.slice(idx + 1).trim();
        if (!key)
            continue;
        out[key] = value;
    }
    return Object.keys(out).length > 0 ? out : undefined;
}
async function withRetryOnConnectionError(fn) {
    try {
        return await fn();
    }
    catch (err) {
        if (isConnectionError(err)) {
            return await fn();
        }
        throw err;
    }
}
function normalizeRef(value) {
    return String(value || "")
        .trim()
        .replace(/[`"]/g, "")
        .replace(/\s+/g, "")
        .toLowerCase();
}
function isAllowedTableRef(ref, allowedSet, database) {
    const normalized = normalizeRef(ref);
    if (!normalized)
        return false;
    const dbNorm = normalizeRef(database);
    const dot = normalized.indexOf(".");
    const tableOnly = dot >= 0 ? normalized.slice(dot + 1) : normalized;
    if (allowedSet.has(normalized))
        return true;
    if (allowedSet.has(tableOnly))
        return true;
    if (dot < 0 && dbNorm && allowedSet.has(`${dbNorm}.${normalized}`))
        return true;
    return false;
}
const server = new McpServer({ name: "doris-query-mcp", version: "0.1.0" }, {});
server.tool("doris_list_clusters", "List configured Doris cluster IDs.", async () => {
    try {
        const config = getConfig();
        const ids = listClusterIds(config);
        const text = ids.length ? ids.join("\n") : "No clusters configured.";
        return { content: [{ type: "text", text }] };
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `配置加载失败：${msg}` }] };
    }
});
server.tool("doris_list_databases", "List databases in a Doris cluster.", { cluster: z.string().describe("Cluster ID from doris_list_clusters") }, async ({ cluster }) => {
    let config;
    try {
        config = getConfig();
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `配置加载失败：${msg}` }] };
    }
    const cfg = config.clusters[cluster];
    if (!cfg) {
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：集群不存在 "${cluster}"。` }] };
    }
    try {
        return await withRetryOnConnectionError(async () => {
            let conn;
            try {
                conn = await getConnection(cluster, cfg, undefined);
                const [rows] = await conn.query("SHOW DATABASES");
                const list = Array.isArray(rows) ? rows : [];
                const names = list.map((r) => r.Database ?? "").filter(Boolean);
                return { content: [{ type: "text", text: names.length ? names.join("\n") : "No databases." }] };
            }
            finally {
                if (conn)
                    await conn.end().catch(() => { });
            }
        });
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (isConnectionError(err))
            evictConnection(cluster, "");
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：${msg}` }] };
    }
});
server.tool("doris_list_tables", "List tables and views in a database.", { cluster: z.string(), database: z.string() }, async ({ cluster, database }) => {
    let config;
    try {
        config = getConfig();
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `配置加载失败：${msg}` }] };
    }
    const cfg = config.clusters[cluster];
    if (!cfg) {
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：集群不存在 "${cluster}"。` }] };
    }
    try {
        return await withRetryOnConnectionError(async () => {
            let conn;
            try {
                conn = await getConnection(cluster, cfg, database);
                const sql = `SELECT TABLE_NAME FROM information_schema.tables WHERE TABLE_SCHEMA = ${mysql.escape(database)} ORDER BY TABLE_NAME`;
                const [rows] = await conn.query(sql);
                const list = Array.isArray(rows) ? rows : [];
                const names = list.map((r) => r.TABLE_NAME ?? "").filter(Boolean);
                return { content: [{ type: "text", text: names.length ? names.join("\n") : "No tables." }] };
            }
            finally {
                if (conn)
                    await conn.end().catch(() => { });
            }
        });
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (isConnectionError(err))
            evictConnection(cluster, database);
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：${msg}` }] };
    }
});
server.tool("doris_table_information", "Get table/view metadata (columns, type, comment) from information_schema + SHOW COLUMNS.", { cluster: z.string(), database: z.string(), table: z.string() }, async ({ cluster, database, table }) => {
    let config;
    try {
        config = getConfig();
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `配置加载失败：${msg}` }] };
    }
    const cfg = config.clusters[cluster];
    if (!cfg) {
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：集群不存在 "${cluster}"。` }] };
    }
    try {
        return await withRetryOnConnectionError(async () => {
            let conn;
            try {
                conn = await getConnection(cluster, cfg, database);
                const tableMeta = await getTableMeta(conn, database, table);
                const columns = await getColumnMeta(conn, database, table);
                return { content: [{ type: "text", text: buildTableInformationText(tableMeta, columns) || "No metadata." }] };
            }
            finally {
                if (conn)
                    await conn.end().catch(() => { });
            }
        });
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (isConnectionError(err))
            evictConnection(cluster, database);
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：${msg}` }] };
    }
});
const TRUNCATE_ALLOWED_CLUSTER = "dev";
server.tool("doris_truncate_table", "Truncate a table (clear all rows). Only allowed on cluster 'dev'.", { cluster: z.string(), database: z.string(), table: z.string() }, async ({ cluster, database, table }) => {
    if (cluster !== TRUNCATE_ALLOWED_CLUSTER) {
        return { content: [{ type: "text", text: `此工具仅对 dev 集群开放，当前集群 "${cluster}" 不允许执行 TRUNCATE。` }] };
    }
    let config;
    try {
        config = getConfig();
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `配置加载失败：${msg}` }] };
    }
    const cfg = config.clusters[cluster];
    if (!cfg)
        return { content: [{ type: "text", text: `集群 "${cluster}" 不存在。` }] };
    const safeTable = "`" + String(table).replace(/`/g, "``") + "`";
    const sql = `TRUNCATE TABLE ${safeTable}`;
    try {
        return await withRetryOnConnectionError(async () => {
            let conn;
            try {
                conn = await getConnection(cluster, cfg, database);
                await conn.query(sql);
                return { content: [{ type: "text", text: `已清空表 ${database}.${table}。` }] };
            }
            finally {
                if (conn)
                    await conn.end().catch(() => { });
            }
        });
    }
    catch (err) {
        if (isConnectionError(err))
            evictConnection(cluster, database);
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `TRUNCATE 失败：${msg}` }] };
    }
});
server.tool("doris_query", "Execute Doris SQL. On cluster 'dev': any SQL allowed (subject to user permissions). On other clusters: read-only only.", {
    cluster: z.string(),
    database: z.string(),
    env: z.string().optional(),
    sql: z.string(),
}, async ({ cluster, database, env, sql }) => {
    const isDev = cluster === "dev";
    let config;
    try {
        config = getConfig();
    }
    catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return { content: [{ type: "text", text: `配置加载失败：${msg}` }] };
    }
    const cfg = config.clusters[cluster];
    if (!cfg) {
        return { content: [{ type: "text", text: `${ONLY_READONLY_MCP}\n失败原因：集群不存在 "${cluster}"。` }] };
    }
    try {
        return await withRetryOnConnectionError(async () => {
            let conn;
            try {
                const envMap = parseEnvSummary(env) ?? cfg.env;
                const envPrefix = buildEnvSetPrefix(envMap);
                const userSql = sql.trim().replace(/;+\s*$/, "");
                const fullSql = envPrefix ? `${envPrefix}\n${userSql}` : userSql;
                const validation = validateDorisSql(fullSql);
                if (!isDev && !validation.ok) {
                    return { content: [{ type: "text", text: validation.message }] };
                }
                const allowedTables = (cfg.allowed_tables || [])
                    .map((x) => normalizeRef(String(x)))
                    .filter(Boolean);
                if (allowedTables.length > 0) {
                    if (!validation.ok) {
                        return {
                            content: [{
                                    type: "text",
                                    text: `${ONLY_READONLY_MCP}\n失败原因：表白名单已开启，但 SQL 引用对象解析失败：${validation.message}`,
                                }],
                        };
                    }
                    const allowedSet = new Set(allowedTables);
                    const refs = validation.tableRefs || [];
                    const disallowed = refs.filter((ref) => !isAllowedTableRef(ref, allowedSet, database));
                    if (disallowed.length > 0) {
                        return {
                            content: [{
                                    type: "text",
                                    text: `${ONLY_READONLY_MCP}\n失败原因：SQL 引用了不在 allowed_tables 白名单内的对象。\n` +
                                        `引用对象: ${refs.join(", ") || "(none)"}\n` +
                                        `越界对象: ${disallowed.join(", ")}\n` +
                                        `白名单: ${Array.from(allowedSet).join(", ")}`,
                                }],
                        };
                    }
                }
                conn = await getConnection(cluster, cfg, database);
                const start = performance.now();
                const [rows, fields] = await conn.query(fullSql);
                const elapsedMs = performance.now() - start;
                const rowList = Array.isArray(rows) ? rows : [];
                const fieldList = Array.isArray(fields) ? fields : [];
                let columns = [];
                if (rowList.length > 0) {
                    const firstWithKeys = rowList.find((r) => Object.keys(r).length > 0);
                    columns = firstWithKeys ? Object.keys(firstWithKeys) : fieldList.map((f) => f.name ?? "");
                }
                else if (fieldList.length > 0) {
                    columns = fieldList.map((f) => f.name ?? "");
                }
                const typedRows = rowList;
                const resultText = formatResult(columns, typedRows, elapsedMs);
                return { content: [{ type: "text", text: resultText }] };
            }
            finally {
                if (conn)
                    await conn.end().catch(() => { });
            }
        });
    }
    catch (err) {
        if (isConnectionError(err))
            evictConnection(cluster, database);
        return { content: [{ type: "text", text: wrapExecutionError() }] };
    }
});
async function main() {
    const transport = new StdioServerTransport();
    await server.connect(transport);
}
main().catch((err) => {
    console.error("Doris MCP error:", err);
    process.exit(1);
});
//# sourceMappingURL=index.js.map