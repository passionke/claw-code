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
if [ -d "$fc_tools_src/tap-runtime/bin" ] && [ -f "$fc_tools_src/tap-runtime/bin/claude-tap" ]; then
  mkdir -p "$CLAW_BIN/tap-runtime/bin" "$CLAW_BIN/tap-runtime/lib"
  cp "$fc_tools_src/tap-runtime/bin/"* "$CLAW_BIN/tap-runtime/bin/" 2>/dev/null || true
  cp -a "$fc_tools_src/tap-runtime/lib/site-packages" "$CLAW_BIN/tap-runtime/lib/" 2>/dev/null \
    || cp -a "$fc_tools_src/tap-runtime/lib/"* "$CLAW_BIN/tap-runtime/lib/" 2>/dev/null || true
  chmod +x "$CLAW_BIN/tap-runtime/bin/"* 2>/dev/null || true
  if [ ! -x "$CLAW_BIN/claude-tap" ]; then
    cat > "$CLAW_BIN/claude-tap" <<'EOF'
#!/bin/sh
export PYTHONPATH="/tmp/claw-fc-bin/tap-runtime/lib/site-packages:${PYTHONPATH:-}"
exec /tmp/claw-fc-bin/tap-runtime/bin/python3.12 /tmp/claw-fc-bin/tap-runtime/bin/claude-tap "$@"
EOF
    chmod +x "$CLAW_BIN/claude-tap"
  fi
fi
export PATH="$CLAW_BIN:$PATH"
