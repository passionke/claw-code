#!/usr/bin/env python3
"""Build claw-observe template (debian + claude-tap Live) on self-hosted e2b. Author: kejiqing"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# v0.0.10+ published to GHCR by claude-tap CI (not ACR). Override via CLAUDE_TAP_IMAGE.
DEFAULT_CLAUDE_TAP_IMAGE = "ghcr.io/passionke/claude-tap:v0.0.10"


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _container_runtime() -> str:
    rt = _env("CLAW_CONTAINER_RUNTIME", "podman")
    return "podman" if rt == "auto" else rt


def _conn_opts() -> dict[str, str]:
    return {
        "api_key": _env("E2B_API_KEY", _env("CLAW_E2B_API_KEY", "e2b_53ae1fed82754c17ad8077fbc8bcdd90")),
        "api_url": _env("E2B_API_URL", _env("CLAW_E2B_API_URL", "http://10.8.0.1:3000")),
        "domain": _env("E2B_DOMAIN", _env("CLAW_E2B_DOMAIN", "supone.top")),
    }


def _stage_tap_runtime(staging: Path, rt: str) -> bool:
    """Stage linux/amd64 claude-tap runtime (never macOS host uv script). Author: kejiqing"""
    tap_rt = staging / "tap-runtime"
    if tap_rt.exists():
        shutil.rmtree(tap_rt)

    explicit = _env("CLAUDE_TAP_SOURCE_BIN")
    if explicit:
        src = Path(explicit)
        if src.is_file():
            tap_rt.mkdir(parents=True)
            (tap_rt / "bin").mkdir()
            shutil.copy2(src, tap_rt / "bin" / "claude-tap")
            return True

    for tools_root in (
        _env("CLAW_NAS_TOOLS_DIR"),
        str(ROOT / "deploy/stack/claw-workspace/.claw-e2b-tools"),
    ):
        if not tools_root:
            continue
        src = Path(tools_root) / "tap-runtime"
        libpython = src / "lib" / "libpython3.12.so.1.0"
        stdlib = src / "lib" / "python3.12"
        if (src / "bin" / "claude-tap").is_file() and libpython.is_file() and stdlib.is_dir():
            shutil.copytree(src, tap_rt)
            print(f"==> staged tap-runtime from {src}")
            return True

    tap_image = _env("CLAUDE_TAP_IMAGE", DEFAULT_CLAUDE_TAP_IMAGE)
    try:
        subprocess.check_call(
            [rt, "pull", "--platform", "linux/amd64", tap_image],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        cid = subprocess.check_output(
            [rt, "create", "--platform", "linux/amd64", tap_image],
            text=True,
        ).strip()
        try:
            (tap_rt / "bin").mkdir(parents=True)
            (tap_rt / "lib").mkdir(parents=True)
            for src_path, dest_name in (
                ("/usr/local/bin/python3.12", "python3.12"),
                ("/usr/local/bin/claude-tap", "claude-tap"),
            ):
                try:
                    subprocess.check_call(
                        [rt, "cp", f"{cid}:{src_path}", str(tap_rt / "bin" / dest_name)]
                    )
                except subprocess.CalledProcessError:
                    continue
            for src_path, dest_name in (
                ("/usr/local/lib/libpython3.12.so.1.0", "libpython3.12.so.1.0"),
                ("/usr/local/lib/libpython3.12.so", "libpython3.12.so"),
            ):
                try:
                    subprocess.check_call(
                        [rt, "cp", f"{cid}:{src_path}", str(tap_rt / "lib" / dest_name)]
                    )
                except subprocess.CalledProcessError:
                    continue
            try:
                subprocess.check_call(
                    [rt, "cp", f"{cid}:/usr/local/lib/python3.12", str(tap_rt / "lib/python3.12")]
                )
            except subprocess.CalledProcessError:
                pass
            subprocess.call(
                ["chmod", "-R", "a+rx", str(tap_rt)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            if (tap_rt / "bin" / "claude-tap").is_file():
                print(f"==> staged tap-runtime from {tap_image}")
                return True
        finally:
            subprocess.call([rt, "rm", "-f", cid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except subprocess.CalledProcessError as exc:
        print(f"warn: podman tap-runtime staging failed: {exc}", file=sys.stderr)
    return False


def _sudo_nfs_setup() -> str:
    return r""" && apt-get install -y --no-install-recommends sudo \
    && echo 'user ALL=(ALL) NOPASSWD: /bin/mount, /bin/umount, /usr/bin/mountpoint, /bin/mkdir, /bin/chown' > /etc/sudoers.d/claw-nfs \
    && chmod 440 /etc/sudoers.d/claw-nfs"""


def _dockerfile_context(live_port: int) -> str:
    sudo = _sudo_nfs_setup()
    return f"""FROM docker.1ms.run/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \\
    nfs-common ca-certificates curl{sudo} \\
    && rm -rf /var/lib/apt/lists/* \\
    && mkdir -p /claw_ws /opt/claw-tap-runtime
COPY tap-runtime.tgz /tmp/tap-runtime.tgz
RUN tar xzf /tmp/tap-runtime.tgz -C /opt/claw-tap-runtime --strip-components=1 \\
    && rm -f /tmp/tap-runtime.tgz \\
    && printf '%s\\n' \\
        '#!/bin/sh' \\
        'export PYTHONHOME="/opt/claw-tap-runtime"' \\
        'export LD_LIBRARY_PATH="/opt/claw-tap-runtime/lib:${{LD_LIBRARY_PATH:-}}"' \\
        'export PYTHONPATH="/opt/claw-tap-runtime/lib/python3.12/site-packages:${{PYTHONPATH:-}}"' \\
        'exec /opt/claw-tap-runtime/bin/python3.12 /opt/claw-tap-runtime/bin/claude-tap "$@"' \\
        > /usr/local/bin/claude-tap \\
    && chmod +x /usr/local/bin/claude-tap /opt/claw-tap-runtime/bin/* \\
    && chmod -R a+rwX /claw_ws
RUN printf '%s\\n' \\
        '#!/bin/sh' \\
        'set -eu' \\
        ': "${{CLAW_CLUSTER_ID:?missing CLAW_CLUSTER_ID (sandbox create envVars)}}"' \\
        ': "${{CLAW_GATEWAY_DATABASE_URL:?missing CLAW_GATEWAY_DATABASE_URL (sandbox create envVars)}}"' \\
        'mkdir -p /claw_ws/tap-traces' \\
        'exec env CLAW_CLUSTER_ID="$CLAW_CLUSTER_ID" CLAW_GATEWAY_DATABASE_URL="$CLAW_GATEWAY_DATABASE_URL" /usr/local/bin/claude-tap --tap-no-launch --tap-live --tap-host 0.0.0.0 --tap-port 8080 --tap-live-port {live_port} --tap-target https://bootstrap.invalid/v1 --tap-output-dir /claw_ws/tap-traces --tap-no-update-check --tap-no-auto-update' \\
        > /usr/local/bin/claw-observe-start \\
    && printf '%s\\n' \\
        '#!/bin/sh' \\
        'exec curl -fsS --connect-timeout 2 http://127.0.0.1:{live_port}/' \\
        > /usr/local/bin/claw-observe-ready \\
    && chmod +x /usr/local/bin/claw-observe-start /usr/local/bin/claw-observe-ready
"""


def _observe_live_port() -> int:
    try:
        return int(_env("CLAW_E2B_OBSERVE_LIVE_PORT", "3000") or "3000")
    except ValueError:
        return 3000


def _observe_start_cmd(port: int) -> str:
    _ = port
    return "/usr/local/bin/claw-observe-start"


def _observe_ready_cmd(port: int) -> str:
    _ = port
    return "/usr/local/bin/claw-observe-ready"


def main() -> int:
    opts = _conn_opts()
    alias = _env("CLAW_E2B_OBSERVE_TEMPLATE", "claw-observe")
    live_port = _observe_live_port()

    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    with tempfile.TemporaryDirectory(prefix="claw-e2b-observe-") as tmp:
        staging = Path(tmp)
        rt = _container_runtime()
        print("==> staging linux/amd64 claude-tap runtime for observe template …")
        if not _stage_tap_runtime(staging, rt):
            print(
                "error: linux tap-runtime missing (run deploy/e2b/install-nas-fc-tools.sh "
                "or ensure podman can pull CLAUDE_TAP_IMAGE linux/amd64)",
                file=sys.stderr,
            )
            return 1
        subprocess.check_call(
            ["tar", "czf", str(staging / "tap-runtime.tgz"), "-C", str(staging), "tap-runtime"]
        )
        tap_image = _env("CLAUDE_TAP_IMAGE", DEFAULT_CLAUDE_TAP_IMAGE)
        tap_revision = tap_image.rsplit(":", 1)[-1].strip() or "latest"
        dockerfile_path = staging / "Dockerfile"
        dockerfile_path.write_text(_dockerfile_context(live_port), encoding="utf-8")
        print(f"==> tap bundle revision {tap_revision!r} (build context COPY)", file=sys.stderr)
        print(f"==> build ctx={staging}")
        template = (
            Template(file_context_path=str(staging))
            .from_dockerfile(str(dockerfile_path))
            .set_start_cmd(_observe_start_cmd(live_port), _observe_ready_cmd(live_port))
        )
        print(f"==> template startCmd=claude-tap Live :{live_port}")
        Template.build(template, alias=alias, on_build_logs=default_build_logger(), **opts)

    print(f"OK: template {alias!r} ready on {opts['api_url']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
