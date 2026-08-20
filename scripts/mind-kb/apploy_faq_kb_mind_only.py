#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Bootstrap common FAQ KB from a Mind folder only. Author: kejiqing"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MCP_CONFIG = Path.home() / ".cursor/mcp.json"
MCP_SERVER = "mind_455785b1-1358-45ce-89c0-5a66e56d7826"
MIND_API_URL_ENV = "CLAW_MIND_API_URL"
MIND_API_HEADERS_ENV = "CLAW_MIND_API_HEADERS_JSON"
MIND_API_AUTH_ENV = "CLAW_MIND_API_AUTHORIZATION"
KB_ALLOWED_TOOLS = ["grep_search", "read_file", "glob_search", "Skill"]
DEFAULT_NAS_SSH = ""
DEFAULT_CLUSTER = "pre-claw-01"
DEFAULT_NAS_ROOT = "/data/claw-nas"
FAQ_SKILL_NAME = "mind-common-faq-qa"

CLAUDE_MD = f"""# 内部 FAQ 专家（faq-kb）

Author: kejiqing

你是 **内部 FAQ 专家**。Router 已将用户问题委托给你；你只根据 `/claw_ds/home/kb/en/internal/mind/faq/` 内的内部 FAQ 文档回答操作步骤与说明。

这层配置只承载 **通用 Mind FAQ KB** 的读取约束；具体业务（如 IPOS / xPos）的项目级对话风格、系统提示词和其他 skill，应由上层业务 proj 自行设定。

## 执行顺序（强制）

1. 只调 `Skill("{FAQ_SKILL_NAME}")`，按 skill 检索 `/claw_ds/home/kb/en/internal/mind/faq/`。
2. 1～2 轮内给出步骤或说明。
3. 禁止 `delegate_project_tool`（你不是 router，不得再委托）。
4. 禁止 SQLBot / 任何经营问数 MCP。
5. 禁止 `bash` 扫盘；只用 `grep_search` / `read_file` / `glob_search`。

## 语言

输出语言 = 用户本轮书写体系（中 / 泰 / English）。

## 语气

简洁、面向店长/店员；不暴露内部路径、Mind 链接、projId、delegate 等实现细节。
"""

SKILL_MD = f"""---
name: {FAQ_SKILL_NAME}
description: 当用户询问内部 FAQ 与产品操作说明时使用。只检索 `/claw_ds/home/kb/en/internal/mind/faq/`，禁止输出 Mind 链接，禁止 SQLBot。
---

# {FAQ_SKILL_NAME}

Author: kejiqing

## 何时使用

内部 FAQ 解释、后台配置步骤、通用产品操作说明。

## 检索范围

- 只查 `/claw_ds/home/kb/en/internal/mind/faq/`
- 禁止读取其他 KB 根目录
- 禁止输出 `mind.maxiot-inc.com` 或 `internal:` 链接

## 检索协议

1. `glob_search(path=/claw_ds/home/kb/en/internal/mind/faq, pattern=**/*.md)`
2. `grep_search(path=/claw_ds/home/kb/en/internal/mind/faq, pattern=<关键词>)`
3. `read_file` 命中文档
4. 只输出步骤与结论，不输出来源链接

## 禁止

1. 禁止 `mcp__sqlbot*`
2. 禁止 `delegate_project_tool`
3. 禁止向用户暴露内部路径 / Mind 链接 / 项目编号
"""


def slug(text: str) -> str:
    s = re.sub(r"[^\w\u0E00-\u0E7F\u4e00-\u9fff-]+", "-", text.strip().lower())
    return re.sub(r"-+", "-", s).strip("-") or "doc"


