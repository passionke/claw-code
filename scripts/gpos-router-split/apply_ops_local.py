#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Bootstrap local ops-analysis project (99012): SQLBot MCP + query skills. Author: kejiqing"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GW = os.environ.get("GW", "http://127.0.0.1:18088").rstrip("/")
TOKEN = os.environ.get("CLAW_ADMIN_TOKEN", "").strip()
OPS_PROJ = int(os.environ.get("OPS_PROJ", "99012"))
SQLBOT_URL = os.environ.get(
    "SQLBOT_MCP_URL", "https://sqlboy.maxiot-inc.com/mcp/mcp-streamable"
).strip()
SQLBOT_BEARER = os.environ.get("SQLBOT_MCP_BEARER", "").strip()

OPS_ALLOWED_TOOLS = [
    "Skill",
    "glob_search",
    "mcp__sqlbot-streamable__*",
]

OPS_SKILL_EXCLUDE = {"product-manual-qa", "self-introduction"}


def http(method: str, path: str, body: dict | None = None) -> dict:
    if not TOKEN:
        raise SystemExit("CLAW_ADMIN_TOKEN required")
    headers = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/json"}
    data = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(f"{GW}{path}", data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read().decode("utf-8"))


def read_text(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def extract_rule(doc: str, rule_name: str) -> str:
    marker = f"## Rule: {rule_name}"
    chunk = doc.split(marker, 1)[1].split("\n## Rule:", 1)[0]
    chunk = chunk.strip()
    if chunk.startswith("```markdown"):
        chunk = chunk[len("```markdown") :].strip()
    if chunk.endswith("```"):
        chunk = chunk[: -len("```")].strip()
    return chunk


def ops_rules() -> list[dict]:
    prompt_doc = read_text("docs/gpos-assistant-prompt-content.md")
    rule_names = [
        "language-lock",
        "skill-before-mcp",
        "progress-mutex",
        "time-bounds",
        "entity-session",
        "sqlbot-workflow",
        "analysis-scope",
        "data-quality",
        "report-output-format",
        "user-facing-style",
    ]
    items = [(name, extract_rule(prompt_doc, name)) for name in rule_names]
    items.extend(
        [
            (
                "ops-no-delegate",
                """# ops-analysis 禁止再委托

你是 ops-analysis specialist，不是 router。

- **禁止** `delegate_project_tool`（无论 target projId 是什么）。
- **禁止** `product-manual-qa`、检索 `/claw_ds/home/kb`、输出 gpos.co.th 手册链接。
- 产品 how-to 不属于本 specialist（router 应转 kb-qa）。
""",
            ),
            (
                "ops-no-manual",
                """# ops-analysis 禁止走手册

经营问数 → SQLBot MCP + 分析 skills。

- **禁止** `grep_search` / `read_file` 查 `/claw_ds/home/kb`。
- **禁止** 向用户输出官方用户手册 URL。
""",
            ),
        ]
    )
    return [
        {"relativePath": f".claw/rules/{name}.md", "content": content, "enabled": True}
        for name, content in items
    ]


def ops_skills() -> list[dict]:
    raw = json.loads(
        (ROOT / "scripts/fixtures/proj271_skills_with_product_manual.json").read_text(
            encoding="utf-8"
        )
    )
    out: list[dict] = []
    for s in raw.get("skillsJson") or []:
        if not isinstance(s, dict):
            continue
        name = (s.get("skillName") or "").strip()
        if not name or name in OPS_SKILL_EXCLUDE:
            continue
        out.append(
            {
                "skillName": name,
                "enabled": s.get("enabled", True),
                "skillContent": s.get("skillContent") or "",
            }
        )
    return out


def mcp_servers() -> dict:
    if not SQLBOT_BEARER:
        raise SystemExit(
            "SQLBOT_MCP_BEARER required (Bearer token for sqlbot-streamable MCP)"
        )
    auth = SQLBOT_BEARER if SQLBOT_BEARER.startswith("Bearer ") else f"Bearer {SQLBOT_BEARER}"
    return {
        "sqlbot-streamable": {
            "url": SQLBOT_URL,
            "headers": {"Authorization": auth},
        }
    }


def put_config_draft() -> None:
    cfg = http("GET", f"/v1/project/config/{OPS_PROJ}")
    payload = {
        "rulesJson": ops_rules(),
        "mcpServersJson": mcp_servers(),
        "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
        "skillsJson": ops_skills(),
        "allowedToolsJson": OPS_ALLOWED_TOOLS,
        "claudeMd": read_text("scripts/fixtures/claude/gpos-ops-qa.CLAUDE.md"),
    }
    http("PUT", f"/v1/project/config/{OPS_PROJ}", payload)
    print(f"==> draft updated for proj {OPS_PROJ}")


def commit_activate() -> str:
    rev = http(
        "POST",
        f"/v1/project/config/{OPS_PROJ}/versions/commit",
        {"note": "feat: ops-analysis bootstrap sqlbot + query skills (kejiqing)"},
    )
    content_rev = rev.get("savedContentRev") or rev.get("contentRev") or ""
    if not content_rev:
        raise SystemExit(f"commit failed: {rev}")
    act = http("POST", f"/v1/project/config/{OPS_PROJ}/versions/{content_rev}/activate")
    print(f"==> activated {content_rev} materialized={act.get('materialized')}")
    return content_rev


def reset_worker() -> None:
    try:
        out = http("POST", f"/v1/projects/{OPS_PROJ}/e2b-worker/reset")
        print(f"==> worker reset: {out}")
    except urllib.error.HTTPError as e:
        print(f"warn: worker reset HTTP {e.code} (may need manual reset)", file=sys.stderr)


def verify() -> None:
    cfg = http("GET", f"/v1/project/config/{OPS_PROJ}")
    skills = [s.get("skillName") for s in (cfg.get("skillsJson") or []) if isinstance(s, dict)]
    tools = cfg.get("allowedToolsJson") or []
    mcp = list((cfg.get("mcpServersJson") or {}).keys())
    print(
        f"verify proj={OPS_PROJ} skills_n={len(skills)} mcp={mcp} tools={tools} rev={cfg.get('stableContentRev')}"
    )
    assert "sqlbot-streamable" in mcp, mcp
    assert "product-manual-qa" not in skills, skills
    assert "generate-yesterday-sales-report" in skills, skills
    assert "delegate_project_tool" not in tools, tools
    assert "bash" not in tools, tools
    assert "grep_search" not in tools, tools


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-reset", action="store_true")
    args = ap.parse_args()
    put_config_draft()
    commit_activate()
    if not args.skip_reset:
        reset_worker()
    verify()
    print("Done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
