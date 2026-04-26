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

## Two ways to run

### A. Node CLI (works today, no Rust needed)

```bash
pnpm install
pnpm build
ANTHROPIC_API_KEY=sk-... node dist/cli.js start --dev   # foreground
node dist/cli.js status     # in another terminal
node dist/cli.js logs anchor-backend
```

### B. Tauri tray app (build yourself; needs Rust)

Tauri scaffold is **complete** in `src-tauri/`. To build:

```bash
# 1. Install Rust (one-time)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# 2. Install Tauri CLI (one-time)
cargo install tauri-cli --version "^2"

# 3. Generate icons (one-time — see src-tauri/icons/README.md)

# 4. Run in dev mode
cargo tauri dev    # builds + launches; tray icon appears in menubar

# 5. Build distributable
cargo tauri build  # → src-tauri/target/release/bundle/{dmg, app, msi, deb, appimage}
```

### Distribution caveats (post-launch work)

- **macOS .dmg**: needs Apple Developer ID for code signing (~$99/year) + notarization. Without these, the app shows scary "unidentified developer" warning.
- **Windows .msi**: needs EV code-signing certificate ($300+/year) to avoid SmartScreen warnings.
- **Linux .AppImage / .deb**: no signing required, but may need additional packaging for distro repos.

For "ship to developers" today, **path A** (Node CLI) is enough. Path B (signed Tauri binary) is post-launch work.

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