def read_text(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def extract_rule(doc: str, rule_name: str) -> str:
    marker = f"## Rule: {rule_name}"
    chunk = doc.split(marker, 1)[1].split("\n## Rule:", 1)[0].strip()
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
            "faq-no-delegate",
            f"""# faq-kb 禁止再委托

你是 faq-kb specialist，不是 router。

- **禁止** `delegate_project_tool`（无论 target projId 是什么）。
- **禁止** SQLBot / 经营问数 MCP。
- **禁止** `bash` 扫盘找文档；只查 `/claw_ds/home/kb/en/internal/mind/faq/`。
- **禁止**向用户输出 `mind.maxiot-inc.com` 或 `internal:` 链接。
- 通用 FAQ → 只走 `Skill("{FAQ_SKILL_NAME}")`。
""",
        ),
    ]
    return [
        {"relativePath": f".claw/rules/{name}.md", "content": content, "enabled": True}
        for name, content in items
    ]


def load_repo_env() -> dict[str, str]:
    out: dict[str, str] = {}
    env_path = ROOT / ".env"
    if not env_path.exists():
        return out
    for line in env_path.read_text(encoding="utf-8").splitlines():
        raw = line.strip()
        if not raw or raw.startswith("#") or "=" not in raw:
            continue
        key, value = raw.split("=", 1)
        out[key.strip()] = value.strip().strip('"').strip("'")
    return out


def http(method: str, gw: str, token: str, path: str, body: dict | None = None) -> dict:
    headers = {"Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(f"{gw}{path}", data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read().decode("utf-8"))


def load_mcp() -> tuple[str, dict[str, str]]:
    env_url = os.environ.get(MIND_API_URL_ENV, "").strip()
    if env_url:
        headers_json = os.environ.get(MIND_API_HEADERS_ENV, "").strip()
        if headers_json:
            headers = json.loads(headers_json)
        else:
            headers = {}
        auth = os.environ.get(MIND_API_AUTH_ENV, "").strip()
        if auth:
            headers.setdefault("Authorization", auth)
        headers.setdefault("Content-Type", "application/json")
        headers.setdefault("Accept", "application/json, text/event-stream")
        return env_url, {k: str(v) for k, v in headers.items()}
    cfg = json.loads(MCP_CONFIG.read_text(encoding="utf-8"))
    srv = cfg["mcpServers"][MCP_SERVER]
    url = srv["url"]
    headers = {k: str(v) for k, v in srv.get("headers", {}).items()}
    headers.setdefault("Content-Type", "application/json")
    headers.setdefault("Accept", "application/json, text/event-stream")
    return url, headers


def mind_call(url: str, headers: dict[str, str], name: str, arguments: dict) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "id": int(datetime.now(tz=timezone.utc).timestamp() * 1000) % 1_000_000_000,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    }
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        body = json.loads(resp.read().decode("utf-8"))
    if body.get("error"):
        raise RuntimeError(str(body["error"]))
    content = body.get("result", {}).get("content") or []
    text = "".join(item.get("text") or "" for item in content if isinstance(item, dict))
    return json.loads(text) if text else {}


def resolve_folder(url: str, headers: dict[str, str], folder_id: str) -> tuple[str, list[dict]]:
    spaces = mind_call(url, headers, "list_spaces", {}).get("spaces") or []
    for space in spaces:
        doc_space_id = str(space["id"])
        tree = mind_call(url, headers, "get_space_tree", {"docSpaceId": doc_space_id})
        nodes = tree.get("nodes") or []
        if any(str(node.get("id")) == folder_id for node in nodes):
            return doc_space_id, nodes
    raise SystemExit(f"mind folder not found: {folder_id}")


def descendant_folder_ids(nodes: list[dict], root_folder_id: str) -> list[str]:
    folder_ids = {
        str(node.get("id") or "")
        for node in nodes
        if str(node.get("type") or "").lower() == "folder" and str(node.get("id") or "")
    }
    folder_ids.add(root_folder_id)
    children: dict[str, list[str]] = {}
    for node in nodes:
        if str(node.get("type") or "").lower() != "folder":
            continue
        parent = str(node.get("parentId") or "")
        node_id = str(node.get("id") or "")
        if not node_id:
            continue
        children.setdefault(parent, []).append(node_id)
    out: list[str] = []
    stack = [root_folder_id]
    seen: set[str] = set()
    while stack:
        cur = stack.pop()
        if cur in seen:
            continue
        if cur not in folder_ids:
            continue
        seen.add(cur)
        out.append(cur)
        stack.extend(children.get(cur, []))
    return out


