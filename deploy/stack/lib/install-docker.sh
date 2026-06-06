#!/usr/bin/env bash
# Standard Docker install for production Linux (docker_pool + gateway.sh compose). Author: kejiqing
#
# Usage:
#   ./deploy/stack/gateway.sh install-docker
#   ./deploy/stack/lib/install-docker.sh
#
# Idempotent. Registry mirror defaults to CONTAINER_BASE_REGISTRY (docker.1ms.run) unless CLAW_USE_DOCKER_IO=1.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

die() {
  echo "error: $*" >&2
  exit 1
}

info() {
  echo "==> $*" >&2
}

[[ "$(uname -s)" == "Linux" ]] || die "install-docker is for Linux production only; local macOS uses Podman (deploy/stack/README.md)"

if [[ -f "${REPO_ROOT}/.env" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${REPO_ROOT}/.env"
  set +a
fi

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  info "docker already installed and daemon running"
  docker --version
  if docker compose version >/dev/null 2>&1; then
    docker compose version
  elif command -v docker-compose >/dev/null 2>&1; then
    docker-compose --version
  fi
  exit 0
fi

need_sudo=0
[[ "$(id -u)" -ne 0 ]] && need_sudo=1

run() {
  if [[ "${need_sudo}" -eq 1 ]]; then
    sudo "$@"
  else
    "$@"
  fi
}

install_compose_apt() {
  if run apt-get install -y docker-compose-v2 2>/dev/null; then
    return 0
  fi
  if run apt-get install -y docker-compose-plugin 2>/dev/null; then
    return 0
  fi
  run apt-get install -y docker-compose
}

install_apt() {
  run apt-get update
  run apt-get install -y ca-certificates curl docker.io
  install_compose_apt
}

install_dnf() {
  if run dnf install -y docker docker-compose-plugin 2>/dev/null; then
    return 0
  fi
  run dnf install -y docker docker-compose
}

if command -v apt-get >/dev/null 2>&1; then
  install_apt
elif command -v dnf >/dev/null 2>&1; then
  install_dnf
else
  die "unsupported distro: need apt-get or dnf (manual: apt install docker.io + docker compose plugin)"
fi

run systemctl enable --now docker

# deploy/stack/README.md — real Docker hosts: avoid podman masquerading as docker.
if [[ ! -f /etc/containers/nodocker ]]; then
  run mkdir -p /etc/containers
  run touch /etc/containers/nodocker
  info "created /etc/containers/nodocker"
fi

if [[ "${CLAW_USE_DOCKER_IO:-0}" != "1" ]]; then
  mirror="${CONTAINER_BASE_REGISTRY:-docker.1ms.run}"
  mirror="${mirror#http://}"
  mirror="${mirror#https://}"
  daemon_json="/etc/docker/daemon.json"
  if [[ ! -f "${daemon_json}" ]] || ! grep -q "${mirror}" "${daemon_json}" 2>/dev/null; then
    tmp="$(mktemp)"
    python3 - "${daemon_json}" "https://${mirror}" "${tmp}" <<'PY'
import json
import os
import sys

path, mirror, out = sys.argv[1], sys.argv[2], sys.argv[3]
data = {}
if os.path.isfile(path):
    with open(path, encoding="utf-8") as f:
        try:
            data = json.load(f)
        except json.JSONDecodeError:
            data = {}
mirrors = list(data.get("registry-mirrors") or [])
if mirror not in mirrors:
    mirrors.insert(0, mirror)
data["registry-mirrors"] = mirrors
with open(out, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PY
    run mkdir -p /etc/docker
    run install -m 0644 "${tmp}" "${daemon_json}"
    rm -f "${tmp}"
    run systemctl restart docker || true
    info "configured registry mirror https://${mirror} (set CLAW_USE_DOCKER_IO=1 to skip)"
  fi
fi

target_user="${SUDO_USER:-${USER:-}}"
if [[ -n "${target_user}" ]] && [[ "${target_user}" != "root" ]]; then
  if ! id -nG "${target_user}" 2>/dev/null | grep -qw docker; then
    run usermod -aG docker "${target_user}" || true
    info "added ${target_user} to docker group (re-login or: newgrp docker)"
  fi
fi

docker info >/dev/null 2>&1 || die "docker daemon not ready after install (check: systemctl status docker)"
docker --version
if docker compose version >/dev/null 2>&1; then
  docker compose version
elif command -v docker-compose >/dev/null 2>&1; then
  docker-compose --version
else
  die "docker compose / docker-compose not found after install"
fi

info "install-docker ok"
