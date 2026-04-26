# anchor-tray-app

System-tray companion app for [anchor](https://github.com/123oqwe/anchor-backend). One download → starts anchor-backend + 7 anchor-*-mcp servers + shows tray icon + auto-starts on boot.

**Status: scaffold (v0.0.1)**. This repo currently holds the architecture + design + boot scripts. Tauri build is the next session's work.

## What it bundles

| Component | Source |
|-----------|------|
| anchor-backend | git submodule or npm `@anchor/backend` |
| anchor-activity-mcp | npm `@anchor/activity-mcp` |
| anchor-browser-mcp  | npm `@anchor/browser-mcp` |
| anchor-input-mcp    | npm `@anchor/input-mcp` |
| anchor-system-mcp   | npm `@anchor/system-mcp` |
| anchor-screen-mcp   | npm `@anchor/screen-mcp` |
| anchor-code-mcp     | npm `@anchor/code-mcp` |
| anchor-shell-mcp    | npm `@anchor/shell-mcp` (CLI-first, token-efficient) |

## What it provides

1. **Tray icon** — anchor presence in macOS menubar / Windows system tray / Linux notification area.
2. **Auto-start at boot** — launchd (Mac) / Task Scheduler (Win) / systemd user service (Linux).
3. **MCP server lifecycle** — spawns + supervises 7 MCP servers, restarts on crash.
4. **One-click open** — clicks tray → opens anchor's web UI on `http://localhost:3001/`.
5. **Quit / restart / status** — basic controls in tray menu.

## Architecture

```
                    ┌────────────────────┐
                    │  anchor-tray-app   │  Tauri (Rust + WebView)
                    │  (system tray)     │
                    └─────────┬──────────┘
                              │ spawns + supervises
        ┌──────────┬──────────┼──────────┬──────────┬──────────┐
        ▼          ▼          ▼          ▼          ▼          ▼
   anchor-      anchor-    anchor-    anchor-    anchor-    anchor-
   backend      activity   browser    input      system     screen
   :3001        -mcp       -mcp       -mcp       -mcp       -mcp
                  ↑           ↑           ↑           ↑           ↑
                  └───────────┴── stdio MCP ──┴───────────┴───────┘
                  (anchor-backend's MCP host connects to all 6)
```

## Build (next session)

```bash
# Prereqs: Rust + Tauri CLI
cargo install tauri-cli

# Scaffold
cargo tauri init

# Dev
cargo tauri dev

# Build for distribution
cargo tauri build  # → .dmg / .msi / .AppImage
```

## Why Tauri (not Electron)?

| | Tauri | Electron |
|--|-------|----------|
| Bundle size | ~5-10 MB | ~150 MB |
| RAM | ~50 MB | ~200 MB |
| Tray API | Native | Needs nativeTheme polyfill |
| Auto-update | Built-in | Squirrel.Mac / NSIS |
| Cross-platform | Native (Rust) | Node + Chromium |

Anchor's tray companion should be lightweight (it bundles 7 Node processes already). Tauri keeps the wrapper minimal.

## What's NOT in this scaffold yet

- Tauri project files (Cargo.toml, src-tauri/, etc) — next session
- Process supervisor logic (spawn anchor-backend + 7 MCP servers, monitor, restart)
- Tray menu (status / open UI / quit)
- Auto-start at boot per-OS
- Code signing (Apple Developer ID for .dmg, EV cert for .msi)
- Auto-update infrastructure

## License

MIT
