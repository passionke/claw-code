#!/usr/bin/env bash
# Gitea Actions on neurogate starts Jenkins pack and waits.
# Caller then runs up --release. Do not also use a Gitea webhook (double fire).
# Author: kejiqing
# Usage: home-neurogate-pack-via-jenkins.sh GIT_REF=<sha>|GIT_TAG=vX.Y.Z
# Exit 20: GIT_REF path gate published neither image (caller should skip up).
set -euo pipefail

SPEC="${1:-}"
if [[ "$SPEC" =~ ^GIT_REF=.+ ]]; then
  MODE=ref
  VALUE="${SPEC#GIT_REF=}"
elif [[ "$SPEC" =~ ^GIT_TAG=v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  MODE=tag
  VALUE="${SPEC#GIT_TAG=}"
else
  echo "usage: $0 GIT_REF=<sha>|GIT_TAG=vX.Y.Z" >&2
  exit 2
fi

JENKINS_URL="${JENKINS_URL:-https://jenkins.home.passionke.top/}"
CLI_JAR="${JENKINS_CLI_JAR:-${HOME}/.local/share/jenkins/jenkins-cli.jar}"
JENKINS_ENV_FILE="${JENKINS_ENV_FILE:-${HOME}/.secrets/jenkins.env}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
PREFIX="${CLAW_IMAGE_PREFIX:-nora.home.passionke.top/passionke}"
DOCKER_CONFIG_JSON="${CLAW_DOCKER_CONFIG:-${ROOT}/deploy/stack/.host-secrets/docker-config.json}"

if [[ ! -s "$CLI_JAR" ]]; then
  mkdir -p "$(dirname "$CLI_JAR")"
  echo "==> download jenkins-cli.jar → ${CLI_JAR}"
  curl -fsSL -o "$CLI_JAR" "${JENKINS_URL%/}/jnlpJars/jenkins-cli.jar"
fi

TMP_AUTH=""
cleanup() {
  if [[ -n "${TMP_AUTH}" && -f "${TMP_AUTH}" ]]; then
    rm -f "${TMP_AUTH}"
  fi
}
trap cleanup EXIT

AUTH_FILE="${JENKINS_AUTH_FILE:-}"
if [[ -z "$AUTH_FILE" ]]; then
  if [[ -f "${HOME}/.local/share/jenkins/auth" ]]; then
    AUTH_FILE="${HOME}/.local/share/jenkins/auth"
  elif [[ -f "$JENKINS_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    set -a
    # shellcheck source=/dev/null
    source "$JENKINS_ENV_FILE"
    set +a
    TMP_AUTH="$(mktemp)"
    chmod 600 "$TMP_AUTH"
    printf '%s:%s\n' "${JENKINS_ADMIN_USER:?}" "${JENKINS_ADMIN_PASSWORD:?}" >"$TMP_AUTH"
    AUTH_FILE="$TMP_AUTH"
  fi
fi
if [[ -z "$AUTH_FILE" || ! -f "$AUTH_FILE" ]]; then
  echo "error: Jenkins auth missing (${HOME}/.local/share/jenkins/auth or ${JENKINS_ENV_FILE})" >&2
  exit 1
fi

JAVA_BIN="${JENKINS_JAVA:-}"
if [[ -z "$JAVA_BIN" ]]; then
  if command -v java >/dev/null 2>&1; then
    JAVA_BIN="$(command -v java)"
  else
    echo "error: java not found on neurogate runner" >&2
    exit 1
  fi
fi

jcli() {
  "$JAVA_BIN" -jar "$CLI_JAR" -s "$JENKINS_URL" -http -auth @"$AUTH_FILE" "$@"
}

image_ready() {
  local ref="$1"
  if command -v skopeo >/dev/null 2>&1 && [[ -f "$DOCKER_CONFIG_JSON" ]]; then
    skopeo inspect --authfile "$DOCKER_CONFIG_JSON" "docker://${ref}" >/dev/null 2>&1
    return $?
  fi
  if command -v docker >/dev/null 2>&1; then
    docker manifest inspect "$ref" >/dev/null 2>&1
    return $?
  fi
  echo "error: need skopeo+authfile or docker to probe ${ref}" >&2
  return 2
}

JOBS=(claw-code-gateway-nora claw-code-playground-nora)
echo "==> jenkins pack ${SPEC}"

pids=()
logs=()
for job in "${JOBS[@]}"; do
  log="$(mktemp)"
  logs+=("$log")
  echo "==> start ${job} -p ${SPEC}"
  jcli build "$job" -s -v -p "$SPEC" >"$log" 2>&1 &
  pids+=("$!")
done

fail=0
for i in "${!pids[@]}"; do
  if ! wait "${pids[$i]}"; then
    echo "error: ${JOBS[$i]} failed" >&2
    fail=1
  fi
  echo "----- ${JOBS[$i]} (tail) -----"
  tail -n 30 "${logs[$i]}" || true
  rm -f "${logs[$i]}"
done
if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

if [[ "$MODE" == "tag" ]]; then
  echo "==> jenkins pack ok tag=${VALUE}"
  exit 0
fi

SHA12="$(git -C "$ROOT" rev-parse --short=12 HEAD 2>/dev/null || true)"
if [[ ${#SHA12} -lt 12 ]]; then
  SHA12="${VALUE:0:12}"
fi
echo "==> jenkins pack ok; probe sha-${SHA12}"
ready=0
for name in claw-code claw-gateway-playground; do
  ref="${PREFIX}/${name}:sha-${SHA12}"
  if image_ready "$ref"; then
    echo "  published: ${ref}"
    ready=1
  else
    echo "  absent: ${ref} (path gate skip or not this component)"
  fi
done
if [[ "$ready" -eq 0 ]]; then
  echo "==> neither image published for sha-${SHA12}; skip neurogate up"
  exit 20
fi
exit 0
