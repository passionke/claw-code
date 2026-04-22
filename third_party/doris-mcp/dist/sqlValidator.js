/**
 * Strict validation for Doris SQL: single statement only, SELECT / SET / EXPLAIN / SHOW.
 * Delegates parsing to Python sqlglot (scripts/validate_sql.py). Author: kejiqing
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { wrapSqlError } from "./errors.js";
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT_PATH = path.join(__dirname, "..", "scripts", "validate_sql.py");
function runPythonValidator(sql) {
    const result = spawnSync("python3", [SCRIPT_PATH], {
        input: sql,
        encoding: "utf-8",
        maxBuffer: 1024 * 1024,
        timeout: 10000,
    });
    const stdout = result.stdout?.trim() ?? "";
    const stderr = result.stderr?.trim() ?? "";
    if (result.status !== 0 || !stdout) {
        return {
            valid: false,
            reason: "VALIDATOR_ERROR",
            detail: stderr || result.error?.message || "Python 校验脚本执行失败。请确保已安装：pip install sqlglot",
        };
    }
    try {
        return JSON.parse(stdout);
    }
    catch {
        return { valid: false, reason: "VALIDATOR_ERROR", detail: "校验脚本输出非 JSON。" };
    }
}
export function validateDorisSql(sql) {
    const trimmed = sql.trim();
    if (!trimmed) {
        return { ok: false, message: wrapSqlError("SQL 为空。") };
    }
    const out = runPythonValidator(trimmed);
    if (out.valid) {
        const refs = Array.isArray(out.table_refs)
            ? out.table_refs.map((x) => String(x).trim().toLowerCase()).filter(Boolean)
            : [];
        return { ok: true, tableRefs: refs };
    }
    const reason = out.reason ?? "UNKNOWN";
    const detail = out.detail ?? "";
    switch (reason) {
        case "SQL_EMPTY":
            return { ok: false, message: wrapSqlError("SQL 为空。") };
        case "SQLGLOT_NOT_INSTALLED":
            return {
                ok: false,
                message: wrapSqlError("未安装 Python sqlglot。请执行：pip install sqlglot", detail),
            };
        case "MULTIPLE_STATEMENTS":
            return {
                ok: false,
                message: wrapSqlError("多条语句时仅允许前面为 SET，最后一条为 SELECT 或 EXPLAIN。", detail),
            };
        case "PARSE_ERROR":
            return {
                ok: false,
                message: wrapSqlError("SQL 解析失败，请检查是否为合法 Doris/MySQL 语法。本服务仅支持单条 SELECT、SET、EXPLAIN 或 SHOW。", detail),
            };
        case "PARSE_EMPTY":
        case "DISALLOWED_TYPE":
            return {
                ok: false,
                message: wrapSqlError(detail || `语句类型不允许：${out.stmt_type ?? "未知"}。`),
            };
        default:
            return { ok: false, message: wrapSqlError(detail || reason) };
    }
}
//# sourceMappingURL=sqlValidator.js.map