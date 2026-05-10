#!/usr/bin/env sh
# Idle worker entrypoint for http-gateway-rs container pool (sleep until `docker exec`).
# Author: kejiqing
set -eu
cleanup() {
  if [ -n "${child:-}" ]; then
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
  fi
  exit 0
}
trap cleanup TERM INT
sleep infinity &
child=$!
wait "$child"