def list_folder_documents(
    url: str, headers: dict[str, str], doc_space_id: str, folder_id: str
) -> list[dict]:
    docs: list[dict] = []
    cursor: str | None = None
    while True:
        args: dict[str, object] = {"docSpaceId": doc_space_id, "parentId": folder_id, "limit": 50}
        if cursor:
            args["cursor"] = cursor
        resp = mind_call(url, headers, "list_documents", args)
        docs.extend(resp.get("documents") or [])
        cursor = resp.get("nextCursor")
        if not cursor:
            break
    return docs


def export_mind_folder(folder_id: str, out_root: Path, target_rel_path: str) -> int:
    url, headers = load_mcp()
    doc_space_id, nodes = resolve_folder(url, headers, folder_id)
    folder_ids = descendant_folder_ids(nodes, folder_id)
    docs: list[dict] = []
    for fid in folder_ids:
        docs.extend(list_folder_documents(url, headers, doc_space_id, fid))
    if not docs:
        raise SystemExit(f"no documents found under mind folder {folder_id}")
    rel_parts = [part for part in target_rel_path.strip("/").split("/") if part]
    if not rel_parts:
        raise SystemExit("target_rel_path must not be empty")
    faq_root = out_root.joinpath(*rel_parts)
    faq_root.mkdir(parents=True, exist_ok=True)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    index_lines = [
        "# 通用 FAQ（内部 Mind 文档导出）",
        "",
        "Author: kejiqing",
        "",
        "内部 FAQ 文档，仅供 specialist 检索；禁止向用户暴露 Mind 链接。",
        "",
    ]
    count = 0
    for doc in docs:
        rid = str(doc.get("resourceId") or doc.get("id") or "").strip()
        title = str(doc.get("title") or rid).strip()
        if not rid or not title:
            continue
        detail = mind_call(url, headers, "get_document", {"resourceId": rid})
        markdown = str(detail.get("markdown") or "").strip()
        path_parts = [slug(str(x)) for x in (doc.get("path") or [])]
        if not path_parts:
            path_parts = ["uncategorized"]
        name = slug(title) + ".md"
        out = faq_root.joinpath(*path_parts, name)
        out.parent.mkdir(parents=True, exist_ok=True)
        internal_ref = f"internal:mind-faq/{'/'.join(path_parts)}/{name}"
        wrapped = f"""---
title: {title}
source_url: {internal_ref}
lang: zh
keywords: [faq, internal, mind]
crawled_at: {now}
origin: mind
public_citation: false
---

# {title}

## Steps / Content

{markdown}
"""
        out.write_text(wrapped, encoding="utf-8")
        index_lines.append(f"- {title} -> `{'/'.join(path_parts)}/{name}`")
        count += 1
    (faq_root / "index.md").write_text("\n".join(index_lines) + "\n", encoding="utf-8")
    return count


def put_config_draft(gw: str, token: str, proj_id: int) -> None:
    cfg = http("GET", gw, token, f"/v1/project/config/{proj_id}")
    payload = {
        "rulesJson": kb_rules(),
        "mcpServersJson": cfg.get("mcpServersJson") or {},
        "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
        "skillsJson": [
            {
                "skillName": FAQ_SKILL_NAME,
                "enabled": True,
                "skillContent": SKILL_MD,
            }
        ],
        "allowedToolsJson": KB_ALLOWED_TOOLS,
        "claudeMd": CLAUDE_MD,
    }
    http("PUT", gw, token, f"/v1/project/config/{proj_id}", payload)
    print(f"==> draft updated for proj {proj_id}")


def commit_activate(gw: str, token: str, proj_id: int) -> str:
    rev = http(
        "POST",
        gw,
        token,
        f"/v1/project/config/{proj_id}/versions/commit",
        {"note": "feat: bootstrap common mind-only faq kb (kejiqing)"},
    )
    content_rev = rev.get("savedContentRev") or rev.get("contentRev") or ""
    if not content_rev:
        raise SystemExit(f"commit failed: {rev}")
    act = http(
        "POST",
        gw,
        token,
        f"/v1/project/config/{proj_id}/versions/{content_rev}/activate",
    )
    print(f"==> activated {content_rev} materialized={act.get('materialized')}")
    return content_rev


