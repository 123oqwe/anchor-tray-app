#!/usr/bin/env node
/**
 * anchor — CLI entry. Spawns + supervises anchor stack.
 *
 *   anchor start [--dev]     — start anchor-backend + 6 MCP servers
 *   anchor stop              — gracefully stop everything
 *   anchor status            — show running services + restarts + log paths
 *   anchor restart <name>    — restart a single service
 *   anchor logs <name>       — tail the service's log
 *
 * v0.1: stays in foreground. Press Ctrl+C → graceful shutdown.
 * v0.2 plan: Tauri tray icon wraps this so it survives terminal close.
 */
import { registerServices, startAll, stopAll, statusAll, restart, defaultStack } from "./supervisor.js";
import { spawn } from "node:child_process";

const cmd = process.argv[2] ?? "start";
const arg = process.argv[3];

function fmtStatus(): string {
  const rows = statusAll();
  const w = (s: string, n: number) => s.padEnd(n);
  const lines = [w("SERVICE", 26) + w("STATE", 12) + w("PID", 8) + w("RESTARTS", 10) + "STARTED"];
  for (const s of rows) {
    lines.push(
      w(s.name, 26) + w(s.state, 12) +
      w(String(s.pid ?? "-"), 8) + w(String(s.restarts), 10) +
      (s.startedAt ?? "-")
    );
  }
  return lines.join("\n");
}

if (cmd === "start") {
  const useLocalDev = process.argv.includes("--dev");
  const services = defaultStack({ useLocalDev });
  registerServices(services);
  startAll();
  console.log(`⚓ anchor stack starting (${services.length} services)`);
  console.log(`Logs: ${process.env.ANCHOR_LOG_DIR ?? "~/.anchor/logs"}/`);
  setTimeout(() => { console.log("\nStatus after 5s:\n" + fmtStatus()); }, 5000);
  process.on("SIGINT", () => { console.log("\n[anchor] shutting down..."); stopAll(); setTimeout(() => process.exit(0), 1000); });
  process.on("SIGTERM", () => { stopAll(); setTimeout(() => process.exit(0), 1000); });
} else if (cmd === "status") {
  console.log(fmtStatus());
} else if (cmd === "stop") {
  stopAll();
  console.log("⚓ stopped");
} else if (cmd === "restart") {
  if (!arg) { console.error("Usage: anchor restart <service-name>"); process.exit(1); }
  const ok = restart(arg);
  console.log(ok ? `✓ restarted ${arg}` : `✗ unknown service: ${arg}`);
} else if (cmd === "logs") {
  if (!arg) { console.error("Usage: anchor logs <service-name>"); process.exit(1); }
  const path = `${process.env.ANCHOR_LOG_DIR ?? `${process.env.HOME}/.anchor/logs`}/${arg}.log`;
  spawn("tail", ["-f", path], { stdio: "inherit" });
} else {
  console.log(`Usage:
  anchor start [--dev]     start anchor stack (backend + 6 MCP servers)
  anchor stop              gracefully stop everything
  anchor status            show running services
  anchor restart <name>    restart a single service
  anchor logs <name>       tail the service's log

Services managed: anchor-backend / anchor-activity-mcp / anchor-browser-mcp /
anchor-input-mcp / anchor-system-mcp / anchor-screen-mcp / anchor-code-mcp

--dev = run from local ~/anchor-* checkouts via tsx (instead of npm packages).`);
  process.exit(cmd === "help" || cmd === "--help" ? 0 : 1);
}
