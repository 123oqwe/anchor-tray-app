// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Anchor tray-app — system-tray supervisor for the 3 anchor backends.
//!
//! Supervised services:
//!   - anchor-backend       :3001 (user surface)
//!   - anchor-admin-backend :3002 (operator API)
//!   - anchor-security      :3004 (out-of-band detector + pentest)
//!
//! NOT supervised here: the 7 anchor-*-mcp servers. Those are stdio JSON-RPC
//! peers and must be spawned by anchor-backend's own MCP host with stdin
//! piped — supervising them with stdin closed makes them EOF and exit
//! immediately. Mirror of the TS supervisor's design (src/supervisor.ts).
//!
//! Lifecycle:
//!   - Each child runs on its own thread that wait()s for exit, applies
//!     exponential backoff (1s → 2s → 4s → 8s → max 60s), and respawns.
//!   - 5 crashes within 5 min marks a service DEGRADED (no more auto-restart;
//!     manual restart required from tray menu).
//!   - On app quit: sends SIGTERM to every child, waits briefly, exits.
//!
//! Logs: per-service file at ~/.anchor/logs/<name>.log (rotated at 10MB).

use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_opener::OpenerExt;

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const FAILURE_WINDOW_SECS: u64 = 5 * 60;
const MAX_FAILURES: usize = 5;
const STARTUP_GRACE_MS: u64 = 3000;

#[derive(Clone, Debug)]
struct ServiceDef {
    name: String,
    cmd: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    /// Optional HTTP port (informational; not used by supervisor itself).
    port: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ServiceState {
    Stopped,
    Starting,
    Running,
    Crashed,
    Degraded,
}

struct ServiceRuntime {
    def: ServiceDef,
    child: Option<Child>,
    state: ServiceState,
    restarts: u32,
    failures: Vec<Instant>,
    backoff_ms: u64,
    /// True when shutdown was requested — supervisor thread should NOT respawn.
    shutdown_flag: bool,
}

struct Supervisor {
    services: Mutex<HashMap<String, ServiceRuntime>>,
    log_dir: PathBuf,
}

impl Supervisor {
    fn new(defs: Vec<ServiceDef>, log_dir: PathBuf) -> Self {
        let mut map = HashMap::new();
        for def in defs {
            map.insert(
                def.name.clone(),
                ServiceRuntime {
                    def,
                    child: None,
                    state: ServiceState::Stopped,
                    restarts: 0,
                    failures: Vec::new(),
                    backoff_ms: 1000,
                    shutdown_flag: false,
                },
            );
        }
        std::fs::create_dir_all(&log_dir).ok();
        Self {
            services: Mutex::new(map),
            log_dir,
        }
    }

    fn log_path(&self, name: &str) -> PathBuf {
        self.log_dir.join(format!("{}.log", name))
    }

    fn rotate_if_needed(&self, name: &str) {
        let p = self.log_path(name);
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.len() > MAX_LOG_BYTES {
                let _ = std::fs::rename(&p, p.with_extension("log.1"));
            }
        }
    }

    /// Spawn one service, return the new Child (caller stores it).
    fn spawn_one(&self, def: &ServiceDef) -> Option<Child> {
        self.rotate_if_needed(&def.name);
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path(&def.name))
            .ok()?;
        let stderr_file = log_file.try_clone().ok()?;

        let mut cmd = Command::new(&def.cmd);
        cmd.args(&def.args)
            .envs(env::vars())
            .envs(def.env.iter())
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr_file));

        match cmd.spawn() {
            Ok(child) => {
                println!("[supervisor] started {} (pid {})", def.name, child.id());
                Some(child)
            }
            Err(e) => {
                eprintln!("[supervisor] failed to spawn {}: {}", def.name, e);
                None
            }
        }
    }
}

