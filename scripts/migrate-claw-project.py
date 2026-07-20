#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Cross-gateway project migrate: effective project_config (HTTP) + conversation rows (PG).

Author: kejiqing

Migrates one projId from a source claw-code gateway to a destination gateway.
Default scope is ``all`` (config + sessions), matching the verified 171→172 flow.

Write operations require explicit ``--apply``. Without it (or with ``--dry-run``), the
script only inventories and validates exports.

``cc_messages.message_id`` is a global serial. When the destination already has the same
id under another project, the script heals by inserting missing ``(turn_id, seq)`` rows
with new serial ids (logical transcript key is preserved).

Database URLs: pass ``--src-database-url`` / ``--dst-database-url``, or set
``CLAW_MIGRATE_SRC_DATABASE_URL`` / ``CLAW_MIGRATE_DST_DATABASE_URL``. Passwords are
never hardcoded.

Depends on: Python 3 stdlib + local ``psql`` on PATH.

Example (preview, project 10 — do not apply until confirmed)::

  python3 scripts/migrate-claw-project.py \\
    --proj-id 10 \\
    --src-gateway http://10.200.2.171:18088 \\
    --dst-gateway http://10.200.2.172:18088 \\
    --src-database-url \"$CLAW_MIGRATE_SRC_DATABASE_URL\" \\
    --dst-database-url \"$CLAW_MIGRATE_DST_DATABASE_URL\" \\
    --scope all \\
    --dry-run

Example (apply after confirmation)::

  python3 scripts/migrate-claw-project.py ... --scope all --apply --verify-http

What this does NOT migrate: disk workspace/jsonl, project_config revision history,
global settings / LLM / admin tokens. Does not delete extra rows on destination.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from typing import Any

# FK insert order for conversation tables. Author: kejiqing
SESSION_TABLES = [
    "gateway_sessions",
    "gateway_turns",
    "gateway_runtime_iterations",
    "cc_messages",
    "gateway_feedback",
    "gateway_conversation_translate",
    "gateway_session_artifacts",
]

CONFIG_COMPARE_FIELDS = (
    "claudeMd",
    "mcpServersJson",
    "rulesJson",
    "skillsJson",
    "skillsSourcesJson",
    "allowedToolsJson",
    "extraSessionFieldsJson",
    "solvePreflightJson",
    "solveOrchestrationJson",
    "languagePipelineJson",
    "promptLimitsJson",
    "gitSyncJson",
)

SESSION_VERIFY_KEYS = {
    "gateway_sessions": ("session_id", "ds_id"),
    "gateway_turns": ("turn_id",),
    "gateway_runtime_iterations": ("iteration_id",),
    # message_id is a gateway-global serial and can legitimately be remapped.
    "cc_messages": ("turn_id", "seq"),
    "gateway_feedback": ("session_id", "ds_id", "turn_id"),
    "gateway_conversation_translate": ("session_id", "ds_id"),
    "gateway_session_artifacts": ("artifact_id",),
}


def die(msg: str, code: int = 1) -> None:
    print(f"error: {msg}", file=sys.stderr)
    raise SystemExit(code)


def log(msg: str) -> None:
    print(msg, flush=True)


def http(
    method: str,
    base: str,
    path: str,
    body: Any | None = None,
    timeout: float = 120,
) -> tuple[int, Any]:
    url = base.rstrip("/") + path
    data = None if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8")
    headers = {"Accept": "application/json"}
    if data is not None:
        headers["Content-Type"] = "application/json; charset=utf-8"
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            if not raw:
                return resp.status, None
            return resp.status, json.loads(raw.decode("utf-8"))
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            payload: Any = json.loads(raw.decode("utf-8"))
        except Exception:
            payload = raw.decode("utf-8", errors="replace")
        return e.code, payload


