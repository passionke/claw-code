#!/usr/bin/env bash
# Pull bilingual GPOS manual KB from pre-release NAS snapshot (read-only; no 271 config change).
# Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${GPOS_MANUAL_KB:-$ROOT/knowledge/gpos-user-manual}"
NAS="${GPOS_KB_NAS_HOST:-admin@192.168.9.250}"
# Best-known bilingual snapshot (en+th, 281 pages). Override with GPOS_KB_NAS_VER.
VER="${GPOS_KB_NAS_VER:-/data/claw-nas/pre-claw-01/proj_271/home/.claw/project-home-versions/2026-07-13_06-48-55/home/kb}"
echo "==> rsync $NAS:$VER/ -> $DEST/"
mkdir -p "$DEST"
export GPOS_MANUAL_KB="$DEST"
rsync -az --delete \
  --exclude 'eval/' \
  --exclude 'README.md' \
  "$NAS:$VER/" "$DEST/"
python3 - <<PY
import json
import os
from pathlib import Path
root = Path(os.environ["GPOS_MANUAL_KB"])
m = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
print(m)
assert m.get("en_count", 0) > 50 and m.get("th_count", 0) > 50, m
assert (root / "en/membership/add-member-back-office.md").exists()
assert (root / "th/membership/add-member-back-office.md").exists()
print("kb fetch ok")
PY
