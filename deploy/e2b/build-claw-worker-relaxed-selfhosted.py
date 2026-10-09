#!/usr/bin/env python3
"""Build claw-worker-relaxed on self-hosted e2b.
Thin path: from_image claw-worker-base-relaxed (CLI via worker.init).
Legacy: debian + COPY claw. Author: kejiqing
"""
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
from e2b_template_content_hash import digest_parts, digest_tree, try_skip_unchanged
from registry_extract import extract_file_from_image, try_image_digest

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)

DOCKERFILE_E2B_RELAXED = _E2B_DIR / "Dockerfile.claw-worker-relaxed-selfhosted"
RELAXED_START_CMD = "/usr/local/bin/claw-worker-relaxed-start"
RELAXED_READY_CMD = "/usr/local/bin/claw-worker-relaxed-ready"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _truthy(name: str) -> bool:
    return _env(name) in ("1", "true", "yes")


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    if rt == "auto":
        for candidate in ("podman", "docker"):
            if shutil.which(candidate):
                return candidate
        return "podman"
    return rt


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _stage_worker_bins(staging: Path, worker_image: str) -> None:
    """Extract claw from a local or remote worker image (pull only if missing)."""
    rt = _container_runtime()
    platform = _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")
    if subprocess.call([rt, "image", "inspect", worker_image], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) != 0:
        print(f"==> pull worker image {worker_image!r}")
        subprocess.check_call([rt, "pull", "--platform", platform, worker_image])
    else:
        print(f"==> use local worker image {worker_image!r}")
    cid = subprocess.check_output([rt, "create", "--platform", platform, worker_image], text=True).strip()
    try:
        for name in ("claw",):
            dest = staging / name
            subprocess.check_call([rt, "cp", f"{cid}:/usr/local/bin/{name}", str(dest)])
            dest.chmod(0o755)
            probe = subprocess.check_output(["file", "-b", str(dest)], text=True).strip()
            print(f"  {name}: {probe}")
    finally:
        subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _worker_base_image() -> str:
    return template_gateway_worker_image()


def _stage_claw_into(staging: Path) -> None:
    """Prefer staged COPY_DIR claw (e2b-worker-deploy); else extract from worker image. Author: kejiqing"""
    copy_dir = _env("CLAW_E2B_TEMPLATE_COPY_DIR")
    if copy_dir:
        src = Path(copy_dir) / "claw"
        if not src.is_file():
            raise SystemExit(
                f"error: CLAW_E2B_TEMPLATE_COPY_DIR={copy_dir!r} missing claw "
                "(run gateway.sh e2b-worker-deploy stage first)"
            )
        print(f"==> stage claw from COPY_DIR {src}")
        shutil.copy2(src, staging / "claw.bin")
        (staging / "claw.bin").chmod(0o755)
        probe = subprocess.check_output(["file", "-b", str(staging / "claw.bin")], text=True).strip()
        print(f"  claw: {probe}")
        return

    worker_image = _worker_base_image()
    bin_dir = staging / "bins"
    bin_dir.mkdir(exist_ok=True)
    print(f"==> stage claw from {worker_image!r}")
    _stage_worker_bins(bin_dir, worker_image)
    shutil.copy2(bin_dir / "claw", staging / "claw.bin")
    (staging / "claw.bin").chmod(0o755)


def _stage_claw_from_registry(staging: Path, image: str) -> None:
    claw_bin = staging / "claw.bin"
    extract_file_from_image(
        image,
        "/usr/local/bin/claw",
        claw_bin,
        platform=_env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64"),
    )


def _relaxed_start_ready_install() -> str:
    """Same shape as strict worker: sleep infinity + claw present. Author: kejiqing"""
    return r"""RUN printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        'exec sleep infinity' \
        > /usr/local/bin/claw-worker-relaxed-start \
    && printf '%s\n' \
        '#!/bin/sh' \
        'command -v claw >/dev/null 2>&1' \
        > /usr/local/bin/claw-worker-relaxed-ready \
    && chmod +x /usr/local/bin/claw-worker-relaxed-start /usr/local/bin/claw-worker-relaxed-ready
"""


