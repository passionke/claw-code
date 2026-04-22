/**
 * Standard error messages for MCP responses.
 * Author: kejiqing
 */
export const RESTRICTION_MSG = "本服务仅允许 SELECT、SET、EXPLAIN、SHOW（可多条 SET 后跟一条 SELECT/EXPLAIN/SHOW，SET 不计数）；禁止 INSERT/UPDATE/DELETE/DROP/CREATE/ALTER 等写操作与 DDL。";
export const DISCLAIMER_MSG = "本次为 SQL 报错，具体错误信息仅供参考，可能无法反映真实原因，请勿过度依赖。";
export const RESTRICTION_DISCLAIMER = `${RESTRICTION_MSG}\n${DISCLAIMER_MSG}`;
export function wrapSqlError(category, rawMessage) {
    let out = `${RESTRICTION_DISCLAIMER}\n\n校验未通过：${category}`;
    if (rawMessage && rawMessage.trim()) {
        out += `\n\n（附带信息，参考作用有限：${rawMessage.trim()}）`;
    }
    return out;
}
export function wrapExecutionError(_dorisMessage) {
    return `Doris SQL 执行错误。

Doris 语法比较严谨，请严格遵循标准语法书写。除此之外需要查清楚原表的字段，看是否是字段引用错误，包括一些类型错误。如果是复杂的子查询，先验证好子查询单项 OK，然后再一层一层直到最后一层，同样也要注意字段引用。

常见问题排查建议：
• 字段名、表名：是否与 information_schema / SHOW 结果一致，有无大小写、空格、保留字未加反引号。
• 类型：数值/字符串/日期是否混用，隐式转换是否被支持；字符串是否用单引号。
• 聚合：SELECT 中非聚合列是否都在 GROUP BY 中；聚合与标量子查询混用时的作用域。
• 子查询：内层别名在外层是否正确引用；多层级时每层字段来源是否明确。
• 别名：同一层级内别名是否在后续表达式中正确使用，避免重复定义或未定义。
• 分区/分桶：WHERE 中尽量带上分区列以利用剪枝；某些语法对分桶表有要求。
• NULL：比较与运算注意 NULL 语义，必要时用 COALESCE/IFNULL 或 IS NULL。`;
}
export const ONLY_READONLY_MCP = "本服务为只读 Doris 查询 MCP，仅支持 SELECT、SET、EXPLAIN、SHOW；具体错误信息仅供参考，参考作用有限。";
const CONNECTION_ERROR_PATTERN = /closed state|Can't add new command when connection|Connection lost|fatal error|Cannot enqueue|offset out of range/i;
export function isConnectionError(err) {
    const msg = err instanceof Error ? err.message : String(err);
    return CONNECTION_ERROR_PATTERN.test(msg);
}
//# sourceMappingURL=errors.js.map