#!/usr/bin/env bash
# Build one image from a minimal per-Containerfile context (never the repo root).
# Context = the non-`--from` COPY sources of the Containerfile, at their repo-relative paths:
#   - directory sources: git-visible files only (tracked + untracked non-ignored), so ignored
#     target/, node_modules/, dist/ never enter the context;
#   - file sources: copied as-is (also ignored build outputs such as .linux-artifacts/release/*).
# A source missing on disk fails here, before the build starts.
# Usage: container-build.sh <container-cli> <containerfile (repo-relative)> [build args...]
# Author: kejiqing
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CLI="${1:?usage: $0 <container-cli> <containerfile> [build args...]}"
CONTAINERFILE="${2:?usage: $0 <container-cli> <containerfile> [build args...]}"
shift 2

cd "${ROOT_DIR}"
CONTAINERFILE="${CONTAINERFILE#./}"
if [[ ! -f "${CONTAINERFILE}" ]]; then
  echo "container-build: missing ${CONTAINERFILE}" >&2
  exit 1
fi

# Join backslash continuations, keep COPY lines without --from, drop flags and the destination.
copy_sources() {
  sed -e ':a' -e '/\\$/N; s/\\\n/ /; ta' "$1" \
    | awk 'toupper($1) == "COPY" {
        n = 0
        for (i = 2; i <= NF; i++) {
          if ($i ~ /^--from=/) { n = -1; break }
          if ($i ~ /^--/) continue
          a[++n] = $i
        }
        for (i = 1; i < n; i++) print a[i]
      }'
}

CTX="${ROOT_DIR}/deploy/stack/.build-ctx/$(basename "${CONTAINERFILE}")"
rm -rf "${CTX}"
trap 'rm -rf "${CTX}"' EXIT
mkdir -p "${CTX}/$(dirname "${CONTAINERFILE}")"
cp "${CONTAINERFILE}" "${CTX}/${CONTAINERFILE}"

while IFS= read -r src; do
  src="${src#./}"
  src="${src%/}"
  if [[ -d "${src}" ]]; then
    while IFS= read -r -d '' f; do
      [[ -e "${f}" ]] || continue
      mkdir -p "${CTX}/$(dirname "${f}")"
      cp -p "${f}" "${CTX}/${f}"
    done < <(git ls-files -z -co --exclude-standard -- "${src}")
  elif [[ -e "${src}" ]]; then
    mkdir -p "${CTX}/$(dirname "${src}")"
    cp -p "${src}" "${CTX}/${src}"
  else
    echo "container-build: ${CONTAINERFILE} COPY source missing: ${src}" >&2
    exit 1
  fi
done < <(copy_sources "${CONTAINERFILE}" | sort -u)

echo "==> build context ${CTX#"${ROOT_DIR}/"}: $(find "${CTX}" -type f | wc -l | tr -d ' ') files, $(du -sh "${CTX}" | cut -f1)"
"${CLI}" build -f "${CTX}/${CONTAINERFILE}" "$@" "${CTX}"