def _relaxed_dockerfile() -> str:
    debian = template_debian_base_image()
    apt = template_apt_prepare_prefix()
    return (
        f"FROM {debian}\n"
        # Relaxed package set = CI claw-gateway-worker-relaxed tools (no OVS). Author: kejiqing
        f"RUN {apt}apt-get update && apt-get install -y --no-install-recommends \\\n"
        "    nfs-common ca-certificates sudo \\\n"
        "    curl git python3 python3-pip \\\n"
        "    && ln -sf /usr/bin/pip3 /usr/local/bin/pip \\\n"
        "    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \\\n"
        "    && chmod 440 /etc/sudoers.d/claw-nfs \\\n"
        "    && rm -rf /var/lib/apt/lists/*\n"
        "COPY claw.bin /usr/local/bin/claw\n"
        "RUN chmod +x /usr/local/bin/claw\n"
        f"{_relaxed_start_ready_install()}"
    )


def _persist_pg(alias: str, build, content_digest: str, image_ref: str) -> None:
    now_ms = int(time.time() * 1000)
    try:
        from e2b_pg_settings import merge_settings_json_key

        merge_settings_json_key(
            "e2bWorkerRelaxed",
            {
                "templateId": build.template_id,
                "buildId": build.build_id,
                "contentHash": content_digest,
                "alias": alias,
                "imageRef": image_ref,
                "imageDigest": try_image_digest(image_ref),
                "updatedAtMs": now_ms,
            },
            now_ms=now_ms,
        )
        print(
            f"==> persisted e2bWorkerRelaxed.templateId={build.template_id!r} "
            f"buildId={build.build_id!r} to PG"
        )
    except Exception as exc:  # noqa: BLE001
        print(f"warn: skip PG e2bWorkerRelaxed.templateId persist: {exc}", file=sys.stderr)


def _is_thin_relaxed(image: str) -> bool:
    return "claw-worker-base-relaxed" in image and "claw-gateway-worker" not in image


def _e2b_relaxed_image_tag(worker_image: str) -> str:
    # Parallel to strict debian-bookworm-claw-worker tagging. Author: kejiqing
    if ":" in worker_image.rsplit("/", 1)[-1]:
        base, tag = worker_image.rsplit(":", 1)
        prefix = base.rsplit("/", 1)[0]
        return f"{prefix}/debian-bookworm-claw-worker-relaxed:{tag}"
    return f"{worker_image}-e2b-relaxed"


def _acr_login_if_needed(image: str) -> None:
    if "/" not in image or image.startswith("localhost"):
        return
    registry = image.split("/", 1)[0]
    user = _env("CLAW_REGISTRY_USER", _env("NEXUS_USER"))
    password = _env("CLAW_REGISTRY_PASSWORD", _env("NEXUS_PASSWORD"))
    if not user or not password:
        return
    rt = _container_runtime()
    subprocess.run(
        [shutil.which(rt) or rt, "login", registry, "-u", user, "--password-stdin"],
        input=password.encode(),
        check=False,
    )


