#!/usr/bin/env python3
"""Self-hosted e2b template build helpers. Author: kejiqing"""
from __future__ import annotations

import os
import time
from datetime import datetime
from pathlib import Path
from typing import Callable, Optional

from e2b.api.client_sync import get_api_client
from e2b.connection_config import ConnectionConfig
from e2b.exceptions import FileUploadException
from e2b.template.consts import RESOLVE_SYMLINKS
from e2b.template.logger import LogEntry, LogEntryEnd, LogEntryStart
from e2b.template.main import TemplateBase, TemplateClass
from e2b.template.types import BuildInfo, InstructionType
from e2b.template.utils import normalize_build_arguments, read_dockerignore
from e2b.template_sync.build_api import (
    get_file_upload_link,
    request_build,
    trigger_build,
    upload_file,
    wait_for_build_finish,
)


def _env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def _env_int(name: str, default: int) -> int:
    raw = _env(name)
    if not raw:
        return default
    return int(raw)


def _env_float(name: str, default: float) -> float:
    raw = _env(name)
    if not raw:
        return default
    return float(raw)


def podman_platform_args() -> list[str]:
    """FC sandboxes are linux/amd64; Mac arm64 host must extract with matching platform."""
    plat = _env("CLAW_E2B_TEMPLATE_PLATFORM", "linux/amd64")
    return ["--platform", plat] if plat else []


def _copy_source_bytes(path: Path) -> int:
    if path.is_file():
        return path.stat().st_size
    if path.is_dir():
        total = sum(item.stat().st_size for item in path.rglob("*") if item.is_file())
        if total > 0:
            return total
        raise FileUploadException(f"COPY directory is empty: {path}")
    raise FileUploadException(f"COPY source missing on disk: {path}")


def _assert_local_copy_nonempty(context_path: str, src: str, min_bytes: int) -> int:
    path = Path(context_path) / src
    size = _copy_source_bytes(path)
    if size < min_bytes:
        raise FileUploadException(
            f"COPY source too small: {path} ({size} bytes; need >= {min_bytes})"
        )
    return size


def _wait_panel_file_present(
    api_client,
    template_id: str,
    files_hash: str,
    *,
    src: str,
    timeout_secs: float,
    poll_secs: float,
    stack_trace,
) -> None:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        info = get_file_upload_link(api_client, template_id, files_hash, stack_trace)
        if info.present:
            return
        time.sleep(poll_secs)
    raise FileUploadException(
        f"Panel file cache not present after {timeout_secs:.0f}s "
        f"(src={src!r}, hash={files_hash})"
    )


def _upload_copy_verified(
    api_client,
    template_obj: TemplateClass,
    template_id: str,
    *,
    src: str,
    files_hash: str,
    force_upload: bool,
    resolve_symlinks: bool,
    stack_trace,
    on_build_logs: Optional[Callable[[LogEntry], None]],
    min_copy_bytes: int,
    upload_retries: int,
    present_timeout_secs: float,
    present_poll_secs: float,
    post_upload_settle_secs: float,
) -> None:
    context_path = template_obj._template._file_context_path
    size = _assert_local_copy_nonempty(context_path, src, min_copy_bytes)
    if on_build_logs:
        on_build_logs(
            LogEntry(
                timestamp=datetime.now(),
                level="info",
                message=f"local COPY {src!r}: {size} bytes OK",
            )
        )

    ignore_patterns = [
        *template_obj._template._file_ignore_patterns,
        *read_dockerignore(context_path),
    ]

    info = get_file_upload_link(api_client, template_id, files_hash, stack_trace)
    force_reupload = _env("CLAW_E2B_FORCE_UPLOAD", "0") in ("1", "true", "yes")
    must_upload = force_reupload or force_upload or (
        info.present is False and bool(info.url)
    )

    if not must_upload:
        if on_build_logs:
            on_build_logs(
                LogEntry(
                    timestamp=datetime.now(),
                    level="info",
                    message=f"Skipping upload of {src!r}, already cached (hash present)",
                )
            )
        _wait_panel_file_present(
            api_client,
            template_id,
            files_hash,
            src=src,
            timeout_secs=present_timeout_secs,
            poll_secs=present_poll_secs,
            stack_trace=stack_trace,
        )
        return

    if not info.url:
        raise FileUploadException(f"No upload URL for {src!r} (hash={files_hash})")

    last_err: Optional[Exception] = None
    for attempt in range(1, upload_retries + 2):
        if on_build_logs:
            on_build_logs(
                LogEntry(
                    timestamp=datetime.now(),
                    level="info",
                    message=f"Uploading {src!r} (attempt {attempt})",
                )
            )
        try:
            upload_file(
                api_client,
                src,
                context_path,
                info.url,
                ignore_patterns,
                resolve_symlinks,
                stack_trace,
            )
            if post_upload_settle_secs > 0:
                time.sleep(post_upload_settle_secs)
            _wait_panel_file_present(
                api_client,
                template_id,
                files_hash,
                src=src,
                timeout_secs=present_timeout_secs,
                poll_secs=present_poll_secs,
                stack_trace=stack_trace,
            )
            if on_build_logs:
                on_build_logs(
                    LogEntry(
                        timestamp=datetime.now(),
                        level="info",
                        message=f"Uploaded {src!r}; Panel cache present (hash={files_hash})",
                    )
                )
            return
        except FileUploadException as exc:
            last_err = exc
            info = get_file_upload_link(api_client, template_id, files_hash, stack_trace)
            if not info.url:
                raise
            if attempt <= upload_retries:
                if on_build_logs:
                    on_build_logs(
                        LogEntry(
                            timestamp=datetime.now(),
                            level="warning",
                            message=f"upload verify failed for {src!r}: {exc}; retrying",
                        )
                    )
                time.sleep(present_poll_secs)
                continue
            raise
    if last_err:
        raise last_err


