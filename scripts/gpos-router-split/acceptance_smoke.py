#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Local specialist-router acceptance helpers (no pre-release changes). Author: kejiqing"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

GW = os.environ.get("GW", "http://127.0.0.1:18088").rstrip("/")
TOKEN = os.environ.get("CLAW_ADMIN_TOKEN", "").strip()
ROUTER_PROJ = int(os.environ.get("ROUTER_PROJ", os.environ.get("GPOS_PROJ_ID", "99010")))
KB_PROJ = int(os.environ.get("KB_PROJ", "99011"))
OPS_PROJ = int(os.environ.get("OPS_PROJ", "99012"))
MCP_URL = os.environ.get("CLAW_ADMIN_MCP_URL", f"{GW}/v1/admin/mcp")
OUT = Path(os.environ.get("ROUTER_ACCEPTANCE_OUT", "knowledge/router-test-local/eval"))
EXTRA = {
    "store_id": "S20240930134500007303",
    "org_id": "O20240930134500007302",
    "tenant_code": "GPOS",
}

HANDOFF_ONLY_MARKERS = ("已转交", "稍后为您", "转交给", "稍后再试")
OPS_DATA_MARKERS = (
    "销售额",
    "订单",
    "泰铢",
    "THB",
    "ยอดขาย",
    "ออเดอร์",
    "sales",
    "order",
    "经营数据",
    "无经营数据",
)
MANUAL_URL_MARKERS = ("gpos.co.th/user-manual", "gpos.co.th/th/user-manual", "gpos.co.th/en/user-manual")


def is_handoff_only(text: str) -> bool:
    t = (text or "").strip()
    if len(t) < 80:
        return any(m in t for m in HANDOFF_ONLY_MARKERS)
    return False


def delegate_tool_output(tools: list) -> dict:
    for t in tools:
        if not isinstance(t, dict) or t.get("toolName") != "delegate_project_tool":
            continue
        raw = t.get("output") or ""
        try:
            out = json.loads(raw) if isinstance(raw, str) else raw
        except json.JSONDecodeError:
            continue
        if isinstance(out, dict):
            return out
    return {}


def delegate_tool_report_path(tools: list) -> str:
    return (delegate_tool_output(tools).get("reportPath") or "").strip()


def delegate_tool_message(tools: list) -> str:
    """Legacy helper: body is no longer inlined; prefer reportPath + user report."""
    return (delegate_tool_output(tools).get("message") or "").strip()


def delegate_target_proj(tools: list) -> int | None:
    found: int | None = None
    for t in tools:
        if not isinstance(t, dict) or t.get("toolName") != "delegate_project_tool":
            continue
        if t.get("isError"):
            continue
        raw = t.get("input") or t.get("arguments") or ""
        try:
            inp = json.loads(raw) if isinstance(raw, str) else raw
        except json.JSONDecodeError:
            continue
        if isinstance(inp, dict):
            pid = inp.get("projId")
            if isinstance(pid, int):
                found = pid
            elif isinstance(pid, str) and pid.isdigit():
                found = int(pid)
    return found


def looks_like_ops_answer(text: str) -> bool:
    t = (text or "").lower()
    return any(m.lower() in t for m in OPS_DATA_MARKERS)


def has_manual_url(text: str) -> bool:
    t = (text or "").lower()
    return any(m in t for m in MANUAL_URL_MARKERS)


def http(method: str, path: str, body: dict | None = None) -> dict:
    headers = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/json"}
    data = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(f"{GW}{path}", data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read().decode("utf-8"))


