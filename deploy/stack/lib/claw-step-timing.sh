#!/usr/bin/env bash
# Step timing for gateway.sh build / pack-deploy. Author: kejiqing

claw_now_epoch() {
  date +%s
}

claw_ts_hms() {
  date '+%Y-%m-%d %H:%M:%S'
}

claw_timing_init() {
  CLAW_TIMING_T0="$(claw_now_epoch)"
  CLAW_STEP_T0=""
  CLAW_STEP_NAME=""
  CLAW_TIMING_ROWS=()
  echo "==> timing start: $(claw_ts_hms)"
}

claw_step_end_internal() {
  [[ -n "${CLAW_STEP_NAME:-}" ]] || return 0
  local t1
  t1="$(claw_now_epoch)"
  local dur=$((t1 - CLAW_STEP_T0))
  local total=$((t1 - CLAW_TIMING_T0))
  CLAW_TIMING_ROWS+=("${dur}|${total}|${CLAW_STEP_NAME}")
  printf '==> [%s] +%ss stage %ss DONE: %s\n' "$(claw_ts_hms)" "${total}" "${dur}" "${CLAW_STEP_NAME}"
  CLAW_STEP_NAME=""
  CLAW_STEP_T0=""
}

claw_step_begin() {
  claw_step_end_internal
  CLAW_STEP_NAME="$*"
  CLAW_STEP_T0="$(claw_now_epoch)"
  local elapsed=$((CLAW_STEP_T0 - CLAW_TIMING_T0))
  echo ""
  echo "========== [$(claw_ts_hms)] +${elapsed}s | ${CLAW_STEP_NAME} =========="
  echo ""
}

claw_timing_summary() {
  claw_step_end_internal
  local t_end total
  t_end="$(claw_now_epoch)"
  total=$((t_end - CLAW_TIMING_T0))
  echo ""
  echo "========== ${CLAW_TIMING_LABEL:-timing} summary (total ${total}s, ended $(claw_ts_hms)) =========="
  if [[ ${#CLAW_TIMING_ROWS[@]} -eq 0 ]]; then
    echo "(no steps recorded)"
    echo ""
    return 0
  fi
  printf '%-8s  %-8s  %s\n' 'stage_s' 'total_s' 'step'
  local row dur at name
  for row in "${CLAW_TIMING_ROWS[@]}"; do
    dur="${row%%|*}"
    at="${row#*|}"
    at="${at%%|*}"
    name="${row#*|}"
    name="${name#*|}"
    printf '%-8s  %-8s  %s\n' "${dur}" "${at}" "${name}"
  done
  echo ""
}