/// Spawn one service + spawn its watcher thread. The watcher waits for the
/// child to exit, then either respawns (with backoff) or marks DEGRADED.
fn launch_with_watcher(sup: Arc<Supervisor>, name: String) {
    let def = {
        let mut map = sup.services.lock().unwrap();
        let rt = match map.get_mut(&name) {
            Some(r) => r,
            None => return,
        };
        if rt.child.is_some() {
            return; // already running
        }
        rt.shutdown_flag = false;
        rt.state = ServiceState::Starting;
        rt.def.clone()
    };

    let child = sup.spawn_one(&def);
    {
        let mut map = sup.services.lock().unwrap();
        if let Some(rt) = map.get_mut(&name) {
            rt.child = child;
            if rt.child.is_none() {
                rt.state = ServiceState::Crashed;
                return;
            }
        }
    }

    // After grace period, flip Starting → Running unless something already
    // happened (crashed / shutdown).
    let sup_grace = sup.clone();
    let name_grace = name.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(STARTUP_GRACE_MS));
        let mut map = sup_grace.services.lock().unwrap();
        if let Some(rt) = map.get_mut(&name_grace) {
            if rt.state == ServiceState::Starting {
                rt.state = ServiceState::Running;
            }
        }
    });

    // Watcher thread: take ownership of the child, wait for exit, decide.
    let sup_w = sup.clone();
    let name_w = name.clone();
    thread::spawn(move || {
        // Take the child out of the runtime so we can wait() without holding the lock.
        let mut child_opt = {
            let mut map = sup_w.services.lock().unwrap();
            map.get_mut(&name_w).and_then(|rt| rt.child.take())
        };
        let exit_status = child_opt.as_mut().and_then(|c| c.wait().ok());

        let respawn_after = {
            let mut map = sup_w.services.lock().unwrap();
            let rt = match map.get_mut(&name_w) {
                Some(r) => r,
                None => return,
            };
            rt.child = None;
            if rt.shutdown_flag {
                rt.state = ServiceState::Stopped;
                return;
            }
            rt.state = ServiceState::Crashed;
            rt.failures.push(Instant::now());
            let cutoff = Instant::now() - Duration::from_secs(FAILURE_WINDOW_SECS);
            rt.failures.retain(|t| *t > cutoff);

            if rt.failures.len() >= MAX_FAILURES {
                rt.state = ServiceState::Degraded;
                eprintln!(
                    "[supervisor] {} DEGRADED ({} crashes in {}s) — manual restart needed",
                    name_w,
                    rt.failures.len(),
                    FAILURE_WINDOW_SECS
                );
                return;
            }

            let delay = rt.backoff_ms;
            rt.backoff_ms = (rt.backoff_ms * 2).min(60_000);
            rt.restarts += 1;
            eprintln!(
                "[supervisor] {} crashed ({:?}) — restarting in {}ms",
                name_w, exit_status, delay
            );
            delay
        };

        thread::sleep(Duration::from_millis(respawn_after));
        launch_with_watcher(sup_w, name_w);
    });
}

fn start_all(sup: Arc<Supervisor>) {
    let names: Vec<String> = sup
        .services
        .lock()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    for n in names {
        launch_with_watcher(sup.clone(), n);
    }
}

fn stop_all(sup: &Supervisor) {
    let mut map = sup.services.lock().unwrap();
    for (_, rt) in map.iter_mut() {
        rt.shutdown_flag = true;
        if let Some(mut child) = rt.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            rt.state = ServiceState::Stopped;
        }
    }
}

fn manual_restart(sup: Arc<Supervisor>, name: &str) {
    {
        let mut map = sup.services.lock().unwrap();
        if let Some(rt) = map.get_mut(name) {
            rt.shutdown_flag = true;
            if let Some(mut child) = rt.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            rt.failures.clear();
            rt.backoff_ms = 1000;
            rt.shutdown_flag = false;
        }
    }
    launch_with_watcher(sup, name.to_string());
}

/// Snapshot of service state for the status dialog.
fn status_snapshot(sup: &Supervisor) -> Vec<(String, ServiceState, u32, Option<u16>)> {
    let map = sup.services.lock().unwrap();
    map.values()
        .map(|rt| (rt.def.name.clone(), rt.state, rt.restarts, rt.def.port))
        .collect()
}

// ── Default service stack ───────────────────────────────────────────────────

