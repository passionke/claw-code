#!/usr/bin/env bash
# ONE path. Author: kejiqing. Do not invent forks.
#
# Standard site promote: ACR → site registry (Nexus) → gateway.sh up.
# GHA still publishes legacy package names (claw-code / claw-gateway-playground);
# current gateway.sh up --release expects http-gateway-rs / http-gateway-playground
# + thin claw-worker-base. This script skopeo-copies with that rename map.
#
# Usage (on site host, e.g. sunmax@10.22.28.240):
#   RELEASE_TAG=release-v2.0.23 ./deploy/pack/promote-release-site.sh
#   RELEASE_TAG=release-v2.0.23 ./deploy/pack/promote-release-site.sh mirror   # registry only
#   RELEASE_TAG=release-v2.0.23 ./deploy/pack/promote-release-site.sh up      # up only
#
# Env:
#   CLAW_SRC_PREFIX          ACR pull prefix (default personal ACR /passionke)
#   CLAW_PROMOTE_PUSH_PREFIX Nexus hosted push (default repo.550w.com:8083/passionke)
#   CLAW_IMAGE_PREFIX        pull/up prefix (default .env or repo.550w.com:8082/passionke group)
#   CLAW_PROMOTE_LEGACY_ALIASES=1  also push claw-code / claw-gateway-* names
set -euo pipefail

# Resolve repo root even if invoked via /tmp copy. Author: kejiqing
_claw_promote_root() {
  local script_dir root
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  if [[ -f "${script_dir}/../stack/gateway.sh" ]]; then
    cd "${script_dir}/../.." && pwd
    return
  fi
  if [[ -n "${CLAW_REPO_ROOT:-}" && -f "${CLAW_REPO_ROOT}/deploy/stack/gateway.sh" ]]; then
    cd "${CLAW_REPO_ROOT}" && pwd
    return
  fi
  if [[ -f "${PWD}/deploy/stack/gateway.sh" ]]; then
    pwd
    return
  fi
  echo "error: cannot find repo root (run from claw-code or set CLAW_REPO_ROOT)" >&2
  return 1
}
ROOT="$(_claw_promote_root)"
# shellcheck source=/dev/null
[[ -f "$ROOT/.env" ]] && { set -a; source "$ROOT/.env"; set +a; }

TAG="${RELEASE_TAG:-${GIT_TAG:-${1:-}}}"
ACTION="all"
if [[ "${1:-}" =~ ^release-v ]] || [[ "${1:-}" =~ ^v[0-9] ]]; then
  TAG="$1"
  ACTION="${2:-all}"
elif [[ -n "${1:-}" && "${1:-}" != "mirror" && "${1:-}" != "up" && "${1:-}" != "all" ]]; then
  echo "usage: RELEASE_TAG=release-vX.Y.Z $0 [mirror|up|all]" >&2
  exit 2
else
  ACTION="${1:-all}"
fi

if [[ ! "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && ! "$TAG" =~ ^release-v[0-9] ]]; then
  echo "RELEASE_TAG must match vX.Y.Z or release-v*, got: ${TAG:-<empty>}" >&2
  exit 1
fi

SRC_PREFIX="${CLAW_SRC_PREFIX:-crpi-cf9vxpq3n8or17mw.cn-hangzhou.personal.cr.aliyuncs.com/passionke}"
# 8083 = Nexus docker hosted (push); 8082 = group (pull for up). Author: kejiqing
PUSH_PREFIX="${CLAW_PROMOTE_PUSH_PREFIX:-${CLAW_DST_PREFIX:-repo.550w.com:8083/passionke}}"
PULL_PREFIX="${CLAW_IMAGE_PREFIX:-repo.550w.com:8082/passionke}"
SRC_PREFIX="${SRC_PREFIX%/}"
PUSH_PREFIX="${PUSH_PREFIX%/}"
PULL_PREFIX="${PULL_PREFIX%/}"

if ! command -v skopeo >/dev/null 2>&1; then
  echo "error: skopeo required" >&2
  exit 1
fi

AUTHFILE="${AUTHFILE:-${HOME}/.docker/config.json}"
if [[ ! -f "$AUTHFILE" ]]; then
  echo "error: missing docker authfile $AUTHFILE (docker login ACR + site registry)" >&2
  exit 1
fi

# src_name -> dest_name (GHA ACR package → names release-images.sh expects). Author: kejiqing
_claw_promote_pairs() {
  cat <<'EOF'
claw-code|http-gateway-rs
claw-gateway-playground|http-gateway-playground
claw-worker-base|claw-worker-base
claw-worker-base-relaxed|claw-worker-base-relaxed
EOF
  if [[ "${CLAW_PROMOTE_LEGACY_ALIASES:-1}" == "1" ]]; then
    cat <<'EOF'
claw-code|claw-code
claw-gateway-playground|claw-gateway-playground
claw-gateway-worker|claw-gateway-worker
EOF
  fi
}

claw_promote_mirror() {
  local src_name dest_name src dest
  echo "==> promote mirror tag=${TAG}"
  echo "    src=${SRC_PREFIX}"
  echo "    push=${PUSH_PREFIX}"
  echo "    pull(up)=${PULL_PREFIX}"
  while IFS='|' read -r src_name dest_name; do
    [[ -z "$src_name" || "$src_name" =~ ^# ]] && continue
    src="${SRC_PREFIX}/${src_name}:${TAG}"
    dest="${PUSH_PREFIX}/${dest_name}:${TAG}"
    if ! skopeo inspect --authfile "$AUTHFILE" "docker://${src}" >/dev/null 2>&1; then
      echo "warn: skip missing src ${src}" >&2
      continue
    fi
    echo "==> skopeo copy ${src} → ${dest}"
    # Nexus docker hosted is plain HTTP. Author: kejiqing
    skopeo_args=(--authfile "$AUTHFILE" --format v2s2)
    if [[ "${CLAW_PROMOTE_DST_TLS_VERIFY:-0}" != "1" ]]; then
      skopeo_args+=(--dest-tls-verify=false)
    fi
    if [[ "${CLAW_PROMOTE_SRC_TLS_VERIFY:-1}" != "1" ]]; then
      skopeo_args+=(--src-tls-verify=false)
    fi
    skopeo copy "${skopeo_args[@]}" "docker://${src}" "docker://${dest}"
  done < <(_claw_promote_pairs)
  echo "OK: mirror complete tag=${TAG}"
}

claw_promote_up() {
  echo "==> git fetch + checkout ${TAG}"
  git -C "$ROOT" fetch --tags --force origin "refs/tags/${TAG}:refs/tags/${TAG}" 2>/dev/null \
    || git -C "$ROOT" fetch --tags --force origin
  git -C "$ROOT" checkout -f "tags/${TAG}" 2>/dev/null || git -C "$ROOT" checkout -f "${TAG}"
  git -C "$ROOT" describe --tags --exact-match HEAD
  export CLAW_IMAGE_PREFIX="$PULL_PREFIX"
  export RELEASE_TAG="$TAG"
  echo "==> gateway.sh up --release ${TAG} (prefix=${PULL_PREFIX})"
  "$ROOT/deploy/stack/gateway.sh" up --release "$TAG"
  echo "OK: up complete tag=${TAG}"
}

case "$ACTION" in
  mirror) claw_promote_mirror ;;
  up) claw_promote_up ;;
  all)
    claw_promote_mirror
    claw_promote_up
    ;;
  *)
    echo "usage: RELEASE_TAG=release-vX.Y.Z $0 [mirror|up|all]" >&2
    exit 2
    ;;
esac