def template_build_verified(
    template: TemplateClass,
    *,
    alias: Optional[str] = None,
    name: Optional[str] = None,
    tags: Optional[list[str]] = None,
    cpu_count: int = 2,
    memory_mb: int = 1024,
    skip_cache: bool = False,
    on_build_logs: Optional[Callable[[LogEntry], None]] = None,
    **opts,
) -> BuildInfo:
    """Template.build with COPY upload present-check before trigger_build. Author: kejiqing"""
    template_name = normalize_build_arguments(name, alias)
    min_copy_bytes = _env_int("CLAW_E2B_MIN_COPY_BYTES", 1024)
    upload_retries = _env_int("CLAW_E2B_UPLOAD_RETRIES", 2)
    present_timeout_secs = _env_float("CLAW_E2B_PRESENT_TIMEOUT_SECS", 300.0)
    present_poll_secs = _env_float("CLAW_E2B_PRESENT_POLL_SECS", 0.5)
    post_upload_settle_secs = _env_float("CLAW_E2B_POST_UPLOAD_SETTLE_SECS", 0.25)

    if skip_cache:
        template._template._force = True

    try:
        if on_build_logs:
            on_build_logs(
                LogEntryStart(timestamp=datetime.now(), message="Build started (upload verify)")
            )

        config = ConnectionConfig(**opts)
        api_client = get_api_client(config)

        if on_build_logs:
            on_build_logs(
                LogEntry(
                    timestamp=datetime.now(),
                    level="info",
                    message=f"Requesting build for template: {template_name}",
                )
            )

        response = request_build(
            api_client,
            name=template_name,
            cpu_count=cpu_count,
            memory_mb=memory_mb,
            tags=tags,
        )
        template_id = response.template_id
        build_id = response.build_id

        if on_build_logs:
            on_build_logs(
                LogEntry(
                    timestamp=datetime.now(),
                    level="info",
                    message=f"Template created with ID: {template_id}, Build ID: {build_id}",
                )
            )

        instructions_with_hashes = template._template._instructions_with_hashes()
        for index, file_upload in enumerate(instructions_with_hashes):
            if file_upload["type"] != InstructionType.COPY:
                continue

            args = file_upload.get("args", [])
            src = args[0] if len(args) > 0 else None
            force_upload = file_upload.get("forceUpload")
            files_hash = file_upload.get("filesHash", None)
            resolve_symlinks = file_upload.get("resolveSymlinks", RESOLVE_SYMLINKS)

            if src is None or files_hash is None:
                raise ValueError("Source path and files hash are required")

            stack_trace = None
            if index + 1 < len(template._template._stack_traces):
                stack_trace = template._template._stack_traces[index + 1]

            _upload_copy_verified(
                api_client,
                template,
                template_id,
                src=src,
                files_hash=files_hash,
                force_upload=bool(force_upload),
                resolve_symlinks=resolve_symlinks,
                stack_trace=stack_trace,
                on_build_logs=on_build_logs,
                min_copy_bytes=min_copy_bytes,
                upload_retries=upload_retries,
                present_timeout_secs=present_timeout_secs,
                present_poll_secs=present_poll_secs,
                post_upload_settle_secs=post_upload_settle_secs,
            )

        if on_build_logs:
            on_build_logs(
                LogEntry(
                    timestamp=datetime.now(),
                    level="info",
                    message="All file uploads verified present; starting build",
                )
            )

        trigger_build(
            api_client,
            template_id,
            build_id,
            template._template._serialize(instructions_with_hashes),
        )

        if on_build_logs:
            on_build_logs(
                LogEntry(
                    timestamp=datetime.now(),
                    level="info",
                    message="Waiting for logs...",
                )
            )

        wait_for_build_finish(
            api_client,
            template_id,
            build_id,
            on_build_logs,
            logs_refresh_frequency=TemplateBase._logs_refresh_frequency,
            stack_traces=template._template._stack_traces,
        )

        return BuildInfo(
            template_id=template_id,
            build_id=build_id,
            alias=template_name,
            name=template_name,
            tags=response.tags,
        )
    finally:
        if on_build_logs:
            on_build_logs(LogEntryEnd(timestamp=datetime.now(), message="Build finished"))