def require_psql() -> None:
    try:
        subprocess.check_call(
            ["psql", "--version"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.CalledProcessError):
        die("psql not found on PATH")


def psql_text(url: str, sql: str) -> str:
    return subprocess.check_output(
        ["psql", url, "-v", "ON_ERROR_STOP=1", "-At", "-c", sql],
        text=True,
        encoding="utf-8",
    )


def psql_run(url: str, sql: str) -> None:
    subprocess.check_call(
        ["psql", url, "-v", "ON_ERROR_STOP=1", "-c", sql],
        stdout=subprocess.DEVNULL,
    )


def psql_file(url: str, path: str) -> None:
    subprocess.check_call(
        ["psql", url, "-v", "ON_ERROR_STOP=1", "-f", path],
        stdout=subprocess.DEVNULL,
    )


def psql_json_rows(url: str, sql: str) -> list[dict[str, Any]]:
    wrap = f"SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json) FROM ({sql}) t"
    out = psql_text(url, wrap).strip() or "[]"
    rows = json.loads(out)
    if not isinstance(rows, list):
        die(f"unexpected json_agg type: {type(rows)}")
    return rows


def table_columns(url: str, table: str) -> list[str]:
    sql = f"""
    SELECT column_name
    FROM information_schema.columns
    WHERE table_schema = 'public' AND table_name = '{table}'
    ORDER BY ordinal_position
    """
    return [c for c in psql_text(url, sql).splitlines() if c]


def select_session_sql(table: str, common_cols: list[str], proj_id: int) -> str:
    col_list = ", ".join(common_cols)
    if table == "gateway_runtime_iterations":
        return f"""
        SELECT {col_list}
        FROM gateway_runtime_iterations
        WHERE turn_id IN (
          SELECT turn_id FROM gateway_turns
          WHERE COALESCE(proj_id, ds_id) = {proj_id}
        )
        """
    return f"""
    SELECT {col_list}
    FROM {table}
    WHERE COALESCE(proj_id, ds_id) = {proj_id}
    """


def count_session_rows(url: str, table: str, proj_id: int) -> int:
    if table == "gateway_runtime_iterations":
        sql = f"""
        SELECT count(*)::text FROM gateway_runtime_iterations
        WHERE turn_id IN (
          SELECT turn_id FROM gateway_turns
          WHERE COALESCE(proj_id, ds_id) = {proj_id}
        )
        """
    else:
        sql = f"""
        SELECT count(*)::text FROM {table}
        WHERE COALESCE(proj_id, ds_id) = {proj_id}
        """
    return int(psql_text(url, sql).strip() or "0")


def sql_literal(v: Any) -> str:
    if v is None:
        return "NULL"
    if isinstance(v, bool):
        return "TRUE" if v else "FALSE"
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return str(v)
    if isinstance(v, (dict, list)):
        s = json.dumps(v, ensure_ascii=False).replace("'", "''")
        return f"'{s}'::jsonb"
    s = str(v).replace("'", "''")
    return f"'{s}'"


def worker_value(cfg: dict[str, Any]) -> Any:
    if "workerProfileJson" in cfg and cfg.get("workerProfileJson") is not None:
        return cfg.get("workerProfileJson")
    return cfg.get("workerIsolationJson")


def build_config_put_body(src: dict[str, Any]) -> dict[str, Any]:
    put: dict[str, Any] = {
        "rulesJson": src.get("rulesJson") or [],
        "mcpServersJson": src.get("mcpServersJson") or {},
        "skillsJson": src.get("skillsJson") or [],
        "allowedToolsJson": src.get("allowedToolsJson") or [],
        "claudeMd": src.get("claudeMd") or "",
        "gitSyncJson": src.get("gitSyncJson"),
        "solvePreflightJson": src.get("solvePreflightJson"),
        "solveOrchestrationJson": src.get("solveOrchestrationJson"),
        "languagePipelineJson": src.get("languagePipelineJson"),
        "extraSessionFieldsJson": src.get("extraSessionFieldsJson"),
        "promptLimitsJson": src.get("promptLimitsJson")
        if src.get("promptLimitsJson") is not None
        else {},
        "skillsSourcesJson": src.get("skillsSourcesJson")
        if src.get("skillsSourcesJson") is not None
        else {},
    }
    worker = worker_value(src)
    if worker is not None:
        put["workerProfileJson"] = worker
    return put


def summarize_config(cfg: dict[str, Any] | None) -> str:
    if not cfg:
        return "(missing)"
    mcp = cfg.get("mcpServersJson") or {}
    mcp_keys = list(mcp.keys()) if isinstance(mcp, dict) else []
    claude = cfg.get("claudeMd") or ""
    first = claude.splitlines()[0] if claude else ""
    return (
        f"contentRev={cfg.get('contentRev')} draftOpen={cfg.get('draftOpen')} "
        f"mcp={mcp_keys} claudeChars={len(claude)} title={first!r} "
        f"worker={worker_value(cfg)}"
    )


def compare_configs(src: dict[str, Any], dst: dict[str, Any]) -> list[str]:
    expected = build_config_put_body(src)
    diffs: list[str] = []
    for field in CONFIG_COMPARE_FIELDS:
        if expected.get(field) != dst.get(field):
            diffs.append(field)
    if expected.get("workerProfileJson") != worker_value(dst):
        diffs.append("workerProfile/Isolation")
    return diffs


def row_key(row: dict[str, Any], columns: tuple[str, ...]) -> tuple[str, ...]:
    """Build a stable logical key from JSON-exported DB values. Author: kejiqing"""
    return tuple(str(row.get(column)) for column in columns)


def migrate_config(
    *,
    src_gateway: str,
    dst_gateway: str,
    proj_id: int,
    apply: bool,
) -> None:
    log(f"[config] GET src /v1/project/config/{proj_id}")
    code, src = http("GET", src_gateway, f"/v1/project/config/{proj_id}")
    if code != 200 or not isinstance(src, dict):
        die(f"source config GET failed: HTTP {code} {src}")
    log(f"[config] source: {summarize_config(src)}")

    code, dst_list = http("GET", dst_gateway, "/v1/projects")
    if code != 200 or not isinstance(dst_list, dict):
        die(f"destination project list failed: HTTP {code} {dst_list}")
    dst_ids = {p.get("projId") for p in (dst_list.get("projects") or [])}
    dst_has = proj_id in dst_ids

    code, dst_cfg = http("GET", dst_gateway, f"/v1/project/config/{proj_id}")
    if code == 200 and isinstance(dst_cfg, dict):
        log(f"[config] destination existing: {summarize_config(dst_cfg)}")
    else:
        log(f"[config] destination config missing (HTTP {code})")

    put = build_config_put_body(src)
    log(
        f"[config] put payload: mcp={list((put.get('mcpServersJson') or {}).keys())} "
        f"claudeChars={len(put.get('claudeMd') or '')} "
        f"create_project={not dst_has}"
    )

    if not apply:
        log("[config] dry-run: skip create/put/commit/activate")
        return

    if not dst_has:
        log(f"[config] POST /v1/projects projId={proj_id}")
        code, created = http(
            "POST", dst_gateway, "/v1/projects", {"projId": proj_id}
        )
        if code not in (200, 201):
            die(f"create project failed: HTTP {code} {created}")
        log(f"[config] created: {created}")

    log(f"[config] PUT draft /v1/project/config/{proj_id}")
    code, put_resp = http("PUT", dst_gateway, f"/v1/project/config/{proj_id}", put)
    if code != 200:
        die(f"put draft failed: HTTP {code} {put_resp}")

    note = (
        f"import from {src_gateway} proj {proj_id} "
        f"contentRev={src.get('contentRev')} by kejiqing"
    )
    log(f"[config] COMMIT draft note={note!r}")
    code, commit = http(
        "POST",
        dst_gateway,
        f"/v1/project/config/{proj_id}/versions/commit",
        {"note": note},
    )
    if code != 200 or not isinstance(commit, dict):
        die(f"commit failed: HTTP {code} {commit}")
    saved = commit.get("savedContentRev")
    if not saved:
        die(f"commit missing savedContentRev: {commit}")

    log(f"[config] ACTIVATE {saved}")
    code, act = http(
        "POST",
        dst_gateway,
        f"/v1/project/config/{proj_id}/versions/{saved}/activate",
        {},
    )
    if code != 200:
        die(f"activate failed: HTTP {code} {act}")

    code, dst_after = http("GET", dst_gateway, f"/v1/project/config/{proj_id}")
    if code != 200 or not isinstance(dst_after, dict):
        die(f"post-activate GET failed: HTTP {code} {dst_after}")
    diffs = compare_configs(src, dst_after)
    if diffs:
        die(f"config field mismatch after migrate: {diffs}")
    log(f"[config] verified OK contentRev={dst_after.get('contentRev')}")


def migrate_sessions(
    *,
    src_db: str,
    dst_db: str,
    proj_id: int,
    cluster_id: str,
    apply: bool,
    work_dir: str,
) -> None:
    require_psql()
    log("[sessions] inventory")
    for table in SESSION_TABLES:
        src_n = count_session_rows(src_db, table, proj_id)
        dst_n = count_session_rows(dst_db, table, proj_id)
        log(f"  {table}: src={src_n} dst={dst_n}")

    os.makedirs(work_dir, exist_ok=True)
    exported: dict[str, int] = {}
    # Verify against this export snapshot (source may keep receiving writes). Author: kejiqing
    exported_keys: dict[str, set[tuple[str, ...]]] = {}
    # Logical message key is (turn_id, seq); message_id is a global serial and may collide. Author: kejiqing
    exported_turn_seq: set[str] = set()
    exported_cc_rows: list[dict[str, Any]] = []

    for table in SESSION_TABLES:
        src_cols = table_columns(src_db, table)
        dst_cols = table_columns(dst_db, table)
        common = [c for c in dst_cols if c in src_cols]
        if not common:
            die(f"{table}: no common columns between src and dst")
        rows = psql_json_rows(src_db, select_session_sql(table, common, proj_id))
        exported[table] = len(rows)
        log(f"[sessions] export {table}: {len(rows)} rows cols={common}")

        key_columns = SESSION_VERIFY_KEYS[table]
        missing_key_columns = [column for column in key_columns if column not in common]
        if missing_key_columns:
            die(f"{table}: verification key columns missing: {missing_key_columns}")
        exported_keys[table] = {row_key(row, key_columns) for row in rows}

        if table == "cc_messages":
            exported_cc_rows = rows
            exported_turn_seq = {
                f"{r.get('turn_id')}:{r.get('seq')}"
                for r in rows
                if r.get("turn_id") is not None and r.get("seq") is not None
            }

        if not apply:
            continue
        if not rows:
            continue

        insert_cols = list(common)
        need_cluster = "cluster_id" in dst_cols and "cluster_id" not in common
        if need_cluster:
            insert_cols.append("cluster_id")

        chunk = 50
        for i in range(0, len(rows), chunk):
            part = rows[i : i + chunk]
            values_sql: list[str] = []
            for r in part:
                vals = [sql_literal(r.get(c)) for c in common]
                if need_cluster:
                    vals.append(sql_literal(cluster_id))
                values_sql.append("(" + ", ".join(vals) + ")")
            sql = (
                f"INSERT INTO {table} ({', '.join(insert_cols)}) VALUES\n"
                + ",\n".join(values_sql)
                + "\nON CONFLICT DO NOTHING;"
            )
            path = os.path.join(work_dir, f"mig_{table}_{i}.sql")
            with open(path, "w", encoding="utf-8") as f:
                f.write(sql)
            psql_file(dst_db, path)

    if not apply:
        log("[sessions] dry-run: skip INSERT / sequence update / PK verify write-side")
        return

    # Heal cc_messages gaps caused by global message_id collisions on destination.
    # Prefer serial new ids; logical uniqueness is (turn_id, seq). Author: kejiqing
    dst_turn_seq = {
        x
        for x in psql_text(
            dst_db,
            f"SELECT turn_id||':'||seq::text FROM cc_messages "
            f"WHERE COALESCE(proj_id, ds_id)={proj_id}",
        ).splitlines()
        if x
    }
    missing_keys = exported_turn_seq - dst_turn_seq
    if missing_keys:
        log(
            f"[sessions] heal cc_messages: {len(missing_keys)} (turn_id,seq) missing "
            f"after PK insert (likely message_id collision); remapping via serial"
        )
        heal_rows = [
            r
            for r in exported_cc_rows
            if f"{r.get('turn_id')}:{r.get('seq')}" in missing_keys
        ]
        # Insert without message_id so destination serial allocates safe ids.
        dst_cols = table_columns(dst_db, "cc_messages")
        heal_cols = [
            c
            for c in (
                "session_id",
                "ds_id",
                "turn_id",
                "iteration_id",
                "seq",
                "role",
                "blocks",
                "usage",
                "created_at_ms",
                "proj_id",
            )
            if c in dst_cols
        ]
        need_cluster = "cluster_id" in dst_cols
        insert_cols = list(heal_cols) + (["cluster_id"] if need_cluster else [])
        chunk = 50
        for i in range(0, len(heal_rows), chunk):
            part = heal_rows[i : i + chunk]
            values_sql = []
            for r in part:
                vals = [sql_literal(r.get(c)) for c in heal_cols]
                if need_cluster:
                    vals.append(sql_literal(cluster_id))
                values_sql.append("(" + ", ".join(vals) + ")")
            sql = (
                f"INSERT INTO cc_messages ({', '.join(insert_cols)}) VALUES\n"
                + ",\n".join(values_sql)
                + "\nON CONFLICT DO NOTHING;"
            )
            path = os.path.join(work_dir, f"mig_cc_messages_heal_{i}.sql")
            with open(path, "w", encoding="utf-8") as f:
                f.write(sql)
            psql_file(dst_db, path)
        log(f"[sessions] heal wrote {len(heal_rows)} rows")

    seq_sql = """
    SELECT setval(
      pg_get_serial_sequence('cc_messages','message_id'),
      GREATEST((SELECT COALESCE(MAX(message_id), 1) FROM cc_messages), 1),
      true
    );
    """
    seq_val = psql_text(dst_db, seq_sql).strip()
    log(f"[sessions] cc_messages sequence -> {seq_val}")

    def id_set(url: str, sql: str) -> set[str]:
        return {x for x in psql_text(url, sql).splitlines() if x}

    dst_ts = id_set(
        dst_db,
        f"SELECT turn_id||':'||seq::text FROM cc_messages "
        f"WHERE COALESCE(proj_id, ds_id)={proj_id}",
    )

    # Destination may already have extra rows; require exported snapshot ⊆ destination.
    ok = True
    for table in SESSION_TABLES:
        key_columns = SESSION_VERIFY_KEYS[table]
        dst_rows = psql_json_rows(
            dst_db, select_session_sql(table, list(key_columns), proj_id)
        )
        dst_keys = {row_key(row, key_columns) for row in dst_rows}
        missing = exported_keys[table] - dst_keys
        if missing:
            ok = False
            log(
                f"[sessions] FAIL {table} key={key_columns} "
                f"missing on dst sample={list(missing)[:10]}"
            )
        else:
            log(
                f"[sessions] OK {table} key={key_columns} "
                f"exported={len(exported_keys[table])} dst={len(dst_keys)} "
                f"extra_on_dst={len(dst_keys - exported_keys[table])}"
            )

    # Keep a direct textual check for the message-heal contract.
    if not exported_turn_seq.issubset(dst_ts):
        ok = False
        log(
            f"[sessions] FAIL cc_messages heal missing sample="
            f"{list(exported_turn_seq - dst_ts)[:10]}"
        )

    for table, n in exported.items():
        log(f"[sessions] exported {table}={n}")
    if not ok:
        die("exported session logical keys are incomplete on destination")


def list_all_project_sessions(
    gateway: str, proj_id: int, *, page_size: int = 100
) -> dict[str, dict[str, Any]]:
    """Paginate GET /v1/projects/{id}/sessions until hasMore is false. Author: kejiqing"""
    out: dict[str, dict[str, Any]] = {}
    before_ms: int | None = None
    before_sid: str | None = None
    for _ in range(10_000):
        qs = f"/v1/projects/{proj_id}/sessions?limit={page_size}"
        if before_ms is not None and before_sid:
            qs += f"&beforeUpdatedAtMs={before_ms}&beforeSessionId={before_sid}"
        code, body = http("GET", gateway, qs)
        if code != 200 or not isinstance(body, dict):
            die(f"session list failed: HTTP {code} {body}")
        sessions = body.get("sessions") or []
        if not sessions:
            break
        for s in sessions:
            out[s["sessionId"]] = s
        if not body.get("hasMore"):
            break
        last = sessions[-1]
        before_ms = last.get("updatedAtMs")
        before_sid = last.get("sessionId")
        if before_ms is None or not before_sid:
            die(f"cannot paginate sessions: missing cursor fields on {last}")
    return out


def verify_http_sessions(src_gateway: str, dst_gateway: str, proj_id: int) -> None:
    log("[verify-http] list sessions (paginated)")
    s1 = list_all_project_sessions(src_gateway, proj_id)
    s2 = list_all_project_sessions(dst_gateway, proj_id)
    missing = set(s1) - set(s2)
    if missing:
        die(f"HTTP sessionId missing on dst sample={list(missing)[:20]}")
    log(
        f"[verify-http] session src={len(s1)} dst={len(s2)} "
        f"extra_on_dst={len(set(s2) - set(s1))}"
    )

    mismatches: list[str] = []
    checked = 0
    for sid in s1:
        _, t1 = http("GET", src_gateway, f"/v1/sessions/{sid}/turns?projId={proj_id}")
        _, t2 = http("GET", dst_gateway, f"/v1/sessions/{sid}/turns?projId={proj_id}")
        turns1 = {
            t["turnId"]: t for t in ((t1 or {}).get("turns") if isinstance(t1, dict) else [])
        }
        turns2 = {
            t["turnId"]: t for t in ((t2 or {}).get("turns") if isinstance(t2, dict) else [])
        }
        if not set(turns1).issubset(set(turns2)):
            mismatches.append(f"{sid}: turnId missing on dst={set(turns1) - set(turns2)}")
            continue
        for tid, tr in turns1.items():
            checked += 1
            other = turns2[tid]
            for field in (
                "userPrompt",
                "status",
                "reportBody",
                "createdAtMs",
                "finishedAtMs",
            ):
                if tr.get(field) != other.get(field):
                    mismatches.append(f"{sid}/{tid}: {field}")
        # turnCount on list summary should at least cover migrated turns
        if (s1[sid].get("turnCount") or 0) > (s2[sid].get("turnCount") or 0):
            mismatches.append(f"{sid}: turnCount dst smaller")

    if mismatches:
        die(f"HTTP turn verify failed ({len(mismatches)}): {mismatches[:20]}")
    log(f"[verify-http] OK turns checked={checked}")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description=(
            "Migrate one claw-code projId (config via HTTP, sessions via PG). "
            "Writes require --apply."
        )
    )
    p.add_argument("--proj-id", type=int, required=True, help="Project id (>=1)")
    p.add_argument(
        "--src-gateway",
        required=True,
        help="Source gateway base URL, e.g. http://10.200.2.171:18088",
    )
    p.add_argument(
        "--dst-gateway",
        required=True,
        help="Destination gateway base URL, e.g. http://10.200.2.172:18088",
    )
    p.add_argument(
        "--src-database-url",
        default=os.environ.get("CLAW_MIGRATE_SRC_DATABASE_URL", ""),
        help="Source PG URL (or env CLAW_MIGRATE_SRC_DATABASE_URL)",
    )
    p.add_argument(
        "--dst-database-url",
        default=os.environ.get("CLAW_MIGRATE_DST_DATABASE_URL", ""),
        help="Destination PG URL (or env CLAW_MIGRATE_DST_DATABASE_URL)",
    )
    p.add_argument(
        "--scope",
        choices=("all", "config", "sessions"),
        default="all",
        help="What to migrate (default: all)",
    )
    p.add_argument(
        "--cluster-id",
        default="prod-claw-01",
        help="cluster_id filled on destination when source lacks the column",
    )
    p.add_argument(
        "--work-dir",
        default="/tmp/claw-migrate",
        help="Directory for generated INSERT SQL chunks",
    )
    p.add_argument(
        "--apply",
        action="store_true",
        help="Perform writes on destination (default is dry-run)",
    )
    p.add_argument(
        "--dry-run",
        action="store_true",
        help="Force inventory-only mode (default when --apply is absent)",
    )
    p.add_argument(
        "--verify-http",
        action="store_true",
        help="After sessions migrate (or alone with existing data), compare session/turn HTTP APIs",
    )
    return p.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.proj_id < 1:
        die("--proj-id must be >= 1")

    apply = bool(args.apply) and not bool(args.dry_run)
    mode = "APPLY" if apply else "DRY-RUN"
    log(
        f"migrate-claw-project mode={mode} projId={args.proj_id} "
        f"scope={args.scope} src={args.src_gateway} dst={args.dst_gateway}"
    )

    do_config = args.scope in ("all", "config")
    do_sessions = args.scope in ("all", "sessions")

    if do_sessions:
        if not args.src_database_url or not args.dst_database_url:
            die(
                "sessions scope requires --src-database-url and --dst-database-url "
                "(or CLAW_MIGRATE_SRC_DATABASE_URL / CLAW_MIGRATE_DST_DATABASE_URL)"
            )

    if do_config:
        migrate_config(
            src_gateway=args.src_gateway,
            dst_gateway=args.dst_gateway,
            proj_id=args.proj_id,
            apply=apply,
        )

    if do_sessions:
        migrate_sessions(
            src_db=args.src_database_url,
            dst_db=args.dst_database_url,
            proj_id=args.proj_id,
            cluster_id=args.cluster_id,
            apply=apply,
            work_dir=args.work_dir,
        )

    if args.verify_http:
        if not apply and do_sessions:
            log("[verify-http] running against current destination state (no apply)")
        verify_http_sessions(args.src_gateway, args.dst_gateway, args.proj_id)

    log("done")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
