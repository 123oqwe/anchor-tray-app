# anchor-tray-app supervisor state — snapshot 2026-04-28

**Two parallel supervisors exist in this repo. They disagree.** This doc
inventories which is the production path today and what's broken in each.

## TL;DR

| | TS supervisor (`src/supervisor.ts` + `src/cli.ts`) | Rust supervisor (`src-tauri/src/main.rs`) |
|---|---|---|
| Status | **Production today** (CLI: `anchor start`) | v0.2 plan — Tauri tray UI |
| Services managed | anchor-backend + admin-backend + security (3) ✅ | anchor-backend + 7 MCP (8) ❌ |
| Crash restart | ✅ exponential backoff, DEGRADED after 5 fails | ❌ none |
| Log rotation | ✅ 10MB | ❌ none |
| Autostart on login | ❌ user must run manually | ❌ no Tauri autostart plugin yet |
| Build prereq | tsx (already installed) | **Rust toolchain (not installed locally)** |
| Build status | ✅ runs today | ⚠️ unknown — never `cargo build`-ed in this session |

## What changed today (2026-04-28)

- **TS side:** `defaultStack()` updated from 1 service → 3. Now spawns
  anchor-backend (:3001) + anchor-admin-backend (:3002) + anchor-security
  (:3004). Each gets correct env (port, ANCHOR_DB_PATH, cross-service
  tokens). MCP servers intentionally excluded — they need stdin piped for
  JSON-RPC, which the supervisor can't provide.
- **CLI help text** updated to match.
- **Rust side:** unchanged — still has the 2026-04-26 bugs documented below.

## Known bugs in the Rust supervisor

These prevent `anchor-tray-app` from being a real production tray app on
macOS until they're fixed (which requires installing Rust):

1. **Spawns MCP servers as supervised children.** Lines 49-69 list 7
   anchor-*-mcp servers. These are stdio JSON-RPC peers and must be spawned
   with stdin piped — supervising them with the current Rust code (no stdin
   pipe → EOF immediately) makes them exit at boot and never restart since
   there's no crash-restart logic. The TS supervisor's design comment
   already states this — Rust code drifted.

2. **Missing the Sprint 5/6 backends.** No anchor-admin-backend, no
   anchor-security in the Rust service list.

3. **No crash-restart.** If anchor-backend dies, Rust supervisor leaves it
   dead. TS supervisor has full backoff + 5-fail-DEGRADED logic.

4. **No log rotation.** Logs grow unbounded. TS supervisor rotates at 10MB.

5. **No autostart plugin.** `tauri-plugin-autostart` is the standard
   v2.x plugin for this — not in `Cargo.toml`.

## What needs to happen to ship v0.1 tray app

User-side action required:

```bash
# Install Rust (one-time, ~5 min)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Then in this session I can:
cd anchor-tray-app/src-tauri
cargo build              # verify the existing main.rs even compiles
```

Then I rewrite `main.rs` to:
- Mirror the TS supervisor's 3-service list (no MCP)
- Add crash-restart with the same backoff logic
- Add `tauri-plugin-autostart` and configure for macOS LaunchAgent
- Wire the existing tray menu to the new supervisor functions

Estimated work after Rust is installed: ~half day. The Rust
infrastructure (tray menu, log dir, env) is already there — only the
supervisor body needs replacement.

## What works today without Rust

`anchor start --dev` (the TS CLI path) IS the production-quality
single-engineer install today. It just doesn't have a menu-bar UI — it
runs in a foreground terminal and dies on Ctrl+C. For dev / power users
this is fine. For non-technical beta users it's not.

## Decision pending

Before doing the Rust rewrite, decide: keep both supervisors (CLI for
power users + Tauri for non-tech) or kill one?

- **Kill TS, do everything in Rust:** pure but loses the dev convenience
  of `anchor start --dev` for terminal users.
- **Kill Rust, ship CLI + electron-style instead:** loses the ~10MB
  Tauri size advantage.
- **Keep both, share the service definitions:** what we have now. Risk:
  drift (which is exactly what happened before today's update).

My recommendation: **keep both, but extract the service list into a
shared JSON file** (`stack.json`) that both supervisors read. Eliminates
the drift class of bugs entirely.