def mcp_call(name: str, arguments: dict, timeout: int = 600) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "id": int(time.time() * 1000) % 1_000_000_000,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    }
    req = urllib.request.Request(
        MCP_URL,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = json.loads(resp.read().decode("utf-8"))
    if body.get("error"):
        raise RuntimeError(body["error"])
    content = body.get("result", {}).get("content") or []
    text = "".join(c.get("text") or "" for c in content if isinstance(c, dict))
    try:
        return json.loads(text) if text else body.get("result") or body
    except json.JSONDecodeError:
        return {"raw": text}


def check_config() -> dict:
    targets = http("GET", f"/v1/projects/{ROUTER_PROJ}/delegate-targets")
    tids = {t["targetProjId"] for t in targets.get("targets", [])}
    ok = {KB_PROJ, OPS_PROJ} <= tids and targets.get("initiatorProjId") == ROUTER_PROJ
    return {
        "check": "config",
        "pass": ok,
        "initiatorProjId": targets.get("initiatorProjId"),
        "targetProjIds": sorted(tids),
    }


def scenario_session_reuse() -> dict:
    parent_sid = f"rtest_{int(time.time())}"
    r1 = http(
        "POST",
        f"/v1/projects/{ROUTER_PROJ}/delegate/resolve-session",
        {"parentSessionId": parent_sid, "delegateProjId": KB_PROJ},
    )
    r2 = http(
        "POST",
        f"/v1/projects/{ROUTER_PROJ}/delegate/resolve-session",
        {"parentSessionId": parent_sid, "delegateProjId": KB_PROJ},
    )
    ok = (
        r1["delegateSessionId"] == r2["delegateSessionId"]
        and r1["rootSessionId"] == parent_sid
        and r2.get("created") is False
    )
    return {
        "check": "session_reuse",
        "pass": ok,
        "parentSessionId": parent_sid,
        "delegateSessionId": r1.get("delegateSessionId"),
        "firstCreated": r1.get("created"),
        "secondCreated": r2.get("created"),
    }


def scenario_negative_allowlist() -> dict:
    try:
        http(
            "POST",
            f"/v1/projects/{ROUTER_PROJ}/delegate/resolve-session",
            {"parentSessionId": "neg_test", "delegateProjId": 999999},
        )
        return {"check": "negative_allowlist", "pass": False, "error": "expected 4xx"}
    except urllib.error.HTTPError as e:
        return {
            "check": "negative_allowlist",
            "pass": 400 <= e.code < 500,
            "httpStatus": e.code,
        }


def scenario_solve(prompt: str, session_id: str | None = None) -> dict:
    args: dict = {
        "projId": ROUTER_PROJ,
        "userPrompt": prompt,
        "extraSession": EXTRA,
        "timeoutSeconds": 300,
    }
    if session_id:
        args["sessionId"] = session_id
    t0 = time.time()
    solve = mcp_call("gateway_solve", args, timeout=360)
    sid = solve.get("sessionId")
    oj = solve.get("outputJson") or {}
    if isinstance(oj, str):
        try:
            oj = json.loads(oj)
        except json.JSONDecodeError:
            oj = {}
    msg = ""
    if isinstance(oj, dict):
        msg = (oj.get("message") or oj.get("report") or "")[:500]
    # ToolCompleteTurn: sync MCP may leave message empty; canonical body is task/PG. Author: kejiqing
    if sid and not (msg or "").strip():
        try:
            task = http("GET", f"/v1/tasks/{sid}")
            result = task.get("result") or {}
            toj = result.get("outputJson") or {}
            if isinstance(toj, str):
                try:
                    toj = json.loads(toj)
                except json.JSONDecodeError:
                    toj = {}
            if isinstance(toj, dict):
                msg = (toj.get("message") or toj.get("report") or "")[:500]
            if not (msg or "").strip():
                msg = (result.get("outputText") or "")[:500]
        except urllib.error.HTTPError:
            pass
    http_ok = (
        solve.get("status") in ("succeeded", "completed", None)
        and solve.get("clawExitCode", 0) == 0
    )
    tools: list = []
    turn_id = None
    if sid:
        try:
            turns = http("GET", f"/v1/sessions/{sid}/turns?projId={ROUTER_PROJ}")
            last = (turns.get("turns") or [])[-1] if turns.get("turns") else None
            turn_id = (last or {}).get("turnId")
            if turn_id:
                tools = (
                    http(
                        "GET",
                        f"/v1/sessions/{sid}/turns/{turn_id}/tools?proj_id={ROUTER_PROJ}",
                    ).get("tools")
                    or []
                )
        except urllib.error.HTTPError as e:
            tools = [{"error": f"tools fetch HTTP {e.code}"}]
    delegate_ok = any(
        isinstance(t, dict)
        and t.get("toolName") == "delegate_project_tool"
        and not t.get("isError")
        for t in tools
    )
    report_path = delegate_tool_report_path(tools)
    tool_path_ok = (not delegate_ok) or bool(report_path)
    # User-visible report from task/PG; tool result must point at router-session file.
    content_ok = bool(msg) and not is_handoff_only(msg)
    user_visible_ok = content_ok
    return {
        "check": "solve",
        "pass": http_ok and delegate_ok and tool_path_ok and content_ok and user_visible_ok,
        "sessionId": sid,
        "turnId": turn_id,
        "status": solve.get("status"),
        "clawExitCode": solve.get("clawExitCode"),
        "delegateProjectCalled": delegate_ok,
        "delegateTargetProjId": delegate_target_proj(tools),
        "delegateReportPath": report_path,
        "delegateMessageLen": len(delegate_tool_message(tools)),
        "userReportHandoffOnly": is_handoff_only(msg),
        "toolNames": [t.get("toolName") for t in tools if isinstance(t, dict)],
        "messagePreview": msg,
        "elapsedSec": round(time.time() - t0, 1),
        "userPrompt": prompt,
    }


def scenario_solve_ops(prompt: str = "昨天的销售额和订单量是多少？") -> dict:
    r = scenario_solve(prompt)
    body = r.get("messagePreview") or ""
    target_ok = r.get("delegateTargetProjId") == OPS_PROJ
    ops_shape_ok = looks_like_ops_answer(body) and not has_manual_url(body)
    path_ok = bool(r.get("delegateReportPath"))
    r["check"] = "solve_ops"
    r["pass"] = bool(r.get("pass")) and target_ok and ops_shape_ok and path_ok
    r["delegateTargetOk"] = target_ok
    r["opsAnswerShapeOk"] = ops_shape_ok
    r["hasManualUrl"] = has_manual_url(body)
    return r


def main() -> int:
    if not TOKEN:
        print("缺少 CLAW_ADMIN_TOKEN", file=sys.stderr)
        return 2
    ap = argparse.ArgumentParser()
    ap.add_argument("--check-config", action="store_true")
    ap.add_argument("--scenario", choices=["session", "negative", "1", "2", "4"])
    args = ap.parse_args()

    OUT.mkdir(parents=True, exist_ok=True)
    rows: list[dict] = []

    if args.check_config or not args.scenario:
        rows.append(check_config())
        rows.append(scenario_session_reuse())
        rows.append(scenario_negative_allowlist())

    if args.scenario == "session":
        rows = [scenario_session_reuse()]
    elif args.scenario == "negative":
        rows = [scenario_negative_allowlist()]
    elif args.scenario == "1":
        sid = None
        for i in range(3):
            r = scenario_solve("后台怎么创建商品分类？", sid)
            sid = r.get("sessionId") or sid
            r["round"] = i + 1
            rows.append(r)
    elif args.scenario == "2":
        rows.append(scenario_solve_ops())
    elif args.scenario == "4":
        rows.append(
            scenario_solve("怎么在后台添加商品还有昨天销售额多少？")
        )

    out_path = OUT / f"acceptance-smoke-{int(time.time())}.jsonl"
    out_path.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows), encoding="utf-8")
    for r in rows:
        print(json.dumps(r, ensure_ascii=False))
    print("WROTE", out_path)
    passed = sum(1 for r in rows if r.get("pass"))
    return 0 if passed == len(rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
