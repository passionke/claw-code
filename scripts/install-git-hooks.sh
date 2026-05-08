#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
src_hook="${repo_root}/.githooks/pre-push"
dst_hook="${repo_root}/.git/hooks/pre-push"

if [[ ! -f "${src_hook}" ]]; then
  echo "missing source hook: ${src_hook}" >&2
  exit 1
fi

mkdir -p "${repo_root}/.git/hooks"
cp "${src_hook}" "${dst_hook}"
chmod +x "${dst_hook}"

echo "installed pre-push hook -> ${dst_hook}"
