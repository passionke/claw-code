#!/usr/bin/env python3
# Build e2b cloud sandbox template claw-worker-v1 (E2B SDK). Author: kejiqing
"""Build claw-worker-v1: e2b Beijing base + claw/ttyd from local worker image."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _conn_opts() -> dict[str, str]:
    api_key = _env("E2B_API_KEY") or _env("CLAW_E2B_API_KEY") or _env("ALIYUN_E2B_TOKEN")
    if not api_key:
        print("Set ALIYUN_E2B_TOKEN / CLAW_E2B_API_KEY / E2B_API_KEY", file=sys.stderr)
        sys.exit(1)
    return {
        "api_key": api_key,
        "api_url": _env("E2B_API_URL", "https://api.cn-beijing.e2b.fc.aliyuncs.com"),
        "domain": _env("E2B_DOMAIN", "cn-beijing.e2b.fc.aliyuncs.com"),
    }


def _extract_binaries(worker_image: str, out_dir: Path) -> None:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    if not shutil.which(rt):
        rt = "docker" if shutil.which("docker") else "podman"
    cid = subprocess.check_output(
        [rt, "create", worker_image],
        text=True,
    ).strip()
    try:
        for name in ("claw", "ttyd"):
            dest = out_dir / name
            subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/{name}", str(dest)])
            dest.chmod(0o755)
            print(f"extracted {name} -> {dest} ({dest.stat().st_size} bytes)")
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _build_headers() -> dict[str, str]:
    headers: dict[str, str] = {}
    for env_key, header in (
        ("CLAW_E2B_TEMPLATE_BUILD_MODE", "X-E2B-Template-Build-Mode"),
        ("CLAW_E2B_TEMPLATE_SOURCE_REGISTRY_TYPE", "X-E2B-Template-Source-Registry-Type"),
        ("CLAW_E2B_TEMPLATE_SOURCE_VPC_ID", "X-E2B-Template-Source-VPC-ID"),
        ("CLAW_E2B_TEMPLATE_SOURCE_VSWITCH_IDS", "X-E2B-Template-Source-VSwitch-IDs"),
        ("CLAW_E2B_TEMPLATE_SOURCE_SECURITY_GROUP_ID", "X-E2B-Template-Source-Security-Group-ID"),
        ("CLAW_E2B_TEMPLATE_SOURCE_USERNAME", "X-E2B-Template-Source-Username"),
        ("CLAW_E2B_TEMPLATE_SOURCE_PASSWORD", "X-E2B-Template-Source-Password"),
        ("CLAW_E2B_TEMPLATE_DEST_IMAGE_REF", "X-E2B-Template-Dest-Image-Ref"),
        ("CLAW_E2B_TEMPLATE_DEST_USERNAME", "X-E2B-Template-Dest-Username"),
        ("CLAW_E2B_TEMPLATE_DEST_PASSWORD", "X-E2B-Template-Dest-Password"),
        (
            "CLAW_E2B_TEMPLATE_SOURCE_ACREE_INSTANCE_ID",
            "X-E2B-Template-Source-ACREE-Instance-ID",
        ),
    ):
        val = _env(env_key)
        if val:
            headers[header] = val
    return headers


def _build_from_dockerfile(
    opts: dict[str, str], template_name: str, cpu: int, mem: int, default_skip_cache: str
) -> object:
    from e2b import Template, default_build_logger

    base = _env(
        "CLAW_E2B_TEMPLATE_BASE_IMAGE",
        "fc-e2b-registry.cn-beijing.cr.aliyuncs.com/runtime/code-interpreter-v1:v0.0.18",
    )
    worker_image = _env(
        "CLAW_E2B_TEMPLATE_WORKER_IMAGE",
        "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/claw-gateway-worker:release-v1.6.12",
    )
    strategy = _env("CLAW_E2B_TEMPLATE_BUILD_STRATEGY", "from_image")
    repo_root = Path(__file__).resolve().parents[2]
    fc_dir = repo_root / "deploy" / "e2b"
    fc_dir.mkdir(parents=True, exist_ok=True)

    if strategy == "dockerfile-http" or any(
        _env(key)
        for key in (
            "CLAW_E2B_TEMPLATE_HTTP_HOST",
            "CLAW_E2B_TEMPLATE_HTTP_PORT",
            "CLAW_E2B_TEMPLATE_HTTP_BASE",
        )
    ):
        raise SystemExit(
            "error: HTTP artifact template builds are forbidden. "
            "Use e2b standard file_context/COPY or from_image build paths."
        )
    _extract_binaries(worker_image, fc_dir)
    dockerfile = f"""\
