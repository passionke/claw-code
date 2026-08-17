#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Bootstrap local kb-qa project (99011): config + KB sync to e2b NAS. Author: kejiqing"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GW = os.environ.get("GW", "http://127.0.0.1:18088").rstrip("/")
TOKEN = os.environ.get("CLAW_ADMIN_TOKEN", "").strip()
KB_PROJ = int(os.environ.get("KB_PROJ", "99011"))
NAS_HOST = os.environ.get("CLAW_E2B_NAS_HOST_MOUNT", "/home/sunmax/work/e2bserver/nas")
CLUSTER = os.environ.get("CLAW_CLUSTER_ID", "local-dev")
SSH_HOST = os.environ.get("CLAW_E2B_NAS_SSH", "sunmax@10.22.28.94")

KB_ALLOWED_TOOLS = ["grep_search", "read_file", "glob_search", "Skill"]


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


def kb_rules() -> list[dict]:
    prompt_doc = read_text("docs/gpos-assistant-prompt-content.md")
    items = [
        ("language-lock", extract_rule(prompt_doc, "language-lock")),
        ("product-manual-guard", extract_rule(prompt_doc, "product-manual-guard")),
        ("progress-mutex", extract_rule(prompt_doc, "progress-mutex")),
        (
            "kb-no-delegate",
            """# kb-qa 禁止再委托

你是 kb-qa specialist，不是 router。

- **禁止** `delegate_project`（无论 target projId 是什么）。
- **禁止** SQLBot / 经营问数 MCP。
- **禁止** `bash` 扫盘找文档；只查 `/claw_ds/home/kb`。
- **禁止**向用户输出 `mind.maxiot-inc.com` 或 `internal:` 链接（内部 FAQ 只给步骤）。
- 产品 how-to → 只走 `Skill("product-manual-qa")`。
""",
        ),
    ]
    return [
        {"relativePath": f".claw/rules/{name}.md", "content": content, "enabled": True}
        for name, content in items
    ]


def kb_skills() -> list[dict]:
    return [
        {
            "skillName": "product-manual-qa",
            "enabled": True,
            "skillContent": read_text("scripts/fixtures/skills/product-manual-qa.SKILL.md"),
        }
    ]


def fetch_kb() -> None:
    subprocess.run(["bash", str(ROOT / "scripts/gpos-manual-crawl/fetch_kb_from_nas.sh")], check=True)


def export_internal() -> None:
    subprocess.run(
        [sys.executable, str(ROOT / "scripts/gpos-manual-crawl/export_mind_faq_to_kb.py")],
        check=True,
    )
    subprocess.run(
        [sys.executable, str(ROOT / "scripts/gpos-manual-crawl/export_internal_kb_supplement.py")],
        check=True,
    )


def put_config_draft() -> None:
    cfg = http("GET", f"/v1/project/config/{KB_PROJ}")
    payload = {
        "rulesJson": kb_rules(),
        "mcpServersJson": cfg.get("mcpServersJson") or {},
        "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
        "skillsJson": kb_skills(),
        "allowedToolsJson": KB_ALLOWED_TOOLS,
        "claudeMd": read_text("scripts/fixtures/claude/gpos-kb-qa.CLAUDE.md"),
    }
    http("PUT", f"/v1/project/config/{KB_PROJ}", payload)
    print(f"==> draft updated for proj {KB_PROJ}")


def commit_activate() -> str:
    rev = http("POST", f"/v1/project/config/{KB_PROJ}/versions/commit", {"note": "feat: kb-qa bootstrap product-manual + bilingual kb (kejiqing)"})
    content_rev = rev.get("savedContentRev") or rev.get("contentRev") or ""
    if not content_rev:
        raise SystemExit(f"commit failed: {rev}")
    act = http("POST", f"/v1/project/config/{KB_PROJ}/versions/{content_rev}/activate")
    print(f"==> activated {content_rev} materialized={act.get('materialized')}")
    return content_rev


def sync_kb_to_nas() -> None:
    kb_src = os.environ.get("GPOS_MANUAL_KB", str(ROOT / "knowledge/gpos-user-manual"))
    remote_link = f"{NAS_HOST}/{CLUSTER}/proj_{KB_PROJ}/home/project_home_def"
    cmd = [
        "ssh",
        "-o",
        "BatchMode=yes",
        SSH_HOST,
        f"readlink -f {remote_link}",
    ]
    ver = subprocess.check_output(cmd, text=True).strip()
    if not ver:
        raise SystemExit(f"cannot resolve project_home_def: {remote_link}")
    dest = f"{SSH_HOST}:{ver}/home/kb/"
    print(f"==> rsync KB -> {dest}")
    subprocess.run(
        [
            "rsync",
            "-az",
            "--delete",
            "--exclude",
            "eval/",
            "--exclude",
            "README.md",
            "--exclude",
            ".mind-export/",
            f"{kb_src}/",
            dest,
        ],
        check=True,
    )
    check = subprocess.check_output(
        [
            "ssh",
            "-o",
            "BatchMode=yes",
            SSH_HOST,
            f"python3 -c \"import json;from pathlib import Path;p=Path('{ver}/home/kb/manifest.json');m=json.loads(p.read_text());print(m);assert m.get('en_count',0)>50 and m.get('th_count',0)>50\"",
        ],
        text=True,
    )
    print(check.strip())


def reset_worker() -> None:
    try:
        out = http("POST", f"/v1/projects/{KB_PROJ}/e2b-worker/reset")
        print(f"==> worker reset: {out}")
    except urllib.error.HTTPError as e:
        print(f"warn: worker reset HTTP {e.code} (may need manual reset)", file=sys.stderr)


def verify() -> None:
    cfg = http("GET", f"/v1/project/config/{KB_PROJ}")
    skills = [s.get("skillName") for s in (cfg.get("skillsJson") or []) if isinstance(s, dict)]
    tools = cfg.get("allowedToolsJson") or []
    print(f"verify proj={KB_PROJ} skills={skills} tools={tools} rev={cfg.get('stableContentRev')}")
    assert "product-manual-qa" in skills, skills
    assert "delegate_project" not in tools, tools
    assert "bash" not in tools, tools


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-fetch", action="store_true")
    ap.add_argument("--skip-sync", action="store_true")
    ap.add_argument("--config-only", action="store_true")
    args = ap.parse_args()
    if not args.skip_fetch:
        fetch_kb()
        export_internal()
    put_config_draft()
    commit_activate()
    if not args.config_only and not args.skip_sync:
        sync_kb_to_nas()
        reset_worker()
    verify()
    print("Done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
