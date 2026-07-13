#!/usr/bin/env bash
# Sync local KB cache -> project home/kb (dev/pre). Does NOT git pull.
# Source defaults to knowledge/gpos-user-manual (gitignored); override with GPOS_MANUAL_KB.
# Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="${GPOS_MANUAL_KB:-$ROOT/knowledge/gpos-user-manual}"
DEST="${1:-}"
if [[ -z "$DEST" ]]; then
  echo "Usage: $0 /path/to/proj_N/home/kb" >&2
  exit 2
fi
if [[ ! -d "$SRC" ]]; then
  echo "KB source missing: $SRC (crawl first or set GPOS_MANUAL_KB)" >&2
  exit 2
fi
mkdir -p "$DEST"
rsync -a --delete \
  --exclude 'eval/' \
  --exclude 'README.md' \
  --exclude '.git/' \
  "$SRC/" "$DEST/"
echo "Synced $SRC -> $DEST"
ls "$DEST/index.md" "$DEST/manifest.json"
