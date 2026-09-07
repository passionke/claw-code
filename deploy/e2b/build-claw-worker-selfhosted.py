#!/usr/bin/env python3
"""Build claw-worker template on self-hosted e2bserver. Author: kejiqing"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

_E2B_DIR = Path(__file__).resolve().parent
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))
from e2b_pg_settings import merge_settings_json_key
from e2b_template_registry import (
    apply_template_skip_cache_force,
    load_repo_dotenv,
    log_debian_base_resolution,
    template_apt_prepare_prefix,
    template_debian_apt_mirror,
    template_debian_base_image,
    template_gateway_worker_image,
)
from e2b_template_build import build_template_with_retry
from registry_extract import extract_file_from_image

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)

DOCKERFILE_E2B = _E2B_DIR / "Dockerfile.claw-worker-selfhosted"

WORKER_START_CMD = "/usr/local/bin/claw-worker-start"
WORKER_READY_CMD = "/usr/local/bin/claw-worker-ready"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _worker_base_image() -> str:
    return template_gateway_worker_image()


def _e2b_worker_image_tag(worker_image: str) -> str:
    explicit = _env("CLAW_E2B_WORKER_E2B_IMAGE")
    if explicit:
        return explicit
    if ":" not in worker_image:
        return f"{worker_image}-debian"
    registry_repo, tag = worker_image.rsplit(":", 1)
    if "/" in registry_repo:
        registry, _repo = registry_repo.rsplit("/", 1)
        return f"{registry}/debian-bookworm-claw-worker:{tag}"
    return f"{registry_repo}/debian-bookworm-claw-worker:{tag}"


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    if rt == "auto":
        for candidate in ("podman", "docker"):
            if shutil.which(candidate):
                return candidate
        return "podman"
    return rt


def _template_platform() -> str:
    return _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")


def _linux_arch_from_platform(platform: str) -> str:
    p = platform.strip().lower()
    if p in ("linux/arm64", "arm64", "aarch64"):
        return "arm64"
    if p in ("linux/amd64", "amd64", "x86_64"):
        return "amd64"
    raise SystemExit(f"error: unsupported CLAW_E2B_TEMPLATE_PLATFORM={platform!r}")


def _elf_arch_ok(probe: str, arch: str) -> bool:
    if "ELF" not in probe:
        return False
    if arch == "arm64":
        return "aarch64" in probe or "ARM" in probe
    if arch == "amd64":
        return "x86-64" in probe or "x86_64" in probe
    return False


def _elf_probe(path: Path) -> str:
    """Describe ELF arch; works without the `file` binary (gateway image). Author: kejiqing"""
    if shutil.which("file"):
        return subprocess.check_output(["file", "-b", str(path)], text=True).strip()
    data = path.read_bytes()[:20]
    if len(data) < 20 or data[:4] != b"\x7fELF":
        return "not ELF"
    # e_machine at offset 18 (little-endian)
    machine = int.from_bytes(data[18:20], "little")
    if machine == 0x3E:
        return "ELF 64-bit LSB pie executable, x86-64"
    if machine == 0xB7:
        return "ELF 64-bit LSB pie executable, ARM aarch64"
    return f"ELF machine=0x{machine:x}"


def _acr_registry_host(image_ref: str) -> str:
    if "/" not in image_ref:
        return ""
    return image_ref.split("/", 1)[0]


def _acr_login_if_needed(image_ref: str) -> None:
    registry = _acr_registry_host(image_ref)
    if not registry:
        return
    user = _env("ACR_USERNAME") or _env("ACR_USER")
    password = _env("ACR_PASSWORD") or _env("ACR_PASSWORK")
    if not user or not password:
        return
    if subprocess.call(
        [shutil.which(_container_runtime()) or "podman", "login", registry, "--get-login"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) == 0:
        return
    rt = _container_runtime()
    print(f"==> {rt} login {registry!r} (ACR_USERNAME)")
    subprocess.run(
        [rt, "login", registry, "-u", user, "--password-stdin"],
        input=password.encode(),
        check=True,
    )


def _build_e2b_worker_image(worker_image: str) -> str:
    """Layer e2b runtime on CI worker locally; 250 pulls the pushed image (no 24MB SDK upload)."""
    rt = _container_runtime()
    platform = _template_platform()
    e2b_image = _e2b_worker_image_tag(worker_image)
    if not DOCKERFILE_E2B.is_file():
        raise SystemExit(f"error: missing {DOCKERFILE_E2B}")

    apt_mirror = template_debian_apt_mirror()
    if apt_mirror:
        print(f"==> debian apt mirror (worker layer): {apt_mirror!r}")

    print(
        f"==> {rt} build e2b worker image {e2b_image!r} "
        f"(FROM {worker_image!r}, {platform}); 250 will pull this image"
    )
    build_args = [
        rt,
        "build",
        "-f",
        str(DOCKERFILE_E2B),
        "--build-arg",
        f"WORKER_BASE_IMAGE={worker_image}",
        "--build-arg",
        f"DEBIAN_APT_MIRROR={apt_mirror}",
        "--platform",
        platform,
        "-t",
        e2b_image,
        str(_E2B_DIR),
    ]
    subprocess.check_call(build_args)
    if _env("CLAW_E2B_WORKER_E2B_PUSH", "1") not in ("0", "false", "no"):
        _acr_login_if_needed(e2b_image)
        print(f"==> {rt} push {e2b_image!r}")
        subprocess.check_call([rt, "push", e2b_image])
    return e2b_image


def _worker_runtime_install_run() -> str:
    """Same packages/scripts as Dockerfile.claw-worker-selfhosted (runs on e2b build host)."""
    apt = template_apt_prepare_prefix()
    return (
        f"{apt}apt-get update && apt-get install -y --no-install-recommends "
        "nfs-common ca-certificates sudo "
        "&& echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' "
        "> /etc/sudoers.d/claw-nfs && chmod 440 /etc/sudoers.d/claw-nfs "
        "&& rm -rf /var/lib/apt/lists/* "
        "&& printf '%s\\n' '#!/bin/sh' 'set -eu' 'exec sleep infinity' > /usr/local/bin/claw-worker-start "
        "&& printf '%s\\n' '#!/bin/sh' 'command -v claw >/dev/null 2>&1' > /usr/local/bin/claw-worker-ready "
        "&& chmod +x /usr/local/bin/claw-worker-start /usr/local/bin/claw-worker-ready"
    )


def _worker_start_ready_install() -> str:
    return r"""RUN printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        'exec sleep infinity' \
        > /usr/local/bin/claw-worker-start \
    && printf '%s\n' \
        '#!/bin/sh' \
        'command -v claw >/dev/null 2>&1' \
        > /usr/local/bin/claw-worker-ready \
    && chmod +x /usr/local/bin/claw-worker-start /usr/local/bin/claw-worker-ready
