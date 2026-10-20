#!/usr/bin/env bash
# Shared Nora+ACR publish helpers for Gateway/Admin split Jobs. Author: kejiqing
# Sourced by ci-publish-gateway-nora.sh / ci-publish-playground-nora.sh.
# shellcheck shell=bash

# Formal v* / release-v*: push RELEASE_TAG only.
# Fast sha-*: push RELEASE_TAG + :latest (neurogate-up pins :latest per image).
claw_nora_push_dest_tags() {
  local image="$1"
  if [[ "$RELEASE_TAG" =~ ^sha-[0-9a-f]{7,40}$ ]]; then
    printf '%s\n' "${image}:${RELEASE_TAG}" "${image}:latest"
  else
    printf '%s\n' "${image}:${RELEASE_TAG}"
  fi
}

claw_nora_skopeo_push_package() {
  local local_ref="$1"
  local nora_image="$2"
  local acr_image="$3"
  local -a nora_tags=() acr_tags=()
  mapfile -t nora_tags < <(claw_nora_push_dest_tags "$nora_image")
  mapfile -t acr_tags < <(claw_nora_push_dest_tags "$acr_image")
  chmod +x "$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh"
  "$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" "${local_ref}" "${nora_tags[@]}"
  "$ROOT/deploy/stack/lib/ci-push-acr-skopeo.sh" "${local_ref}" "${acr_tags[@]}"
}

claw_nora_publish_preflight() {
  stage "1/4" "preflight publish ${RELEASE_TAG} (sha-${SHA12}) → Nora ${NEXUS_PUSH_REGISTRY}/${NEXUS_NS} + ACR ${ACR_REGISTRY} REGION=${REGION} platform=${PLATFORM} arch=${TARGETARCH}"

  if ! command -v skopeo >/dev/null 2>&1; then
    echo "preflight: skopeo required (only skopeo v2s2 push; no docker push)" >&2
    exit 1
  fi
  if ! command -v docker >/dev/null 2>&1; then
    echo "preflight: docker CLI required for build/login" >&2
    exit 1
  fi

  V2_CODE="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 10 \
    "https://${NEXUS_PUSH_REGISTRY}/v2/" || echo 000)"
  if [[ "$V2_CODE" != "401" ]]; then
    echo "preflight: https://${NEXUS_PUSH_REGISTRY}/v2/ returned HTTP ${V2_CODE}, want 401" >&2
    echo "preflight: Nora must challenge (docker_anon_pull=false). Fix registry ops; do not patch this script." >&2
    exit 1
  fi

  echo "$NEXUS_PASSWORD" | docker login "$NEXUS_PUSH_REGISTRY" -u "$NEXUS_USER" --password-stdin
  AUTH_B64="$(printf '%s:%s' "$NEXUS_USER" "$NEXUS_PASSWORD" | base64 | tr -d '\n')"
  UP_CODE="$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 10 \
    -X POST -H "Authorization: Basic ${AUTH_B64}" -H "Content-Length: 0" \
    "https://${NEXUS_PUSH_REGISTRY}/v2/${NEXUS_NS}/claw-code/blobs/uploads/" || echo 000)"
  if [[ "$UP_CODE" != "202" ]]; then
    echo "preflight: blob upload initiate returned HTTP ${UP_CODE}, want 202 (bad creds or Nora write auth)" >&2
    exit 1
  fi

  echo "$ACR_PASSWORD" | docker login "$ACR_LOGIN_HOST" -u "$ACR_USERNAME" --password-stdin
  echo "preflight: ok (Nora v2=401 upload-init=202; ACR login ${ACR_LOGIN_HOST}; skopeo present)"
}

claw_nora_publish_init_env() {
  NEXUS_PUSH_REGISTRY="${NEXUS_PUSH_REGISTRY:-nora.home.passionke.top}"
  NEXUS_NS="${NEXUS_NS:-passionke}"
  ACR_REGISTRY="${ACR_REGISTRY:-${CLAW_ACR_IMAGE_PREFIX:-$(claw_default_acr_image_prefix)}}"
  ACR_REGISTRY="${ACR_REGISTRY%/}"
  REGION="${REGION:-china}"
  : "${NEXUS_USER:?NEXUS_USER is required}"
  : "${NEXUS_PASSWORD:?NEXUS_PASSWORD is required}"
  : "${ACR_USERNAME:?ACR_USERNAME is required (dual-write Nora+ACR)}"
  : "${ACR_PASSWORD:?ACR_PASSWORD is required (dual-write Nora+ACR)}"
  ACR_LOGIN_HOST="${ACR_REGISTRY%%/*}"
  if [[ -z "$ACR_LOGIN_HOST" || "$ACR_LOGIN_HOST" == "$ACR_REGISTRY" ]]; then
    echo "ACR_REGISTRY must be host/namespace (e.g. crpi-….aliyuncs.com/passionke), got: ${ACR_REGISTRY}" >&2
    exit 1
  fi

  export CLAW_DISABLE_SCCACHE="${CLAW_DISABLE_SCCACHE:-1}"
  export CLAW_LINUX_COMPILE_PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM:-linux/amd64}"
  case "${CLAW_LINUX_COMPILE_PLATFORM}" in
    *arm64*|*aarch64*) export TARGETARCH="${TARGETARCH:-arm64}" ;;
    *) export TARGETARCH="${TARGETARCH:-amd64}" ;;
  esac
  PLATFORM="${CLAW_LINUX_COMPILE_PLATFORM}"

  if [[ "$REGION" != "china" && "$REGION" != "else" ]]; then
    echo "REGION must be china or else, got: $REGION" >&2
    exit 1
  fi

  RELEASE_TAG="${RELEASE_TAG:-${GIT_TAG:-}}"
  if [[ -z "$RELEASE_TAG" ]]; then
    RELEASE_TAG="$(git -C "$ROOT" describe --tags --exact-match HEAD 2>/dev/null || true)"
  fi
  if [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ \
    && ! "$RELEASE_TAG" =~ ^release-v[0-9] \
    && ! "$RELEASE_TAG" =~ ^sha-[0-9a-f]{7,40}$ ]]; then
    echo "RELEASE_TAG must match vX.Y.Z, release-v*, or sha-<hex>, got: ${RELEASE_TAG:-<empty>}" >&2
    exit 1
  fi

  SHA12="$(git -C "$ROOT" rev-parse --short=12 HEAD)"
  CONTAINER_CLI="$(claw_container_runtime_cli)" || exit 1

  if [[ "$REGION" == "china" ]]; then
    export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-1}"
    export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-1}"
    export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-1}"
    REG="${CONTAINER_BASE_REGISTRY:-docker.m.daocloud.io}"
  else
    export CLAW_USE_CN_APT_MIRROR="${CLAW_USE_CN_APT_MIRROR:-0}"
    export CLAW_USE_CN_CRATES_MIRROR="${CLAW_USE_CN_CRATES_MIRROR:-0}"
    export CLAW_USE_CN_RUST_MIRROR="${CLAW_USE_CN_RUST_MIRROR:-0}"
    REG="${CONTAINER_BASE_REGISTRY:-docker.io}"
  fi
  REG="${REG%/}"
  DEBIAN_IMAGE="${REG}/library/debian:bookworm-slim"
  NODE_IMAGE="${REG}/library/node:20-alpine"
  APT_CN="$CLAW_USE_CN_APT_MIRROR"
}

stage() {
  echo "==> [$1] $2"
}
