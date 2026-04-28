/**
 * Process supervisor — spawn + monitor + restart-on-crash for anchor-backend.
 *
 * MCP servers are NOT supervised here. They are stdio JSON-RPC peers and must
 * be spawned by anchor-backend's MCP host (server/integrations/mcp/registry.ts)
 * with stdin piped — supervising them with stdio:'ignore' makes them read EOF
 * and exit immediately, triggering an infinite restart loop.
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

// Single source of truth for service definitions: ../stack.json
// (Both this TS supervisor and src-tauri/src/main.rs read it.)
interface StackJson {
  services: Array<{
    name: string;
    port: number;
    dev: { command: string; args: string[] };
    prod: { command: string; args: string[] };
    env_keys: string[];
    startup_grace_ms?: number;
  }>;
}

function loadStack(): StackJson {
  const candidates = [
    path.join(process.cwd(), "stack.json"),
    path.join(process.cwd(), "..", "stack.json"),
    "/Users/guanjieqiao/anchor-tray-app/stack.json",
  ];
  for (const p of candidates) {
    try { return JSON.parse(fs.readFileSync(p, "utf8")); } catch { /* try next */ }
  }
  throw new Error(`stack.json not found in: ${candidates.join(", ")}`);
}

function expandHome(s: string): string {
  return s.replace(/\$\{HOME\}/g, os.homedir());
}

export function defaultStack(opts: { backendPort?: number; useLocalDev?: boolean } = {}): ServiceDef[] {
  const stack = loadStack();
  const HOME = os.homedir();
  const ANCHOR_DB = `${HOME}/anchor-backend/server/infra/anchor.db`;

  const buildEnv = (svc: StackJson["services"][number]): Record<string, string> => {
    const out: Record<string, string> = {};
    for (const k of svc.env_keys) {
      switch (k) {
        case "PORT":
          out.PORT = String(opts.backendPort && svc.name === "anchor-backend" ? opts.backendPort : svc.port);
          break;
        case "ANCHOR_DB_PATH":
          out.ANCHOR_DB_PATH = ANCHOR_DB;
          break;
        case "MCP_ENABLED":
          out.MCP_ENABLED = "true";
          break;
        case "SECURITY_API_URL":
          out.SECURITY_API_URL = "http://localhost:3004";
          break;
        case "SECURITY_DB_PATH":
          out.SECURITY_DB_PATH = `${HOME}/anchor-security/security.db`;
          break;
        case "DETECT_TICK_MS":
          out.DETECT_TICK_MS = "30000";
          break;
        case "PENTEST_TARGET":
          out.PENTEST_TARGET = "http://localhost:3001";
          break;
        case "PENTEST_ADMIN_TARGET":
          out.PENTEST_ADMIN_TARGET = "http://localhost:3002";
          break;
        case "SECURITY_API_TOKEN":
        case "ADMIN_API_TOKEN":
          out[k] = process.env.SECURITY_API_TOKEN ?? "dev-security-token-change-me";
          break;
        case "PUSH_TOKEN":
          out.PUSH_TOKEN = process.env.PUSH_TOKEN ?? "dev-push-token-change-me";
          break;
        default:
          if (process.env[k]) out[k] = process.env[k]!;
      }
    }
    return out;
  };

  return stack.services.map(svc => {
    const launcher = opts.useLocalDev ? svc.dev : svc.prod;
    return {
      name: svc.name,
      command: launcher.command,
      args: launcher.args.map(expandHome),
      env: buildEnv(svc),
      port: svc.port,
      startupGraceMs: svc.startup_grace_ms,
    };
  });
}
