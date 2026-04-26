#!/usr/bin/env bash
# Boot the full anchor stack: backend + 6 MCP servers.
# Used by the Tauri tray app's process supervisor (next session).
# Standalone use: just run this script.
#
# Required env: ANTHROPIC_API_KEY (for backend + anchor-screen-mcp).
# Optional: ANCHOR_BACKEND_PORT (default 3001).

set -euo pipefail

PORT="${ANCHOR_BACKEND_PORT:-3001}"
LOG_DIR="${ANCHOR_LOG_DIR:-$HOME/.anchor/logs}"
mkdir -p "$LOG_DIR"

start_mcp() {
  local name="$1"
  local pkg="$2"
  echo "[start-stack] starting $name..."
  npx -y "$pkg" >"$LOG_DIR/$name.log" 2>&1 &
  echo $! > "$LOG_DIR/$name.pid"
}

# 1. Start the 6 MCP servers
start_mcp "anchor-activity" "@anchor/activity-mcp"
start_mcp "anchor-browser"  "@anchor/browser-mcp"
start_mcp "anchor-input"    "@anchor/input-mcp"
start_mcp "anchor-system"   "@anchor/system-mcp"
start_mcp "anchor-screen"   "@anchor/screen-mcp"
start_mcp "anchor-code"     "@anchor/code-mcp"

# 2. Start anchor-backend (will MCP-host-connect to the above on boot)
echo "[start-stack] starting anchor-backend on :$PORT..."
PORT="$PORT" MCP_ENABLED=true npx -y @anchor/backend >"$LOG_DIR/backend.log" 2>&1 &
echo $! > "$LOG_DIR/backend.pid"

echo "[start-stack] anchor stack up. Logs in $LOG_DIR/"
echo "[start-stack] open: http://localhost:$PORT"
