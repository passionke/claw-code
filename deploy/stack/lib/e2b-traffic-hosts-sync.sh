#!/usr/bin/env bash
# Maintain /etc/hosts for self-hosted e2b traffic hosts (tap Live).
# Sandbox id changes on recreate — run this after e2b-tap-live-up.
# Author: kejiqing
set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${LIB_DIR}/../../.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
MARK_BEGIN="# CLAW-FC-TRAFFIC-BEGIN"
MARK_END="# CLAW-FC-TRAFFIC-END"
FC_DOMAIN="${CLAW_E2B_DOMAIN:-supone.top}"
APPLY=0

usage() {
  cat <<EOF
Usage: $(basename "$0") [--apply]

Print or update /etc/hosts block for current e2b traffic hostnames:
  {port}-sbx_*.{CLAW_E2B_DOMAIN}  →  ${FC_DOMAIN}

Options:
  --apply   replace marked block in /etc/hosts (needs sudo)

Why sandbox id keeps changing:
  e2b assigns a new sbx_* on each sandbox CREATE (gateway restart, unhealthy recreate, no --reuse).
  Keep stable: ./deploy/stack/gateway.sh observe-tap-up --reuse

One-time wildcard DNS (no per-id hosts lines): see deploy/stack/lib/e2b-traffic-hosts-sync.sh header in docs
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --apply) APPLY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC1090
  set -a && source "${ENV_FILE}" && set +a
  FC_DOMAIN="${CLAW_E2B_DOMAIN:-${FC_DOMAIN}}"
fi

lines=()
comment="# e2b traffic — auto $(date -u +%Y-%m-%dT%H:%MZ)"

if [[ -x "${LIB_DIR}/e2b-tap-live-up.sh" ]] || [[ -f "${LIB_DIR}/e2b-tap-live-up.sh" ]]; then
  tap_json="$("${LIB_DIR}/e2b-tap-live-up.sh" --reuse --json 2>/dev/null || true)"
  if [[ -n "${tap_json}" ]]; then
    tap_line="$(python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print(d.get('liveBrowserHostsLine',''))" <<<"${tap_json}")"
    tap_sid="$(python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print(d.get('sandboxId',''))" <<<"${tap_json}")"
    if [[ -n "${tap_line}" ]]; then
      lines+=("${tap_line}  # tap Live ${tap_sid}")
    fi
  fi
fi

if [[ ${#lines[@]} -eq 0 ]]; then
  echo "error: no traffic hosts found (run gateway.sh observe-tap-up --reuse)" >&2
  exit 1
fi

block="$(printf '%s\n' "${MARK_BEGIN}" "${comment}" "${lines[@]}" "${MARK_END}")"

if [[ "${APPLY}" -eq 0 ]]; then
  echo "# Paste into /etc/hosts (sudo), or: $0 --apply"
  echo "${block}"
  echo
  echo "# Do NOT use sandbox.local in traffic URLs — e2b routes Host *.${FC_DOMAIN} only."
  exit 0
fi

if [[ "$(id -u)" -ne 0 ]]; then
  exec sudo "$0" --apply
fi

hosts="/etc/hosts"
tmp="$(mktemp)"
if grep -q "${MARK_BEGIN}" "${hosts}" 2>/dev/null; then
  awk -v begin="${MARK_BEGIN}" -v end="${MARK_END}" '
    $0 == begin { skip=1; next }
    $0 == end { skip=0; next }
    skip { next }
    { print }
  ' "${hosts}" >"${tmp}"
else
  cp "${hosts}" "${tmp}"
fi
{
  cat "${tmp}"
  echo
  echo "${block}"
} >"${hosts}.claw-new"
mv "${hosts}.claw-new" "${hosts}"
rm -f "${tmp}"
echo "updated ${hosts} (${#lines[@]} traffic host(s))"
