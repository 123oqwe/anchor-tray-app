/**
 * Process supervisor — spawn + monitor + restart-on-crash for anchor-backend
 * + 6 anchor-*-mcp servers.
 *
 * Backoff: failed restart pauses doubling (1s, 2s, 4s, 8s, max 60s). After
 * 5 consecutive failures within 5 min, marks the service as DEGRADED and
 * stops auto-restart (manual restart required).
 *
 * Logs: each service writes to ~/.anchor/logs/<name>.log (rotated at 10MB).
 */
import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

export interface ServiceDef {
  name: string;
  command: string;
  args: string[];
  env?: Record<string, string>;
  port?: number;       // optional health-check port (HTTP GET /health)
  startupGraceMs?: number;
}

export interface ServiceStatus {
  name: string;
  pid: number | null;
  state: "starting" | "running" | "crashed" | "degraded" | "stopped";
  startedAt?: string;
  restarts: number;
  lastError?: string;
  logPath: string;
}

const LOG_DIR = process.env.ANCHOR_LOG_DIR ?? path.join(os.homedir(), ".anchor", "logs");
const MAX_LOG_BYTES = 10 * 1024 * 1024;
const FAILURE_WINDOW_MS = 5 * 60 * 1000;
const MAX_FAILURES = 5;

if (!fs.existsSync(LOG_DIR)) fs.mkdirSync(LOG_DIR, { recursive: true });

interface ServiceState {
  def: ServiceDef;
  proc: ChildProcess | null;
  status: ServiceStatus;
  restartFailures: { ts: number }[];
  backoffMs: number;
  shuttingDown: boolean;
}

const services = new Map<string, ServiceState>();

function logPath(name: string): string {
  return path.join(LOG_DIR, `${name}.log`);
}

function rotateIfNeeded(p: string): void {
  try {
    const stat = fs.statSync(p);
    if (stat.size > MAX_LOG_BYTES) {
      fs.renameSync(p, p + ".1");
    }
  } catch { /* file may not exist */ }
}

function start(def: ServiceDef): void {
  rotateIfNeeded(logPath(def.name));
  const out = fs.openSync(logPath(def.name), "a");
  const err = fs.openSync(logPath(def.name), "a");
  const proc = spawn(def.command, def.args, {
    env: { ...process.env, ...(def.env ?? {}) },
    stdio: ["ignore", out, err],
    detached: false,
  });

  const state = services.get(def.name);
  if (!state) return;
  state.proc = proc;
  state.status.pid = proc.pid ?? null;
  state.status.state = "starting";
  state.status.startedAt = new Date().toISOString();
  state.status.logPath = logPath(def.name);

  setTimeout(() => {
    if (state.proc === proc && !state.shuttingDown) {
      state.status.state = "running";
    }
  }, def.startupGraceMs ?? 2000);

  proc.on("exit", (code, signal) => {
    if (state.shuttingDown) return;
    state.proc = null;
    state.status.pid = null;
    state.status.state = "crashed";
    state.status.lastError = `exited code=${code} signal=${signal}`;
    state.restartFailures.push({ ts: Date.now() });
    state.restartFailures = state.restartFailures.filter(f => Date.now() - f.ts < FAILURE_WINDOW_MS);

    if (state.restartFailures.length >= MAX_FAILURES) {
      state.status.state = "degraded";
      console.error(`[supervisor] ${def.name} DEGRADED — ${MAX_FAILURES} crashes in ${FAILURE_WINDOW_MS / 60000}min. Manual restart required.`);
      return;
    }
    console.warn(`[supervisor] ${def.name} crashed (${state.status.lastError}) — restarting in ${state.backoffMs}ms`);
    setTimeout(() => {
      state.status.restarts++;
      state.backoffMs = Math.min(state.backoffMs * 2, 60_000);
      start(def);
    }, state.backoffMs);
  });
}

export function registerServices(defs: ServiceDef[]): void {
  for (const def of defs) {
    services.set(def.name, {
      def, proc: null,
      status: { name: def.name, pid: null, state: "stopped", restarts: 0, logPath: logPath(def.name) },
      restartFailures: [],
      backoffMs: 1000,
      shuttingDown: false,
    });
  }
}

export function startAll(): void {
  for (const state of services.values()) {
    state.shuttingDown = false;
    state.backoffMs = 1000;
    start(state.def);
  }
}

export function stopAll(): void {
  for (const state of services.values()) {
    state.shuttingDown = true;
    if (state.proc && !state.proc.killed) {
      try { state.proc.kill("SIGTERM"); } catch {}
    }
    state.status.state = "stopped";
    state.status.pid = null;
  }
}

export function statusAll(): ServiceStatus[] {
  return Array.from(services.values()).map(s => s.status);
}

export function restart(name: string): boolean {
  const state = services.get(name);
  if (!state) return false;
  state.shuttingDown = true;
  if (state.proc && !state.proc.killed) try { state.proc.kill("SIGTERM"); } catch {}
  state.shuttingDown = false;
  state.restartFailures = [];
  state.backoffMs = 1000;
  start(state.def);
  return true;
}

// ── Default anchor stack ───────────────────────────────────────────────────

export function defaultStack(opts: { backendPort?: number; useLocalDev?: boolean } = {}): ServiceDef[] {
  const port = opts.backendPort ?? 3001;
  const env = {
    PORT: String(port),
    MCP_ENABLED: "true",
    ...(process.env.ANTHROPIC_API_KEY ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY } : {}),
    ...(process.env.OPENAI_API_KEY ? { OPENAI_API_KEY: process.env.OPENAI_API_KEY } : {}),
  };

  if (opts.useLocalDev) {
    // Dev mode: run from local checkouts via tsx
    const HOME = os.homedir();
    return [
      { name: "anchor-backend",      command: "pnpm", args: ["tsx", `${HOME}/anchor-backend/server/index.ts`], env, port },
      { name: "anchor-activity-mcp", command: "npx",  args: ["tsx", `${HOME}/anchor-activity-mcp/src/index.ts`], env },
      { name: "anchor-browser-mcp",  command: "npx",  args: ["tsx", `${HOME}/anchor-browser-mcp/src/index.ts`], env },
      { name: "anchor-input-mcp",    command: "npx",  args: ["tsx", `${HOME}/anchor-input-mcp/src/index.ts`], env },
      { name: "anchor-system-mcp",   command: "npx",  args: ["tsx", `${HOME}/anchor-system-mcp/src/index.ts`], env },
      { name: "anchor-screen-mcp",   command: "npx",  args: ["tsx", `${HOME}/anchor-screen-mcp/src/index.ts`], env },
      { name: "anchor-code-mcp",     command: "npx",  args: ["tsx", `${HOME}/anchor-code-mcp/src/index.ts`], env },
    ];
  }

  // Prod mode: from npm
  return [
    { name: "anchor-backend",      command: "npx", args: ["-y", "@anchor/backend"], env, port },
    { name: "anchor-activity-mcp", command: "npx", args: ["-y", "@anchor/activity-mcp"], env },
    { name: "anchor-browser-mcp",  command: "npx", args: ["-y", "@anchor/browser-mcp"], env },
    { name: "anchor-input-mcp",    command: "npx", args: ["-y", "@anchor/input-mcp"], env },
    { name: "anchor-system-mcp",   command: "npx", args: ["-y", "@anchor/system-mcp"], env },
    { name: "anchor-screen-mcp",   command: "npx", args: ["-y", "@anchor/screen-mcp"], env },
    { name: "anchor-code-mcp",     command: "npx", args: ["-y", "@anchor/code-mcp"], env },
  ];
}