FROM {base}
USER root
COPY claw /usr/local/bin/claw
COPY ttyd /usr/local/bin/ttyd
RUN chmod +x /usr/local/bin/claw /usr/local/bin/ttyd
USER 1000
"""
    print(f"==> dockerfile build base={base!r} worker={worker_image!r} ctx={fc_dir}")

    builder = Template(file_context_path=fc_dir).from_dockerfile(dockerfile)
    headers = _build_headers()
    if headers:
        print(f"==> headers: {list(headers.keys())}")

    build = Template.build(
        builder,
        name=template_name,
        alias=template_name,
        cpu_count=cpu,
        memory_mb=mem,
        skip_cache=_env("CLAW_E2B_TEMPLATE_SKIP_CACHE", default_skip_cache)
        not in ("0", "false", "no"),
        on_build_logs=default_build_logger(),
        headers=headers or None,
        **opts,
    )
    return build


def _build_from_image(
    opts: dict[str, str], template_name: str, cpu: int, mem: int, default_skip_cache: str
) -> object:
    from e2b import Template, default_build_logger

    from_image = _env("CLAW_E2B_TEMPLATE_FROM_IMAGE")
    if not from_image:
        print("CLAW_E2B_TEMPLATE_FROM_IMAGE required for from_image mode", file=sys.stderr)
        sys.exit(1)
    headers = _build_headers()
    print(f"==> from_image build image={from_image!r}")
    return Template.build(
        Template().from_image(from_image),
        name=template_name,
        alias=template_name,
        cpu_count=cpu,
        memory_mb=mem,
        skip_cache=_env("CLAW_E2B_TEMPLATE_SKIP_CACHE", default_skip_cache)
        not in ("0", "false", "no"),
        on_build_logs=default_build_logger(),
        headers=headers or None,
        **opts,
    )


def main() -> int:
    mode = _env("CLAW_E2B_TEMPLATE_BUILD_STRATEGY", "from_image")
    if mode == "dockerfile-http" or any(
        _env(key)
        for key in (
            "CLAW_E2B_TEMPLATE_HTTP_HOST",
            "CLAW_E2B_TEMPLATE_HTTP_PORT",
            "CLAW_E2B_TEMPLATE_HTTP_BASE",
        )
    ):
        print(
            "error: HTTP artifact template builds are forbidden. "
            "Use e2b standard from_image or file_context/COPY build paths.",
            file=sys.stderr,
        )
        return 2

    try:
        from dotenv import load_dotenv
        from e2b_code_interpreter import Sandbox
    except ImportError:
        print("pip install e2b==2.26.0 e2b-code-interpreter python-dotenv", file=sys.stderr)
        return 1

    load_dotenv()
    opts = _conn_opts()
    template_name = _env("CLAW_E2B_TEMPLATE", "claw-worker-v1")
    cpu = int(_env("CLAW_E2B_TEMPLATE_CPU", "2"))
    mem = int(_env("CLAW_E2B_TEMPLATE_MEMORY_MB", "4096"))
    verify = _env("CLAW_E2B_TEMPLATE_SKIP_VERIFY", "0") not in ("1", "true", "yes")
    build_mode = _env("CLAW_E2B_TEMPLATE_BUILD_MODE", "builder")
    default_skip_cache = "0" if build_mode == "builder" else "1"

    if mode == "dockerfile-http":
        print(
            "error: CLAW_E2B_TEMPLATE_BUILD_STRATEGY=dockerfile-http is forbidden; "
            "use from_image or dockerfile with e2b file_context/COPY.",
            file=sys.stderr,
        )
        return 2
    if mode == "dockerfile":
        build = _build_from_dockerfile(opts, template_name, cpu, mem, default_skip_cache)
    elif mode == "from_image":
        build = _build_from_image(opts, template_name, cpu, mem, default_skip_cache)
    else:
        print(f"unknown CLAW_E2B_TEMPLATE_BUILD_STRATEGY={mode!r}", file=sys.stderr)
        return 1

    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")

    if not verify:
        return 0

    print("==> verify: create sandbox + check ttyd/claw")
    sandbox = Sandbox.create(template=template_name, timeout=900, **opts)
    try:
        print(f"sandbox_id: {sandbox.sandbox_id}")
        checks = (
            "command -v ttyd",
            "command -v claw",
            "python3 -c \"print('claw-worker-v1 ok')\"",
        )
        for cmd in checks:
            r = sandbox.commands.run(cmd, timeout=120)
            print(f"$ {cmd} -> exit={r.exit_code} stdout={(r.stdout or '').strip()!r}")
            if r.exit_code not in (0, None):
                if r.stderr:
                    print(f"  stderr={(r.stderr or '').strip()!r}", file=sys.stderr)
                return r.exit_code or 1
    finally:
        sandbox.kill()
        print("sandbox killed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
