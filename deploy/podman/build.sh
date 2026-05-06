#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

podman build \
  -f "${ROOT_DIR}/deploy/podman/Containerfile.gateway-rs" \
  -t claw-gateway-rs:local \
  "${ROOT_DIR}"

echo "Built image: claw-gateway-rs:local"
