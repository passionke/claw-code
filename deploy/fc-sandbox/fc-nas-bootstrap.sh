# Idempotent: stage claw + ttyd from NAS into writable bin dir (FC code-interpreter template).
# Author: kejiqing
# Prepended to FC exec scripts; TOOLS_REL via CLAW_FC_NAS_TOOLS_REL (default .claw-fc-tools at workspace root).
set -e
TOOLS_REL="${CLAW_FC_NAS_TOOLS_REL:-.claw-fc-tools}"
CLAW_BIN="${CLAW_FC_BIN_DIR:-/tmp/claw-fc-bin}"
mkdir -p "$CLAW_BIN"
fc_tools_src=""
for d in \
  "/claw_host_root/${TOOLS_REL}" \
  "/claw_ds/${TOOLS_REL}" \
  "/claw_host_root/../../../${TOOLS_REL}" \
  "/claw_ds/../../${TOOLS_REL}"; do
  if [ -f "$d/claw" ] && [ -f "$d/ttyd" ]; then
    fc_tools_src="$d"
    break
  fi
done
if [ -z "$fc_tools_src" ]; then
  echo "fc nas bootstrap: missing claw/ttyd under ${TOOLS_REL} on NAS (check nasConfig mount + ${TOOLS_REL} on NAS)" >&2
  exit 127
fi
for name in claw ttyd; do
  if [ ! -x "$CLAW_BIN/$name" ] || [ "$CLAW_BIN/$name" -ot "$fc_tools_src/$name" ]; then
    cp "$fc_tools_src/$name" "$CLAW_BIN/$name"
    chmod +x "$CLAW_BIN/$name"
  fi
done
export PATH="$CLAW_BIN:$PATH"
