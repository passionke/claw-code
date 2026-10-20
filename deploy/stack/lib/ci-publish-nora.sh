#!/usr/bin/env bash
# Author: kejiqing
#
# Manual all-in-one: Gateway then playground. Prefer split Jobs
# claw-code-gateway-nora / claw-code-playground-nora for CI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
bash "$ROOT/deploy/stack/lib/ci-publish-gateway-nora.sh"
bash "$ROOT/deploy/stack/lib/ci-publish-playground-nora.sh"
