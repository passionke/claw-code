#!/usr/bin/env bash
# Exec claw-pool-daemon with pool-daemon.env (detached entry). Author: kejiqing
set -euo pipefail
RPC_DIR="${1:?rpc_dir}"
ENV_FILE="${RPC_DIR}/pool-daemon.env"
LOG="${RPC_DIR}/daemon.log"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "pool-daemon-run: missing ${ENV_FILE}" >&2
  exit 1
fi

while IFS= read -r line || [[ -n "${line}" ]]; do
  [[ "${line}" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] || continue
  k="${line%%=*}"
  v="${line#*=}"
  if [[ "${v}" =~ ^\'.*\'$ ]]; then
    v="${v:1:${#v}-2}"
    v="${v//\'\\\'\'/\'}"
  fi
  export "${k}=${v}"
done <"${ENV_FILE}"

exec >>"${LOG}" 2>&1
exec "${CLAW_POOL_DAEMON_BIN:?CLAW_POOL_DAEMON_BIN unset}"
