#!/usr/bin/env python3
"""Build claw-worker-relaxed (debian worker + built-in OVS) on self-hosted e2b. Author: kejiqing"""
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
    template_debian_base_image,
)
from ovs_bundle import ovs_port, pack_ovs_bundle, relaxed_worker_ovs_install_runfile, stage_ovs_tree

ROOT = Path(__file__).resolve().parents[2]
load_repo_dotenv(ROOT)


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


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


def _ovs_upstream_image() -> str:
    return _env(
        "CLAW_OVS_IMAGE",
        _env(
            "CLAW_OVS_UPSTREAM_IMAGE",
            "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/openvscode-server:1.109.5-ovs-chat-amd64",
        ),
    )


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
    return (
        _env("CLAW_E2B_WORKER_IMAGE")
        or _env("CLAW_E2B_TEMPLATE_FROM_IMAGE")
        or "crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke/debian-bookworm-claw-worker:release-v1.6.17"
    )


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


def _relaxed_dockerfile(port: int, ext_ver: str) -> str:
    debian = template_debian_base_image()
    apt = template_apt_prepare_prefix()
    return (
        f"FROM {debian}\n"
        # Relaxed package set = CI claw-gateway-worker-relaxed tools + NAS sudo + OVS (below).
        # Author: kejiqing
        f"RUN {apt}apt-get update && apt-get install -y --no-install-recommends \\\n"
        "    nfs-common ca-certificates sudo \\\n"
        "    curl git python3 python3-pip \\\n"
        "    && ln -sf /usr/bin/pip3 /usr/local/bin/pip \\\n"
        "    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \\\n"
        "    && chmod 440 /etc/sudoers.d/claw-nfs \\\n"
        "    && rm -rf /var/lib/apt/lists/*\n"
        "COPY claw.bin /usr/local/bin/claw\n"
        "RUN chmod +x /usr/local/bin/claw\n"
        "COPY openvscode-settings.json /tmp/openvscode-machine-settings.json\n"
        "RUN mkdir -p /tmp/ovs-bundle\n"
        "COPY claw-ovs-bundle.tar.gz /tmp/claw-ovs-bundle.tar.gz\n"
        f"{relaxed_worker_ovs_install_runfile(port, ext_ver)}\n"
    )


def main() -> int:
    opts = _conn_opts()
    log_debian_base_resolution(api_url=opts["api_url"])
    alias = _env("CLAW_E2B_WORKER_RELAXED_ALIAS") or "claw-worker-relaxed"
    port = ovs_port()
    rt = _container_runtime()

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-relaxed-") as tmp:
        staging = Path(tmp)
        _stage_claw_into(staging)

        print("==> staging OVS tree for relaxed worker …")
        ext_ver = stage_ovs_tree(staging, rt, _ovs_upstream_image())
        pack_ovs_bundle(staging)

        dockerfile = staging / "Dockerfile"
        dockerfile.write_text(_relaxed_dockerfile(port, ext_ver), encoding="utf-8")
        print(f"==> e2b Template.build from_dockerfile (debian + OVS :{port}/ovs)")
        template = (
            Template(file_context_path=str(staging))
            .from_dockerfile(str(dockerfile))
            .set_start_cmd(
                "/usr/local/bin/claw-worker-relaxed-start",
                "/usr/local/bin/claw-worker-relaxed-ready",
            )
        )
        apply_template_skip_cache_force(
            template,
            _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no"),
        )
        build = Template.build(template, alias=alias, on_build_logs=default_build_logger(), **opts)

    print(f"template_id: {build.template_id}")
    print(f"build_id: {build.build_id}")
    now_ms = int(time.time() * 1000)
    try:
        from e2b_pg_settings import merge_settings_json_key

        merge_settings_json_key(
            "e2bWorkerRelaxed",
            {
                "templateId": build.template_id,
                "buildId": build.build_id,
                "alias": alias,
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
    print(
        "hint: rebuild only updates PG; new build is used after gateway restart, "
        "manual worker reset, or when the sandbox is dead"
    )
    print(f"OK: relaxed worker template {alias!r} ({build.template_id}) with built-in OVS")

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
