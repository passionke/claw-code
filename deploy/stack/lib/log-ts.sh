# shellcheck shell=bash
# Deploy script timestamps (Asia/Shanghai). Author: kejiqing

claw_log_ts() {
  TZ=Asia/Shanghai date '+%Y-%m-%d %H:%M:%S %Z'
}

claw_log() {
  echo "==> [$(claw_log_ts)] $*" >&2
}