"""


def _dockerfile_debian_copy() -> str:
    debian = template_debian_base_image()
    apt = template_apt_prepare_prefix()
    return f"""FROM {debian}
RUN {apt}apt-get update && apt-get install -y --no-install-recommends \\
    nfs-common ca-certificates sudo \\
    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \\
    && chmod 440 /etc/sudoers.d/claw-nfs \\
    && rm -rf /var/lib/apt/lists/*
COPY claw.bin /usr/local/bin/claw
RUN chmod +x /usr/local/bin/claw
{_worker_start_ready_install()}"""


def _stage_from_worker_tag(staging: Path, worker_image: str) -> None:
    """Pull CI worker tag and stage claw only (strict solve worker)."""
    rt = _container_runtime()
    platform = _template_platform()
    arch = _linux_arch_from_platform(platform)
    print(f"==> stage binaries from worker tag {worker_image!r} via {rt} ({platform})")
    subprocess.check_call([rt, "pull", "--platform", platform, worker_image])
    cid = subprocess.check_output(
        [rt, "create", "--platform", platform, worker_image],
        text=True,
    ).strip()
    try:
        for name in ("claw",):
            dest = staging / name
            subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/{name}", str(dest)])
            dest.chmod(0o755)
            probe = subprocess.check_output(["file", "-b", str(dest)], text=True).strip()
            print(f"  {name}: {probe}")
            if not _elf_arch_ok(probe, arch):
                raise SystemExit(f"error: {name} is not linux/{arch} ({probe})")
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _forbidden_http() -> bool:
    if _env("CLAW_E2B_TEMPLATE_BUILD_STRATEGY") == "http":
        return True
    return any(
        _env(key)
        for key in (
            "CLAW_E2B_TEMPLATE_HTTP_BASE",
            "CLAW_E2B_TEMPLATE_HTTP_BIND",
            "CLAW_E2B_TEMPLATE_HTTP_HOST",
            "CLAW_E2B_TEMPLATE_HTTP_PORT",
        )
    )


def main() -> int:
    opts = _conn_opts()
    log_debian_base_resolution(api_url=opts["api_url"])
    alias = _env("CLAW_E2B_TEMPLATE", "claw-worker")
    strategy = _env("CLAW_E2B_TEMPLATE_BUILD_STRATEGY", "from_image")
    verify = _env("CLAW_E2B_TEMPLATE_SKIP_VERIFY", "0") not in ("1", "true", "yes")

    if _forbidden_http():
        print(
            "error: HTTP artifact template builds are forbidden. "
            "Use CLAW_E2B_TEMPLATE_BUILD_STRATEGY=from_image with a CI worker image tag.",
            file=sys.stderr,
        )
        return 2

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    skip_cache = _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")

    # e2bserver rejects custom ACR apps (claw-gateway-worker) as "must be Debian-based"
    # even when the image IS bookworm. Bootstrap therefore uses debian: + COPY claw.
    # Author: kejiqing
    skip_local = _env("CLAW_E2B_WORKER_SKIP_LOCAL_BUILD") in ("1", "true", "yes")
    if skip_local and strategy == "from_image":
        print(
            "==> SKIP_LOCAL_BUILD: switch from_image(CI worker) → debian + COPY claw "
            "(e2b rejects claw-gateway-worker as non-Debian base)",
            file=sys.stderr,
        )
        strategy = "copy"

    if strategy == "from_image":
        worker_image = _worker_base_image()
        e2b_image = _build_e2b_worker_image(worker_image)
        print(f"==> e2b Template.build from_image={e2b_image!r} (e2b host pulls from registry)")
        template = Template().from_image(e2b_image)
        template = template.set_start_cmd(WORKER_START_CMD, WORKER_READY_CMD)
        apply_template_skip_cache_force(template, skip_cache)
        build = build_template_with_retry(
            Template.build,
            label=alias,
            template=template,
            alias=alias,
            skip_cache=skip_cache,
            on_build_logs=default_build_logger(),
            **opts,
        )
    elif strategy == "copy":
        with tempfile.TemporaryDirectory(prefix="claw-e2b-tpl-") as tmp:
            staging = Path(tmp)
            copy_dir = _env("CLAW_E2B_TEMPLATE_COPY_DIR")
            claw_bin = staging / "claw.bin"
            if copy_dir and (Path(copy_dir) / "claw").is_file():
                src = Path(copy_dir) / "claw"
                claw_bin.write_bytes(src.read_bytes())
                claw_bin.chmod(0o755)
                print(f"==> copy build ctx from COPY_DIR {src}")
            else:
                worker_image = _worker_base_image()
                # Prefer registry HTTP extract (works inside gateway; no nested podman). Author: kejiqing
                try:
                    extract_file_from_image(
                        worker_image,
                        "/usr/local/bin/claw",
                        claw_bin,
                        platform=_template_platform(),
                    )
                except Exception as exc:  # noqa: BLE001
                    print(
                        f"warn: registry extract failed ({exc}); falling back to podman stage",
                        file=sys.stderr,
                    )
                    _stage_from_worker_tag(staging, worker_image)
                    claw_bin.write_bytes((staging / "claw").read_bytes())
                    claw_bin.chmod(0o755)
            arch = _linux_arch_from_platform(_template_platform())
            probe = _elf_probe(claw_bin)
            print(f"  claw: {probe}")
            if not _elf_arch_ok(probe, arch):
                raise SystemExit(f"error: claw is not linux/{arch} ({probe})")
            dockerfile_path = staging / "Dockerfile"
            dockerfile_path.write_text(_dockerfile_debian_copy(), encoding="utf-8")
            print(f"==> e2b Template.build debian+COPY claw (base={template_debian_base_image()!r})")
            template = (
                Template(file_context_path=str(staging))
                .from_dockerfile(str(dockerfile_path))
                .set_start_cmd(WORKER_START_CMD, WORKER_READY_CMD)
            )
            apply_template_skip_cache_force(template, skip_cache)
            build = build_template_with_retry(
                Template.build,
                label=alias,
                template=template,
                alias=alias,
                skip_cache=skip_cache,
                on_build_logs=default_build_logger(),
                **opts,
            )
    else:
        print(f"unknown CLAW_E2B_TEMPLATE_BUILD_STRATEGY={strategy!r}", file=sys.stderr)
        return 1

    now_ms = int(time.time() * 1000)
    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    skip_pg = os.environ.get("CLAW_E2B_SKIP_WORKER_PG_PERSIST", "").strip().lower() in (
        "1",
        "true",
        "yes",
    )
    if skip_pg:
        print(
            f"==> skip PG e2bWorker.templateId (alias {alias!r} only; strict PG unchanged)",
            file=sys.stderr,
        )
    else:
        try:
            merge_settings_json_key(
                "e2bWorker",
                {
                    "templateId": build.template_id,
                    "buildId": build.build_id,
                    "alias": alias,
                    "updatedAtMs": now_ms,
                },
                now_ms=now_ms,
            )
            print(
                f"==> persisted e2bWorker.templateId={build.template_id!r} "
                f"buildId={build.build_id!r} to PG"
            )
        except Exception as exc:  # noqa: BLE001
            print(f"warn: skip PG e2bWorker.templateId persist: {exc}", file=sys.stderr)

    print(f"OK: template {alias!r} ({build.template_id}) ready on {opts['api_url']}")
    print(
        "hint: rebuild only updates PG; new build is used after gateway restart, "
        "manual worker reset, or when the sandbox is dead"
    )
    if verify:
        return _verify(build.template_id, opts)
    return 0


def _verify(template: str, opts: dict[str, str]) -> int:
    from e2b import Sandbox

    print(f"==> verify: create sandbox template={template!r} + check claw")
    sandbox = Sandbox.create(template, timeout=900, **opts)
    try:
        print(f"sandbox_id: {sandbox.sandbox_id}")
        for cmd in (
            "command -v claw",
            "test -x /usr/local/bin/claw-worker-start",
            "test -x /usr/local/bin/claw-worker-ready",
        ):
            r = sandbox.commands.run(cmd, timeout=120)
            out = (r.stdout or "").strip()
            print(f"$ {cmd} -> exit={r.exit_code} stdout={out!r}")
            if r.exit_code not in (0, None) and cmd.startswith("command -v"):
                return r.exit_code or 1
    finally:
        sandbox.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
