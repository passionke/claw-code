# L3 — http-gateway-rs → container worker

Version: **v1**  
Author: kejiqing

## Purpose

How gateway dispatches solve into the pool and what the worker returns. Public HTTP details remain in [http-gateway-rs-api.md](../http-gateway-rs-api.md).

## Dispatch

1. Gateway acquires pool slot (`CLAW_DOCKER_POOL_SIZE` / Podman equivalents).
2. Writes `ds_home/.claw/settings.json` and session dir under `CLAW_WORK_ROOT`.
3. Executes `claw gateway-solve-once` (or pool equivalent) inside worker.
4. Releases slot; persists task row in SQLite.

## Worker outcome

| `clawExitCode` | Meaning |
|----------------|---------|
| 0 | Success |
| non-zero | Failed; `outputText` / task `error` populated |

Session messages: `.claw/gateway-solve-session.jsonl` in session home.

Progress: `.claw/task-progress.json` → surfaced as `currentTaskDesc` on task API.

## Event tap source (v1)

Gateway tails NDJSON lines while solve runs (from worker stdout or session file watcher) and exposes them on `GET /v1/events/{task_id}` per L2.

## Self-check

Covered by `tests/http-gateway-agui-bridge.sh` and existing gateway integration tests.