def _build_e2b_relaxed_image(worker_image: str) -> str:
    """Layer e2b start/ready on thin relaxed base; push for e2b host pull."""
    rt = _container_runtime()
    platform = _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")
    e2b_image = _e2b_relaxed_image_tag(worker_image)
    if not DOCKERFILE_E2B_RELAXED.is_file():
        raise SystemExit(f"error: missing {DOCKERFILE_E2B_RELAXED}")
    apt_mirror = template_debian_apt_mirror()
    print(
        f"==> {rt} build e2b relaxed image {e2b_image!r} "
        f"(FROM {worker_image!r}, {platform})"
    )
    subprocess.check_call(
        [
            rt,
            "build",
            "-f",
            str(DOCKERFILE_E2B_RELAXED),
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
    )
    if _env("CLAW_E2B_WORKER_E2B_PUSH", "1") not in ("0", "false", "no"):
        _acr_login_if_needed(e2b_image)
        print(f"==> {rt} push {e2b_image!r}")
        subprocess.check_call([rt, "push", e2b_image])
    return e2b_image


def _registry_bootstrap_mode() -> bool:
    """Legacy Admin path: registry extract claw into debian Dockerfile. Author: kejiqing"""
    return _truthy("CLAW_E2B_WORKER_RELAXED_FROM_IMAGE") or _truthy(
        "CLAW_E2B_WORKER_SKIP_LOCAL_BUILD"
    )


def main() -> int:
    opts = _conn_opts()
    log_debian_base_resolution(api_url=opts["api_url"])
    alias = _env("CLAW_E2B_WORKER_RELAXED_ALIAS") or "claw-worker-relaxed"

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    skip_cache = _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")
    source_image = _env("CLAW_E2B_WORKER_RELAXED_IMAGE") or _worker_base_image()

    # Thin shell: from_image only (no COPY claw). Author: kejiqing
    if _is_thin_relaxed(source_image):
        e2b_image = _build_e2b_relaxed_image(source_image)
        print(f"==> e2b Template.build from_image={e2b_image!r} (thin relaxed)")
        content_digest = digest_parts(
            [
                ("image", e2b_image.encode()),
                ("start", RELAXED_START_CMD.encode()),
                ("ready", RELAXED_READY_CMD.encode()),
            ]
        )
        if try_skip_unchanged(
            "e2bWorkerRelaxed", content_digest, image_ref=source_image
        ):
            return 0
        template = Template().from_image(e2b_image)
        template = template.set_start_cmd(RELAXED_START_CMD, RELAXED_READY_CMD)
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
        print(f"template_id: {build.template_id}")
        print(f"build_id: {build.build_id}")
        _persist_pg(alias, build, content_digest, source_image)
        print(f"OK: relaxed worker template {alias!r} ({build.template_id}) thin-from_image")
        return 0

    registry_mode = _registry_bootstrap_mode()
    with tempfile.TemporaryDirectory(prefix="claw-e2b-relaxed-") as tmp:
        staging = Path(tmp)
        if registry_mode:
            print(
                f"==> legacy relaxed: debian + COPY claw from {source_image!r}",
                file=sys.stderr,
            )
            _stage_claw_from_registry(staging, source_image)
        else:
            _stage_claw_into(staging)

        dockerfile = staging / "Dockerfile"
        dockerfile.write_text(_relaxed_dockerfile(), encoding="utf-8")
        print("==> e2b Template.build from_dockerfile (legacy debian + claw)")
        content_digest = digest_tree(
            staging,
            [
                ("debian", template_debian_base_image().encode()),
                ("start", RELAXED_START_CMD.encode()),
                ("ready", RELAXED_READY_CMD.encode()),
            ],
        )
        if try_skip_unchanged(
            "e2bWorkerRelaxed", content_digest, image_ref=source_image
        ):
            return 0
        template = (
            Template(file_context_path=str(staging))
            .from_dockerfile(str(dockerfile))
            .set_start_cmd(RELAXED_START_CMD, RELAXED_READY_CMD)
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

    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    _persist_pg(alias, build, content_digest, source_image)
    print(
        "hint: rebuild only updates PG; new build is used after gateway restart, "
        "manual worker reset, or when the sandbox is dead"
    )
    print(f"OK: relaxed worker template {alias!r} ({build.template_id}) tools-only")

    if _env("CLAW_E2B_TEMPLATE_SKIP_VERIFY", "0") not in ("1", "true", "yes"):
        verify_py = _E2B_DIR / "verify-claw-worker-relaxed-sandbox.py"
        if verify_py.is_file():
            print("==> post-build sandbox verify …")
            env = os.environ.copy()
            env["CLAW_E2B_TEMPLATE_RELAXED"] = build.template_id
            subprocess.check_call([sys.executable, str(verify_py)], env=env)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
