#!/usr/bin/env bash
# Sync repo KB seed -> project home/kb (dev/pre). Does NOT git pull.
# Author: kejiqing
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$ROOT/knowledge/gpos-user-manual"
DEST="${1:-}"
if [[ -z "$DEST" ]]; then
  echo "Usage: $0 /path/to/proj_N/home/kb" >&2
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
