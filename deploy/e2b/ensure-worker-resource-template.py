#!/usr/bin/env python3
"""Ensure fingerprint-keyed e2b worker template for project resources. Author: kejiqing

Builds a shared SKU template (not per-project):
  claw-worker-r-cpu{N}-mem{M}
  claw-worker-relaxed-r-cpu{N}-mem{M}

CPU ceil → 1..=16; memory ceil → 256 MiB steps, max 16384 MiB.
Optional Gateway bind writes worker_profile_json.resourceTemplate.

Examples:
  CLAW_E2B_RESOURCE_CPU=4 CLAW_E2B_RESOURCE_MEMORY_MB=8192 \\
    python3 deploy/e2b/ensure-worker-resource-template.py

  python3 deploy/e2b/ensure-worker-resource-template.py --cpu 1.1 --memory-mb 257 \\
    --bind-proj 42
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import math
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from types import ModuleType
from typing import Any

_E2B_DIR = Path(__file__).resolve().parent
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))


def _load_hyphen_module(mod_name: str, filename: str) -> ModuleType:
    """Load deploy/e2b/*.py scripts whose filenames use hyphens (not importable). Author: kejiqing"""
    path = _E2B_DIR / filename
    spec = importlib.util.spec_from_file_location(mod_name, path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"error: cannot load {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

from e2b_template_build import build_template_with_retry
from e2b_template_registry import (
    apply_template_skip_cache_force,
    load_repo_dotenv,
    log_debian_base_resolution,
)

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)

CPU_MIN, CPU_MAX = 1, 16
MEMORY_QUANTUM_MB = 256
MEMORY_MIN_MB = MEMORY_QUANTUM_MB
MEMORY_MAX_MB = 16 * 1024

WORKER_START_CMD = "/usr/local/bin/claw-worker-start"
WORKER_READY_CMD = "/usr/local/bin/claw-worker-ready"
RELAXED_START_CMD = "/usr/local/bin/claw-worker-relaxed-start"
RELAXED_READY_CMD = "/usr/local/bin/claw-worker-relaxed-ready"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def normalize_cpu(raw: float) -> int:
    if not math.isfinite(raw) or raw <= 0:
        raise ValueError(f"cpuCount must be a finite number > 0; got {raw!r}")
    n = int(math.ceil(raw))
    if not (CPU_MIN <= n <= CPU_MAX):
        raise ValueError(f"cpuCount must be {CPU_MIN}..={CPU_MAX} after ceil; got {raw} → {n}")
    return n


def normalize_memory_mb(raw: float) -> int:
    if not math.isfinite(raw) or raw <= 0:
        raise ValueError(f"memoryMb must be a finite number > 0; got {raw!r}")
    n = int(math.ceil(raw / MEMORY_QUANTUM_MB) * MEMORY_QUANTUM_MB)
    if not (MEMORY_MIN_MB <= n <= MEMORY_MAX_MB):
        raise ValueError(
            f"memoryMb must be {MEMORY_MIN_MB}..={MEMORY_MAX_MB} "
            f"(quantum {MEMORY_QUANTUM_MB}); got {raw} → {n}"
        )
    return n


def fingerprint(cpu: int, memory_mb: int) -> str:
    return f"cpu{cpu}-mem{memory_mb}"


def resource_alias(fp: str, relaxed: bool) -> str:
    return f"claw-worker-relaxed-r-{fp}" if relaxed else f"claw-worker-r-{fp}"


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _http_json(
    method: str, url: str, body: dict[str, Any] | None, token: str
) -> dict[str, Any]:
    data = None if body is None else json.dumps(body).encode("utf-8")
    headers = {"Accept": "application/json", "Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            raw = resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8", errors="replace")
        raise SystemExit(f"HTTP {e.code} {url}: {err_body}") from e
    return json.loads(raw) if raw else {}


def bind_project(
    gateway_base: str,
    proj_id: int,
    token: str,
    resources: dict[str, int],
    template_id: str,
    build_id: str,
    fp: str,
) -> None:
    base = gateway_base.rstrip("/")
    cfg = _http_json("GET", f"{base}/v1/project/config/{proj_id}", None, token)
    wp = dict(cfg.get("workerProfileJson") or {"mode": "strict"})
    wp["resources"] = resources
    wp["resourceTemplate"] = {
        "templateId": template_id,
        "buildId": build_id,
        "fingerprint": fp,
    }
    put_body = {
        "contentRev": cfg.get("contentRev") or "",
        "rulesJson": cfg.get("rulesJson") or [],
        "mcpServersJson": cfg.get("mcpServersJson") or {},
        "skillsSourcesJson": cfg.get("skillsSourcesJson") or [],
        "skillsJson": cfg.get("skillsJson") or [],
        "allowedToolsJson": cfg.get("allowedToolsJson"),
        "claudeMd": cfg.get("claudeMd"),
        "workerProfileJson": wp,
    }
    _http_json("PUT", f"{base}/v1/project/config/{proj_id}", put_body, token)
    print(f"OK: bound proj {proj_id} fingerprint={fp} templateId={template_id}")


def _build_e2b_image(relaxed: bool) -> str:
    if relaxed:
        relaxed_mod = _load_hyphen_module(
            "claw_worker_relaxed_build", "build-claw-worker-relaxed-selfhosted.py"
        )
        source = _env("CLAW_E2B_WORKER_RELAXED_IMAGE") or relaxed_mod._worker_base_image()
        if not relaxed_mod._is_protocol_relaxed(source):
            raise SystemExit(
                "error: relaxed resource template requires …/claw-worker-base-relaxed:<tag>"
            )
        return relaxed_mod._build_e2b_relaxed_image(source)
    strict_mod = _load_hyphen_module(
        "claw_worker_strict_build", "build-claw-worker-selfhosted.py"
    )
    source = strict_mod._worker_base_image()
    if not strict_mod._is_worker_series_source(source):
        raise SystemExit(
            "error: strict resource template requires …/claw-worker-base:<tag>"
        )
    return strict_mod._build_e2b_worker_image(source)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cpu", type=float, default=None)
    parser.add_argument("--memory-mb", type=float, default=None)
    parser.add_argument("--relaxed", action="store_true")
    parser.add_argument("--bind-proj", type=int, default=None)
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()

    cpu_raw = args.cpu if args.cpu is not None else float(_env("CLAW_E2B_RESOURCE_CPU") or "0")
    mem_raw = (
        args.memory_mb
        if args.memory_mb is not None
        else float(_env("CLAW_E2B_RESOURCE_MEMORY_MB") or "0")
    )
    if cpu_raw <= 0 or mem_raw <= 0:
        print(
            "error: set --cpu/--memory-mb or CLAW_E2B_RESOURCE_CPU / CLAW_E2B_RESOURCE_MEMORY_MB",
            file=sys.stderr,
        )
        return 2

    relaxed = args.relaxed or _env("CLAW_E2B_RESOURCE_RELAXED") in ("1", "true", "yes")
    try:
        cpu = normalize_cpu(cpu_raw)
        memory_mb = normalize_memory_mb(mem_raw)
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2

    fp = fingerprint(cpu, memory_mb)
    alias = resource_alias(fp, relaxed)
    resources = {"cpuCount": cpu, "memoryMb": memory_mb}
    print(f"==> normalized resources={resources} fingerprint={fp} alias={alias}")

    if args.skip_build:
        print(json.dumps({"resources": resources, "fingerprint": fp, "alias": alias}, ensure_ascii=False))
        return 0

    opts = _conn_opts()
    log_debian_base_resolution(api_url=opts["api_url"])
    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )

    from e2b import Template, default_build_logger

    e2b_image = _build_e2b_image(relaxed)
    start_cmd = RELAXED_START_CMD if relaxed else WORKER_START_CMD
    ready_cmd = RELAXED_READY_CMD if relaxed else WORKER_READY_CMD
    skip_cache = _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")

    print(f"==> Template.build from_image={e2b_image!r} cpu={cpu} memory_mb={memory_mb}")
    template = Template().from_image(e2b_image)
    template = template.set_start_cmd(start_cmd, ready_cmd)
    apply_template_skip_cache_force(template, skip_cache)
    build = build_template_with_retry(
        Template.build,
        label=alias,
        template=template,
        alias=alias,
        cpu_count=cpu,
        memory_mb=memory_mb,
        skip_cache=skip_cache,
        on_build_logs=default_build_logger(),
        **opts,
    )

    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    out = {
        "resources": resources,
        "fingerprint": fp,
        "alias": alias,
        "resourceTemplate": {
            "templateId": build.template_id,
            "buildId": build.build_id,
            "fingerprint": fp,
        },
    }
    print(json.dumps(out, ensure_ascii=False))

    proj = args.bind_proj
    if proj is None:
        raw_proj = _env("CLAW_PROJ_ID")
        proj = int(raw_proj) if raw_proj.isdigit() else None
    gateway = _env("CLAW_GATEWAY_BASE")
    token = _env("CLAW_ADMIN_TOKEN")
    if proj and gateway:
        bind_project(
            gateway,
            proj,
            token,
            resources,
            build.template_id,
            build.build_id,
            fp,
        )
    else:
        print(
            "hint: pass --bind-proj and CLAW_GATEWAY_BASE (+ CLAW_ADMIN_TOKEN) "
            "to write resourceTemplate onto the project"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
