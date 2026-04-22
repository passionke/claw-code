/**
 * Table/view metadata from information_schema + SHOW COLUMNS (Doris).
 * Author: kejiqing
 */

import mysql from "mysql";
import type { DorisConnection } from "./connection.js";

export interface TableMeta {
  tableSchema: string;
  tableName: string;
  tableType: string;
  engine: string;
  tableComment: string;
  tableRows: string | number | null;
  createTime: string | null;
  updateTime: string | null;
}

export interface ColumnMeta {
  field: string;
  type: string;
  null: string;
  key: string;
  default: string | null;
  extra: string;
  comment?: string;
}

export async function getTableMeta(
  conn: DorisConnection,
  database: string,
  table: string
): Promise<TableMeta | null> {
  const sql = `SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, ENGINE, TABLE_COMMENT,
     TABLE_ROWS, CREATE_TIME, UPDATE_TIME
     FROM information_schema.tables
     WHERE TABLE_SCHEMA = ${mysql.escape(database)} AND TABLE_NAME = ${mysql.escape(table)}`;
  const [rows] = await conn.query(sql);
  const r = Array.isArray(rows) ? rows[0] : null;
  if (!r || typeof r !== "object") return null;
  const o = r as Record<string, unknown>;
  const tableRows = o.TABLE_ROWS;
  return {
    tableSchema: String(o.TABLE_SCHEMA ?? ""),
    tableName: String(o.TABLE_NAME ?? ""),
    tableType: String(o.TABLE_TYPE ?? ""),
    engine: String(o.ENGINE ?? ""),
    tableComment: String(o.TABLE_COMMENT ?? ""),
    tableRows:
      tableRows === null || tableRows === undefined
        ? null
        : typeof tableRows === "number" || typeof tableRows === "string"
          ? tableRows
          : null,
    createTime: o.CREATE_TIME != null ? String(o.CREATE_TIME) : null,
    updateTime: o.UPDATE_TIME != null ? String(o.UPDATE_TIME) : null,
  };
}

function escapeId(id: string): string {
  return "`" + String(id).replace(/`/g, "``") + "`";
}

export async function getColumnMeta(
  conn: DorisConnection,
  database: string,
  table: string
): Promise<ColumnMeta[]> {
  const dbId = escapeId(database);
  const tblId = escapeId(table);
  const [rows] = await conn.query(`SHOW FULL COLUMNS FROM ${dbId}.${tblId}`);
  if (!Array.isArray(rows)) return [];
  return rows.map((r) => {
    const o = (r as Record<string, unknown>) ?? {};
    return {
      field: String(o.Field ?? ""),
      type: String(o.Type ?? ""),
      null: String(o.Null ?? ""),
      key: String(o.Key ?? ""),
      default: o.Default != null ? String(o.Default) : null,
      extra: String(o.Extra ?? ""),
      comment: o.Comment != null ? String(o.Comment) : undefined,
    };
  });
}

export function buildTableInformationText(
  table: TableMeta | null,
  columns: ColumnMeta[]
): string {
  const lines: string[] = [];
  if (table) {
    lines.push(`表/视图: ${table.tableSchema}.${table.tableName}`);
    lines.push(`类型: ${table.tableType}`);
    lines.push(`引擎: ${table.engine}`);
    if (table.tableComment) lines.push(`注释: ${table.tableComment}`);
    if (table.tableRows != null) lines.push(`行数(估): ${table.tableRows}`);
    if (table.createTime) lines.push(`创建时间: ${table.createTime}`);
    if (table.updateTime) lines.push(`更新时间: ${table.updateTime}`);
    lines.push("");
  }
  if (columns.length > 0) {
    lines.push("列信息:");
    const colLines = columns.map(
      (c) =>
        `  ${c.field}  ${c.type}  ${c.null}  ${c.key}  default=${c.default ?? "NULL"}  ${c.extra}  ${c.comment ?? ""}`
    );
    lines.push(...colLines);
  }
  return lines.join("\n");
}
