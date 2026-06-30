#!/usr/bin/env python3
"""Patch claudeMd §0 Skill loading: pool worker paths (/claw_ds). Author: kejiqing

**仅预发默认**。预发验收通过后再动线上：
  python3 patch_claude_skill_loading.py

线上须显式 opt-in（禁止与预发同批）：
  ALLOW_PROD=1 GATEWAY=http://10.200.2.171:18088 PROJ_ID=10 python3 patch_claude_skill_loading.py
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request

GATEWAY = os.environ.get("GATEWAY", "http://192.168.9.252:18088").rstrip("/")
PROJ_ID = int(os.environ.get("PROJ_ID", "27"))

SKILL_SECTION_START = "### 第 0 步：Skill 强制检索与装载（最高优先级）"
SKILL_SECTION_END = "## 用户可见的进度汇报"

NEW_SKILL_SECTION = """### 第 0 步：Skill 强制检索与装载（最高优先级）

> ⚠️ META-COGNITION: All Chinese text below is instructional metadata. Never output it.

#### Pool worker 路径（必读，否则 glob 恒为 0）

| 路径 | 作用 |
|------|------|
| `/claw_host_root` | 当前 **cwd**，可写 session 区；**不含** Admin 下发的 skill |
| `/claw_ds` | **只读** project 配置；**所有 SKILL.md 在这里** |

Skill 固定路径：`/claw_ds/.claw/skills/<skillName>/SKILL.md`（`<skillName>` = 目录名）。

#### 步骤（任意 `mcp__*` 之前必须完成）

1. **列举** — 必须带 `path`，禁止只在 cwd 下搜：

   `glob_search` 入参：`{"path": "/claw_ds", "pattern": "**/SKILL.md"}`

   - **禁止**仅 `{ "pattern": "**/SKILL.md" }`（会在 `/claw_host_root` 得到 0 个，**不等于**没有 skill）。
   - `/claw_ds` 下 `numFiles > 0` 时，**禁止**声称「无可用 skill」。

2. **匹配** — 对照 glob 返回的目录名，判断与本题相关的 skill（仅内部推理，不写进用户可见正文）。

3. **载入** — 每个命中 skill **必须**执行其一（优先 `Skill` 工具）：
   - `Skill("<skillName>")`，例：`Skill("dish-name-fuzzy-sales-protocol")`
   - 或 `read_file`：`/claw_ds/.claw/skills/<skillName>/SKILL.md`

4. **放行** — 仅当已对 `/claw_ds` 执行 glob 且 `numFiles == 0`，才可跳过 skill；否则 **禁止**调用任何 MCP。

#### 菜品 / เมนู / จาน 销量类

用户问**具体菜品**在指定日期卖了多少份/几盘/几จาน，且口述菜名可能与 POS 登记名不一致时：若 glob 列表含 `dish-name-fuzzy-sales-protocol`，**必须**先 `Skill("dish-name-fuzzy-sales-protocol")` 再 MCP。

- 本步列举与载入**不得**写入用户可见回复（见「回复风格」）。

"""

OLD_CHECKLIST = (
    "- **已完成「Skill 清单与选型」：在任意 MCP 调用之前已通过 glob 搜索 `**/SKILL.md` "
    "载入 Skill 清单并完成匹配判定；若存在匹配 Skill，则已先完成 `Skill` 载入**"
)

NEW_CHECKLIST = (
    "- **已完成「Skill 清单与选型」：在任意 MCP 之前已对 `/claw_ds` 执行 "
    "`glob_search(path=/claw_ds, pattern=**/SKILL.md)`；若存在匹配 skill，"
    "已 `Skill(name)` 或 `read_file(/claw_ds/.claw/skills/.../SKILL.md)` 载入**"
)


def req(method: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body, ensure_ascii=False).encode() if body is not None else None
    r = urllib.request.Request(
        f"{GATEWAY}{path}",
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"{method} {path} -> HTTP {e.code}: {e.read().decode()[:800]}")


def patch_claude_md(md: str) -> tuple[str, list[str]]:
    applied: list[str] = []
    start = md.find(SKILL_SECTION_START)
    end = md.find(SKILL_SECTION_END, start)
    if start < 0 or end < 0:
        raise SystemExit(f"skill section anchors not found (start={start}, end={end})")
    old_block = md[start:end]
    if "/claw_ds" in old_block and "禁止**仅" in old_block:
        print("skill section already patched (/claw_ds present)")
    else:
        md = md[:start] + NEW_SKILL_SECTION + "\n" + md[end:]
        applied.append("skill_section_pool_paths")

    if OLD_CHECKLIST in md:
        md = md.replace(OLD_CHECKLIST, NEW_CHECKLIST, 1)
        applied.append("checklist_glob_path")
    elif NEW_CHECKLIST in md:
        pass
    else:
        print("warn: checklist line not found, skipped")

    return md, applied


def push_config(cur: dict, md: str, note: str) -> str:
    payload = {
        "rulesJson": cur["rulesJson"],
        "mcpServersJson": cur["mcpServersJson"],
        "skillsSourcesJson": cur.get("skillsSourcesJson") or [],
        "skillsJson": cur["skillsJson"],
        "allowedToolsJson": cur["allowedToolsJson"],
        "claudeMd": md,
        "languagePipelineJson": cur.get("languagePipelineJson"),
    }
    for k in (
        "gitSyncJson",
        "solvePreflightJson",
        "solveOrchestrationJson",
        "extraSessionFieldsJson",
        "promptLimitsJson",
        "workerProfileJson",
    ):
        if k in cur and cur[k] is not None:
            payload[k] = cur[k]

    req("PUT", f"{GATEWAY}/v1/project/config/{PROJ_ID}", payload)
    commit = req(
        "POST",
        f"{GATEWAY}/v1/project/config/{PROJ_ID}/versions/commit",
        {"note": note},
    )
    rev = commit["savedContentRev"]
    act = req("POST", f"{GATEWAY}/v1/project/config/{PROJ_ID}/versions/{rev}/activate")
    print("activated", act.get("activated"), "rev", rev)
    return rev


def main() -> int:
    if PROJ_ID == 10 and os.environ.get("ALLOW_PROD") != "1":
        raise SystemExit(
            "refusing prod proj 10 without ALLOW_PROD=1; patch pre 27 first, verify, then opt in"
        )
    cur = req("GET", f"{GATEWAY}/v1/project/config/{PROJ_ID}")
    md = cur.get("claudeMd") or ""
    patched, applied = patch_claude_md(md)
    if not applied:
        print("no changes")
        return 0
    print(f"gateway={GATEWAY} proj={PROJ_ID}")
    print(f"claudeMd chars: {len(md)} -> {len(patched)}")
    print("applied:", ", ".join(applied))
    rev = push_config(
        cur,
        patched,
        "Skill §0: pool /claw_ds glob+load paths (kejiqing)",
    )
    verify = req("GET", f"{GATEWAY}/v1/project/config/{PROJ_ID}")
    vmd = verify.get("claudeMd") or ""
    print("verify /claw_ds in §0:", "/claw_ds" in vmd[vmd.find(SKILL_SECTION_START) : vmd.find(SKILL_SECTION_END)])
    print("stableContentRev", verify.get("stableContentRev"), "saved", rev)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
