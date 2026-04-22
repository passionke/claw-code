#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Validate Doris SQL: single statement only, SELECT / SET / EXPLAIN / SHOW.
Uses sqlglot (MySQL dialect). Outputs JSON to stdout.
Also returns referenced physical tables for optional allowlist enforcement.
Author: kejiqing
"""
from __future__ import annotations

import json
import re
import sys


def _normalize_identifier(value: str) -> str:
    return str(value or "").strip().strip("`").strip('"').strip().lower()


def _collect_table_refs(statements, exp_module) -> list[str]:
    """
    Collect referenced physical tables as lower-case "db.table" or "table".
    Excludes CTE aliases.
    """
    refs: set[str] = set()
    cte_names: set[str] = set()

    for stmt in statements:
        for cte in stmt.find_all(exp_module.CTE):
            alias = _normalize_identifier(getattr(cte, "alias_or_name", "") or "")
            if alias:
                cte_names.add(alias)

    for stmt in statements:
        for table in stmt.find_all(exp_module.Table):
            table_name = _normalize_identifier(getattr(table, "name", "") or "")
            db_name = _normalize_identifier(getattr(table, "db", "") or "")
            if not table_name:
                continue
            if not db_name and table_name in cte_names:
                continue
            refs.add(f"{db_name}.{table_name}" if db_name else table_name)

    return sorted(refs)


def main() -> None:
    sql = sys.stdin.read()
    if not sql or not sql.strip():
        out = {"valid": False, "reason": "SQL_EMPTY"}
        print(json.dumps(out, ensure_ascii=False))
        return
    try:
        import sqlglot
        from sqlglot import exp
    except ImportError:
        out = {"valid": False, "reason": "SQLGLOT_NOT_INSTALLED", "detail": "pip install sqlglot"}
        print(json.dumps(out, ensure_ascii=False))
        return

    sql_for_parse = re.sub(
        r"\bEXPLAIN\s+VERBOSE\s+",
        "EXPLAIN ",
        sql.strip(),
        flags=re.IGNORECASE,
    )
    try:
        statements = sqlglot.parse(sql_for_parse, dialect="mysql")
    except Exception as e:
        out = {
            "valid": False,
            "reason": "PARSE_ERROR",
            "detail": str(e)[:500],
        }
        print(json.dumps(out, ensure_ascii=False))
        return

    if not statements:
        out = {
            "valid": False,
            "reason": "MULTIPLE_STATEMENTS",
            "detail": "解析结果为空。",
        }
        print(json.dumps(out, ensure_ascii=False))
        return

    allowed_last = ("Select", "Set", "Explain", "Describe", "Show")
    for i, stmt in enumerate(statements):
        if stmt is None:
            out = {"valid": False, "reason": "PARSE_EMPTY", "detail": "解析得到空语句。"}
            print(json.dumps(out, ensure_ascii=False))
            return
        stmt_type = type(stmt).__name__
        display_type = "Explain" if stmt_type == "Describe" else stmt_type
        is_last = i == len(statements) - 1
        if is_last:
            if stmt_type not in allowed_last:
                out = {
                    "valid": False,
                    "reason": "DISALLOWED_TYPE",
                    "detail": f"仅允许 SELECT、SET、EXPLAIN、SHOW，最后一条语句类型为：{display_type}。",
                    "stmt_type": display_type,
                }
                print(json.dumps(out, ensure_ascii=False))
                return
        else:
            if stmt_type != "Set":
                out = {
                    "valid": False,
                    "reason": "MULTIPLE_STATEMENTS",
                    "detail": "多条语句时仅允许前面为 SET，最后一条为 SELECT、EXPLAIN 或 SHOW。",
                }
                print(json.dumps(out, ensure_ascii=False))
                return

    table_refs = _collect_table_refs(statements, exp)
    print(json.dumps({"valid": True, "table_refs": table_refs}, ensure_ascii=False))


if __name__ == "__main__":
    main()