def sync_kb_tree_local(local_kb_root: Path, project_home: Path) -> None:
    if not project_home.is_dir():
        raise SystemExit(f"project_home does not exist: {project_home}")
    stage_id = datetime.now(timezone.utc).strftime("kb-sync-%Y%m%dT%H%M%SZ")
    stage_root = project_home / ".kb-staging" / stage_id
    stage_kb = stage_root / "kb"
    stage_kb.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(local_kb_root, stage_kb)
    manifest_count = sum(1 for _ in stage_kb.rglob("*.md"))
    if manifest_count <= 0:
        raise SystemExit("staged KB has no markdown files")
    target = project_home / "kb"
    backup = project_home / ".kb-backup" / stage_id
    backup.parent.mkdir(parents=True, exist_ok=True)
    if backup.exists():
        shutil.rmtree(backup, ignore_errors=True)
    if target.exists():
        target.rename(backup)
    stage_kb.rename(target)
    shutil.rmtree(stage_root, ignore_errors=True)


def sync_kb_tree_remote(
    local_kb_root: Path, faq_proj_id: int, cluster: str, nas_root: str, ssh_host: str
) -> None:
    remote_link = f"{nas_root}/{cluster}/proj_{faq_proj_id}/home/project_home_def"
    ver = subprocess.check_output(
        ["ssh", "-o", "BatchMode=yes", ssh_host, f"readlink -f {remote_link}"],
        text=True,
    ).strip()
    if not ver:
        raise SystemExit(f"cannot resolve project_home_def: {remote_link}")
    stage_id = datetime.now(timezone.utc).strftime("kb-sync-%Y%m%dT%H%M%SZ")
    remote_stage_root = f"{ver}/home/.kb-staging/{stage_id}"
    remote_stage_kb = f"{remote_stage_root}/kb"
    subprocess.run(
        [
            "ssh",
            "-o",
            "BatchMode=yes",
            ssh_host,
            f"mkdir -p {remote_stage_kb}",
        ],
        check=True,
    )
    remote_stage = f"{ssh_host}:{remote_stage_kb}/"
    print(f"==> rsync staged KB -> {remote_stage}")
    subprocess.run(["rsync", "-az", f"{local_kb_root}/", remote_stage], check=True)
    manifest_count = sum(1 for _ in local_kb_root.rglob("*.md"))
    if manifest_count <= 0:
        raise SystemExit("staged KB has no markdown files")
    swap_cmd = f"""
set -euo pipefail
target="{ver}/home/kb"
stage="{remote_stage_kb}"
backup="{ver}/home/.kb-backup/{stage_id}"
mkdir -p "$(dirname "$backup")"
test -d "$stage"
count="$(find "$stage" -type f | wc -l | tr -d ' ')"
test "$count" -gt 0
rm -rf "$backup"
if [ -d "$target" ]; then mv "$target" "$backup"; fi
mv "$stage" "$target"
rm -rf "{remote_stage_root}"
"""
    subprocess.run(
        ["ssh", "-o", "BatchMode=yes", ssh_host, swap_cmd],
        check=True,
        text=True,
    )


def parse_kb_sources_json(raw: str) -> list[dict]:
    items = json.loads(raw)
    if not isinstance(items, list):
        raise SystemExit("--kb-sources-json must be a JSON array")
    out: list[dict] = []
    for idx, item in enumerate(items):
        if not isinstance(item, dict):
            raise SystemExit(f"kbSourcesJson[{idx}] must be an object")
        if item.get("enabled") is False:
            continue
        source_url = str(item.get("sourceUrl") or "").strip()
        target_rel_path = str(item.get("targetRelPath") or "").strip()
        if not source_url or not target_rel_path:
            raise SystemExit(f"kbSourcesJson[{idx}] requires sourceUrl + targetRelPath")
        folder_id = source_url.rstrip("/").split("/")[-1]
        out.append(
            {
                "sourceUrl": source_url,
                "folderId": folder_id,
                "targetRelPath": target_rel_path,
            }
        )
    if not out:
        raise SystemExit("kbSourcesJson produced no enabled sources")
    return out


