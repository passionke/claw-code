#!/usr/bin/env bash
# e2b singletons (nas-api / ovs / observe) before gateway compose up. Author: kejiqing
set -euo pipefail

claw_e2b_singletons_up_and_verify() {
  local lib_dir="${1:?}"
  local repo_root="${2:?}"

  echo "==> e2b singletons: up --reuse (nas-api / ovs / observe → PG)" >&2
  bash "${lib_dir}/e2b-singletons-up.sh" --reuse

  echo "==> e2b singletons: verify health" >&2
  (
    cd "${repo_root}"
    set -a
    # shellcheck disable=SC1091
    source "${repo_root}/.env"
    set +a
    export PYTHONPATH="${repo_root}/deploy/e2b:${PYTHONPATH:-}"
    python3 - <<'PY'
import json
import os
import sys
import urllib.error
import urllib.request

from deploy.e2b.e2b_pg_settings import load_settings_json_key


def curl_ok(url: str, label: str) -> None:
    req = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            if resp.status >= 400:
                raise RuntimeError(f"HTTP {resp.status}")
    except urllib.error.HTTPError as exc:
        raise RuntimeError(f"{label} HTTP {exc.code} at {url}") from exc
    except OSError as exc:
        raise RuntimeError(f"{label} unreachable at {url}: {exc}") from exc


nas = load_settings_json_key("e2bNasApi")
nas_url = str(nas.get("baseUrl") or "").rstrip("/")
if not nas_url:
    raise SystemExit("e2bNasApi.baseUrl missing after singletons-up")
curl_ok(f"{nas_url}/healthz", "nas-api")

tap = load_settings_json_key("clawTap")
proxy_url = str(tap.get("proxyBaseUrl") or "").rstrip("/")
observe_id = str(tap.get("e2bObserveSandboxId") or "").strip()
if not proxy_url or not observe_id:
    raise SystemExit("clawTap.proxyBaseUrl / e2bObserveSandboxId missing after singletons-up")
curl_ok(f"{proxy_url}/healthz", "observe-tap")

ovs = load_settings_json_key("e2bOvs")
ovs_url = str(ovs.get("baseUrl") or "").rstrip("/")
if ovs_url:
    curl_ok(f"{ovs_url}/", "ovs")
else:
    print("warn: e2bOvs.baseUrl empty; skip ovs probe", file=sys.stderr)

print(
    json.dumps(
        {
            "nasApi": nas_url,
            "observeSandboxId": observe_id,
            "observeProxy": proxy_url,
            "ovs": ovs_url or None,
        },
        ensure_ascii=False,
    )
)
PY
  )
}