fn default_stack() -> Vec<ServiceDef> {
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let use_local = env::var("ANCHOR_USE_LOCAL_DEV")
        .map(|v| v == "true")
        .unwrap_or(false);

    let backend_port: u16 = env::var("ANCHOR_BACKEND_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3001);
    let admin_port: u16 = 3002;
    let security_port: u16 = 3004;
    let anchor_db = format!("{}/anchor-backend/server/infra/anchor.db", home);
    let security_token = env::var("SECURITY_API_TOKEN")
        .unwrap_or_else(|_| "dev-security-token-change-me".into());
    let push_token = env::var("PUSH_TOKEN").unwrap_or_else(|_| "dev-push-token-change-me".into());

    let mut backend_env: HashMap<String, String> = HashMap::new();
    backend_env.insert("PORT".into(), backend_port.to_string());
    backend_env.insert("MCP_ENABLED".into(), "true".into());

    let mut admin_env: HashMap<String, String> = HashMap::new();
    admin_env.insert("PORT".into(), admin_port.to_string());
    admin_env.insert("ANCHOR_DB_PATH".into(), anchor_db.clone());
    admin_env.insert("SECURITY_API_URL".into(), format!("http://localhost:{}", security_port));
    admin_env.insert("SECURITY_API_TOKEN".into(), security_token.clone());

    let mut security_env: HashMap<String, String> = HashMap::new();
    security_env.insert("PORT".into(), security_port.to_string());
    security_env.insert("ANCHOR_DB_PATH".into(), anchor_db);
    security_env.insert("SECURITY_DB_PATH".into(), format!("{}/anchor-security/security.db", home));
    security_env.insert("DETECT_TICK_MS".into(), "30000".into());
    security_env.insert("PENTEST_TARGET".into(), format!("http://localhost:{}", backend_port));
    security_env.insert("PENTEST_ADMIN_TARGET".into(), format!("http://localhost:{}", admin_port));
    security_env.insert("ADMIN_API_TOKEN".into(), security_token);
    security_env.insert("PUSH_TOKEN".into(), push_token);

    if use_local {
        vec![
            ServiceDef {
                name: "anchor-backend".into(),
                cmd: "pnpm".into(),
                args: vec!["tsx".into(), format!("{}/anchor-backend/server/index.ts", home)],
                env: backend_env,
                port: Some(backend_port),
            },
            ServiceDef {
                name: "anchor-admin-backend".into(),
                cmd: "pnpm".into(),
                args: vec!["tsx".into(), format!("{}/anchor-admin-backend/server/index.ts", home)],
                env: admin_env,
                port: Some(admin_port),
            },
            ServiceDef {
                name: "anchor-security".into(),
                cmd: "pnpm".into(),
                args: vec!["tsx".into(), format!("{}/anchor-security/server/index.ts", home)],
                env: security_env,
                port: Some(security_port),
            },
        ]
    } else {
        vec![
            ServiceDef {
                name: "anchor-backend".into(),
                cmd: "npx".into(),
                args: vec!["-y".into(), "@anchor/backend".into()],
                env: backend_env,
                port: Some(backend_port),
            },
            ServiceDef {
                name: "anchor-admin-backend".into(),
                cmd: "npx".into(),
                args: vec!["-y".into(), "@anchor/admin-backend".into()],
                env: admin_env,
                port: Some(admin_port),
            },
            ServiceDef {
                name: "anchor-security".into(),
                cmd: "npx".into(),
                args: vec!["-y".into(), "@anchor/security".into()],
                env: security_env,
                port: Some(security_port),
            },
        ]
    }
}

// ── Tauri app entry ────────────────────────────────────────────────────────

fn main() {
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let log_dir: PathBuf = env::var("ANCHOR_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(format!("{}/.anchor/logs", home)));

    let supervisor = Arc::new(Supervisor::new(default_stack(), log_dir.clone()));
    let backend_port: u16 = env::var("ANCHOR_BACKEND_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3001);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(supervisor.clone())
        .setup({
            let supervisor = supervisor.clone();
            move |app| {
                start_all(supervisor.clone());

                let open_ui = MenuItem::with_id(app, "open_ui", "Open Anchor", true, None::<&str>)?;
                let status = MenuItem::with_id(app, "status", "Status", true, None::<&str>)?;
                let restart_all =
                    MenuItem::with_id(app, "restart_all", "Restart all", true, None::<&str>)?;
                let logs = MenuItem::with_id(app, "logs", "Open log folder", true, None::<&str>)?;
                let sep = PredefinedMenuItem::separator(app)?;
                let quit = MenuItem::with_id(app, "quit", "Quit Anchor", true, None::<&str>)?;

                let menu = Menu::with_items(
                    app,
                    &[&open_ui, &status, &restart_all, &logs, &sep, &quit],
                )?;

                let log_dir_menu = log_dir.clone();
                let _tray = TrayIconBuilder::with_id("anchor-tray")
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .show_menu_on_left_click(true)
                    .on_menu_event(move |app_handle, event| match event.id.as_ref() {
                        "open_ui" => {
                            let url = format!("http://localhost:{}", backend_port);
                            let _ = app_handle.opener().open_url(&url, None::<String>);
                        }
                        "status" => {
                            let sup: tauri::State<Arc<Supervisor>> = app_handle.state();
                            let snap = status_snapshot(&sup);
                            let lines: Vec<String> = snap
                                .into_iter()
                                .map(|(name, state, restarts, port)| {
                                    format!(
                                        "{:<22} {:?} restarts={} port={:?}",
                                        name, state, restarts, port
                                    )
                                })
                                .collect();
                            println!("[Anchor Status]\n{}", lines.join("\n"));
                        }
                        "restart_all" => {
                            let sup: tauri::State<Arc<Supervisor>> = app_handle.state();
                            let names: Vec<String> = sup
                                .services
                                .lock()
                                .unwrap()
                                .keys()
                                .cloned()
                                .collect();
                            for n in names {
                                manual_restart(sup.inner().clone(), &n);
                            }
                        }
                        "logs" => {
                            let path = log_dir_menu.to_string_lossy().to_string();
                            let _ = app_handle.opener().open_path(path, None::<String>);
                        }
                        "quit" => {
                            let sup: tauri::State<Arc<Supervisor>> = app_handle.state();
                            stop_all(&sup);
                            app_handle.exit(0);
                        }
                        _ => {}
                    })
                    .build(app)?;

                Ok(())
            }
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running anchor-tray-app");
}
