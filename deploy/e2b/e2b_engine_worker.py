"""e2b template for a neuro-harness engine worker = strict claw-worker template + engine layer.

Layer comes from the CI image claw-gateway-worker-<engine>:<tag> via registry HTTP extract
(no nested podman): /usr/local/bin (claw, neuro-<engine>, node) and /usr/local/lib/neuro-engines.
Start/ready commands and the debian base are the strict worker's. PG key: e2bWorker<Engine>.
Author: kejiqing
"""
from __future__ import annotations

import importlib.util
import os
import shutil
import sys
import tempfile
import time
from pathlib import Path

_E2B_DIR = Path(__file__).resolve().parent
if str(_E2B_DIR) not in sys.path:
    sys.path.insert(0, str(_E2B_DIR))
from e2b_pg_settings import merge_settings_json_key
from e2b_template_build import build_template_with_retry
from e2b_template_content_hash import digest_tree, try_skip_unchanged
from e2b_template_registry import (
    apply_template_skip_cache_force,
    log_debian_base_resolution,
    template_debian_base_image,
    template_image_prefix,
)
from registry_extract import extract_paths_from_image, try_image_digest

ENGINES: dict[str, dict[str, object]] = {
    "opencode": {
        "settings_key": "e2bWorkerOpencode",
        "alias": "claw-worker-opencode",
        "smoke": ["/usr/local/lib/neuro-engines/opencode/bin/opencode --version"],
    },
    "appserver": {
        "settings_key": "e2bWorkerAppserver",
        "alias": "claw-worker-appserver",
        "smoke": [
            "node --version",
            "node /usr/local/lib/neuro-engines/codex-acp/node_modules/@openai/codex/bin/codex.js --version",
        ],
    },
}


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _strict_worker_module():
    spec = importlib.util.spec_from_file_location(
        "claw_worker_strict_build", _E2B_DIR / "build-claw-worker-selfhosted.py"
    )
    if spec is None or spec.loader is None:
        raise SystemExit("error: cannot load build-claw-worker-selfhosted.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def engine_image(engine: str) -> str:
    """CLAW_E2B_WORKER_<ENGINE>_IMAGE, else <prefix>/claw-gateway-worker-<engine>:<CLAW_IMAGE_RELEASE_TAG>."""
    explicit = _env(f"CLAW_E2B_WORKER_{engine.upper()}_IMAGE")
    if explicit:
        return explicit
    tag = _env("CLAW_IMAGE_RELEASE_TAG")
    if not tag:
        raise SystemExit(
            f"error: set CLAW_E2B_WORKER_{engine.upper()}_IMAGE or CLAW_IMAGE_RELEASE_TAG"
        )
    return f"{template_image_prefix()}/claw-gateway-worker-{engine}:{tag}"


def _engine_dockerfile(strict) -> str:
    return (
        strict._dockerfile_debian_copy()
        + "COPY bin/ /usr/local/bin/\n"
        + "COPY neuro-engines/ /usr/local/lib/neuro-engines/\n"
    )


def _check_elf(strict, path: Path, arch: str) -> None:
    probe = strict._elf_probe(path)
    print(f"  {path.name}: {probe}")
    if not strict._elf_arch_ok(probe, arch):
        raise SystemExit(f"error: {path.name} is not linux/{arch} ({probe})")


def _verify(template_id: str, engine: str, opts: dict[str, str]) -> int:
    from e2b import Sandbox

    print(f"==> verify: sandbox template={template_id!r} engine={engine}")
    sandbox = Sandbox.create(template_id, timeout=900, **opts)
    try:
        cmds = [
            "command -v claw",
            f"command -v neuro-{engine}",
            *ENGINES[engine]["smoke"],  # type: ignore[misc]
        ]
        for cmd in cmds:
            r = sandbox.commands.run(cmd, timeout=120)
            print(f"$ {cmd} -> exit={r.exit_code} stdout={(r.stdout or '').strip()!r}")
            if r.exit_code not in (0, None):
                return r.exit_code or 1
    finally:
        sandbox.kill()
    return 0


def build(engine: str) -> int:
    spec = ENGINES[engine]
    settings_key = str(spec["settings_key"])
    alias = _env(f"CLAW_E2B_WORKER_{engine.upper()}_ALIAS") or str(spec["alias"])
    strict = _strict_worker_module()
    opts = strict._conn_opts()
    log_debian_base_resolution(api_url=opts["api_url"])
    os.environ.setdefault("E2B_API_KEY", opts["api_key"])
    os.environ.setdefault("E2B_API_URL", opts["api_url"])
    os.environ.setdefault(
        "E2B_SANDBOX_URL",
        _env("E2B_SANDBOX_URL", _env("CLAW_E2B_SANDBOX_URL", "http://10.8.0.1:3002")),
    )
    os.environ.setdefault("E2B_DOMAIN", opts["domain"])

    from e2b import Template, default_build_logger

    skip_cache = _env("CLAW_E2B_TEMPLATE_SKIP_CACHE", "0") not in ("0", "false", "no")
    platform = strict._template_platform()
    arch = strict._linux_arch_from_platform(platform)
    image = engine_image(engine)

    with tempfile.TemporaryDirectory(prefix=f"claw-e2b-{engine}-") as tmp:
        staging = Path(tmp)
        extract_paths_from_image(
            image,
            {
                "/usr/local/bin": staging / "bin",
                "/usr/local/lib/neuro-engines": staging / "neuro-engines",
            },
            platform=platform,
        )
        shutil.copy2(staging / "bin" / "claw", staging / "claw.bin")
        _check_elf(strict, staging / "claw.bin", arch)
        _check_elf(strict, staging / "bin" / f"neuro-{engine}", arch)
        dockerfile = staging / "Dockerfile"
        dockerfile.write_text(_engine_dockerfile(strict), encoding="utf-8")
        content_digest = digest_tree(
            staging,
            [
                ("debian", template_debian_base_image().encode()),
                ("start", strict.WORKER_START_CMD.encode()),
                ("ready", strict.WORKER_READY_CMD.encode()),
            ],
        )
        if try_skip_unchanged(settings_key, content_digest):
            return 0
        print(f"==> e2b Template.build {alias!r}: strict worker + {engine} layer from {image!r}")
        template = (
            Template(file_context_path=str(staging))
            .from_dockerfile(str(dockerfile))
            .set_start_cmd(strict.WORKER_START_CMD, strict.WORKER_READY_CMD)
        )
        apply_template_skip_cache_force(template, skip_cache)
        build_result = build_template_with_retry(
            Template.build,
            label=alias,
            template=template,
            alias=alias,
            skip_cache=skip_cache,
            on_build_logs=default_build_logger(),
            **opts,
        )

    print(f"template_id: {build_result.template_id}")
    print(f"build_id: {build_result.build_id}")
    now_ms = int(time.time() * 1000)
    merge_settings_json_key(
        settings_key,
        {
            "templateId": build_result.template_id,
            "buildId": build_result.build_id,
            "contentHash": content_digest,
            "alias": alias,
            "imageRef": image,
            "imageDigest": try_image_digest(image, platform=platform),
            "updatedAtMs": now_ms,
        },
        now_ms=now_ms,
    )
    print(f"==> persisted {settings_key}.templateId={build_result.template_id!r} to PG")
    if _env("CLAW_E2B_TEMPLATE_SKIP_VERIFY", "0") in ("1", "true", "yes"):
        return 0
    return _verify(build_result.template_id, engine, opts)
