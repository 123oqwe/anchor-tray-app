#!/usr/bin/env bash
# Stop everything started by start-stack.sh.

set -euo pipefail
LOG_DIR="${ANCHOR_LOG_DIR:-$HOME/.anchor/logs}"

for pidfile in "$LOG_DIR"/*.pid; do
  [ -f "$pidfile" ] || continue
  pid=$(cat "$pidfile")
  name=$(basename "$pidfile" .pid)
  if kill -0 "$pid" 2>/dev/null; then
    echo "[stop-stack] killing $name (pid $pid)..."
    kill "$pid" 2>/dev/null || true
  fi
  rm -f "$pidfile"
done
echo "[stop-stack] done"