def reset_worker(gw: str, token: str, proj_id: int) -> None:
    try:
        out = http("POST", gw, token, f"/v1/projects/{proj_id}/e2b-worker/reset")
        print(f"==> worker reset: {out}")
    except urllib.error.HTTPError as exc:
        print(f"warn: worker reset HTTP {exc.code} (may need manual reset)", file=sys.stderr)


def verify(gw: str, token: str, proj_id: int) -> None:
    cfg = http("GET", gw, token, f"/v1/project/config/{proj_id}")
    skills = [s.get("skillName") for s in (cfg.get("skillsJson") or []) if isinstance(s, dict)]
    tools = cfg.get("allowedToolsJson") or []
    print(f"verify proj={proj_id} role={cfg.get('projectRole')} skills={skills} tools={tools}")
    assert FAQ_SKILL_NAME in skills, skills
    assert "delegate_project_tool" not in tools, tools


def main() -> int:
    repo_env = load_repo_env()
    ap = argparse.ArgumentParser()
    ap.add_argument("--gw", required=True)
    ap.add_argument("--admin-token", default="")
    ap.add_argument("--faq-proj-id", required=True, type=int)
    ap.add_argument("--mind-folder-id")
    ap.add_argument("--kb-sources-json", default="")
    ap.add_argument(
        "--cluster-id",
        default=os.environ.get("CLAW_CLUSTER_ID") or repo_env.get("CLAW_CLUSTER_ID") or DEFAULT_CLUSTER,
    )
    ap.add_argument(
        "--nas-host-mount",
        default=os.environ.get("CLAW_E2B_NAS_HOST_MOUNT")
        or repo_env.get("CLAW_E2B_NAS_HOST_MOUNT")
        or DEFAULT_NAS_ROOT,
    )
    ap.add_argument("--nas-ssh", default=os.environ.get("CLAW_E2B_NAS_SSH", DEFAULT_NAS_SSH))
    ap.add_argument("--project-home", default="")
    ap.add_argument("--skip-sync", action="store_true")
    ap.add_argument("--config-only", action="store_true")
    ap.add_argument("--skip-config", action="store_true")
    args = ap.parse_args()

    gw = args.gw.rstrip("/")
    token = args.admin_token.strip()

    temp_root = Path(tempfile.mkdtemp(prefix=f"faq-kb-{args.faq_proj_id}-"))
    try:
        sources = (
            parse_kb_sources_json(args.kb_sources_json)
            if args.kb_sources_json.strip()
            else [
                {
                    "folderId": str(args.mind_folder_id or "").strip(),
                    "targetRelPath": "en/internal/mind/faq",
                }
            ]
        )
        total = 0
        kb_root = temp_root / "kb"
        for item in sources:
            count = export_mind_folder(
                item["folderId"],
                kb_root,
                item["targetRelPath"],
            )
            print(
                f"==> exported {count} mind docs into {item['targetRelPath']} from folder {item['folderId']}"
            )
            total += count
        if total <= 0:
            raise SystemExit("no documents exported")
        if not args.skip_config:
            put_config_draft(gw, token, args.faq_proj_id)
            commit_activate(gw, token, args.faq_proj_id)
        if not args.config_only and not args.skip_sync:
            project_home = Path(args.project_home).expanduser() if args.project_home.strip() else None
            if project_home is not None:
                sync_kb_tree_local(kb_root, project_home)
            else:
                if not args.nas_ssh.strip():
                    raise SystemExit("sync target missing: provide --project-home or --nas-ssh")
                sync_kb_tree_remote(
                    kb_root,
                    args.faq_proj_id,
                    args.cluster_id,
                    args.nas_host_mount,
                    args.nas_ssh,
                )
            reset_worker(gw, token, args.faq_proj_id)
        if not args.skip_config:
            verify(gw, token, args.faq_proj_id)
    finally:
        shutil.rmtree(temp_root, ignore_errors=True)
    print("Done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
