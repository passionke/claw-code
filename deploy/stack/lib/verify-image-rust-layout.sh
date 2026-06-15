#!/usr/bin/env bash
# Verify in-image compile Containerfiles match cross-tree Cargo path-deps.
# Contract (from repo Cargo.toml path = "../../../rust|sandbox/..."):
#   /rust/crates/*  <->  /sandbox/crates/*
# Worker/gateway-rs builder stages must COPY both trees at those absolute paths.
# Author: kejiqing
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${ROOT}"

FILES=(
  deploy/stack/Containerfile.gateway-rs
  deploy/stack/Containerfile.gateway-worker
)

need_in_file() {
  local file="$1"
  local needle="$2"
  if ! grep -qF "${needle}" "${file}"; then
    echo "verify-image-rust-layout: missing '${needle}' in ${file}" >&2
    exit 1
  fi
}

for f in "${FILES[@]}"; do
  [[ -f "${f}" ]] || { echo "missing ${f}" >&2; exit 1; }
  need_in_file "${f}" "WORKDIR /rust"
  need_in_file "${f}" "COPY rust/Cargo.toml /rust/Cargo.toml"
  need_in_file "${f}" "COPY rust/crates/ /rust/crates/"
  need_in_file "${f}" "COPY sandbox/Cargo.toml sandbox/Cargo.lock /sandbox/"
  need_in_file "${f}" "COPY sandbox/crates/ /sandbox/crates/"
done

echo "verify-image-rust-layout: ok (${#FILES[@]} containerfiles)"
