// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::env;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

/// One supervised child process (anchor-backend or one of the MCP servers).
struct Service {
    name: String,
    cmd: String,
    args: Vec<String>,
    child: Option<Child>,
}

/// Mutable shared state held in Tauri's State.
struct Supervisor {
    services: Mutex<Vec<Service>>,
    log_dir: PathBuf,
    backend_port: u16,
}

impl Supervisor {
    fn new() -> Self {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let log_dir: PathBuf = env::var("ANCHOR_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(format!("{}/.anchor/logs", home)));
        std::fs::create_dir_all(&log_dir).ok();

        let backend_port: u16 = env::var("ANCHOR_BACKEND_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3001);

        // Default service definitions. Prod uses npm packages via npx; users
        // can override via env ANCHOR_USE_LOCAL_DEV=true to run from
        // ~/anchor-* checkouts (handy during dev).
        let use_local = env::var("ANCHOR_USE_LOCAL_DEV").map(|v| v == "true").unwrap_or(false);
        let services: Vec<Service> = if use_local {
            vec![
                ("anchor-backend",       "pnpm", vec!["tsx", &format!("{}/anchor-backend/server/index.ts", home)]),
                ("anchor-activity-mcp",  "npx",  vec!["tsx", &format!("{}/anchor-activity-mcp/src/index.ts", home)]),
                ("anchor-browser-mcp",   "npx",  vec!["tsx", &format!("{}/anchor-browser-mcp/src/index.ts", home)]),
                ("anchor-input-mcp",     "npx",  vec!["tsx", &format!("{}/anchor-input-mcp/src/index.ts", home)]),
                ("anchor-system-mcp",    "npx",  vec!["tsx", &format!("{}/anchor-system-mcp/src/index.ts", home)]),
                ("anchor-screen-mcp",    "npx",  vec!["tsx", &format!("{}/anchor-screen-mcp/src/index.ts", home)]),
                ("anchor-code-mcp",      "npx",  vec!["tsx", &format!("{}/anchor-code-mcp/src/index.ts", home)]),
                ("anchor-shell-mcp",     "npx",  vec!["tsx", &format!("{}/anchor-shell-mcp/src/index.ts", home)]),
            ]
        } else {
            vec![
                ("anchor-backend",       "npx", vec!["-y", "@anchor/backend"]),
                ("anchor-activity-mcp",  "npx", vec!["-y", "@anchor/activity-mcp"]),
                ("anchor-browser-mcp",   "npx", vec!["-y", "@anchor/browser-mcp"]),
                ("anchor-input-mcp",     "npx", vec!["-y", "@anchor/input-mcp"]),
                ("anchor-system-mcp",    "npx", vec!["-y", "@anchor/system-mcp"]),
                ("anchor-screen-mcp",    "npx", vec!["-y", "@anchor/screen-mcp"]),
                ("anchor-code-mcp",      "npx", vec!["-y", "@anchor/code-mcp"]),
                ("anchor-shell-mcp",     "npx", vec!["-y", "@anchor/shell-mcp"]),
            ]
        }
        .into_iter()
        .map(|(name, cmd, args)| Service {
            name: name.to_string(),
            cmd: cmd.to_string(),
            args: args.into_iter().map(|s| s.to_string()).collect(),
            child: None,
        })
        .collect();

        Self {
            services: Mutex::new(services),
            log_dir,
            backend_port,
        }
    }

    fn start_all(&self) {
        let mut svcs = self.services.lock().unwrap();
        for svc in svcs.iter_mut() {
            if svc.child.is_some() {
                continue;
            }
            let log_path = self.log_dir.join(format!("{}.log", svc.name));
            let log_file = OpenOptions::new()
                .append(true)
                .create(true)
                .open(&log_path)
                .ok();

            let mut env_vars: HashMap<String, String> = std::env::vars().collect();
            env_vars.insert("PORT".into(), self.backend_port.to_string());
            env_vars.insert("MCP_ENABLED".into(), "true".into());

            let mut cmd = Command::new(&svc.cmd);
            cmd.args(&svc.args).envs(env_vars);
            if let Some(file) = log_file {
                let stderr = file.try_clone().ok();
                cmd.stdout(Stdio::from(file));
                if let Some(s) = stderr {
                    cmd.stderr(Stdio::from(s));
                }
            }
            cmd.stdin(Stdio::null());

            match cmd.spawn() {
                Ok(child) => {
                    println!("[supervisor] started {} (pid {})", svc.name, child.id());
                    svc.child = Some(child);
                }
                Err(err) => {
                    eprintln!("[supervisor] failed to start {}: {}", svc.name, err);
                }
            }
        }
    }

    fn stop_all(&self) {
        let mut svcs = self.services.lock().unwrap();
        for svc in svcs.iter_mut() {
            if let Some(mut child) = svc.child.take() {
                let _ = child.kill();
                let _ = child.wait();
                println!("[supervisor] stopped {}", svc.name);
            }
        }
    }

    fn status(&self) -> Vec<(String, Option<u32>)> {
        let svcs = self.services.lock().unwrap();
        svcs.iter()
            .map(|s| (s.name.clone(), s.child.as_ref().map(|c| c.id())))
            .collect()
    }
}

fn main() {
    let supervisor = Supervisor::new();
    let backend_port = supervisor.backend_port;

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(supervisor)
        .setup(move |app| {
            // Start all services on launch
            let sup: tauri::State<Supervisor> = app.state();
            sup.start_all();

            // Build tray menu
            let open_ui  = MenuItem::with_id(app, "open_ui",  "Open Anchor",        true, None::<&str>)?;
            let status   = MenuItem::with_id(app, "status",   "Status",             true, None::<&str>)?;
            let restart  = MenuItem::with_id(app, "restart",  "Restart all",        true, None::<&str>)?;
            let logs     = MenuItem::with_id(app, "logs",     "Open log folder",    true, None::<&str>)?;
            let sep      = PredefinedMenuItem::separator(app)?;
            let quit     = MenuItem::with_id(app, "quit",     "Quit Anchor",        true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&open_ui, &status, &restart, &logs, &sep, &quit])?;

            let _tray = TrayIconBuilder::with_id("anchor-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .menu_on_left_click(true)
                .on_menu_event(move |app_handle, event| {
                    let id = event.id.as_ref();
                    match id {
                        "open_ui" => {
                            let url = format!("http://localhost:{}", backend_port);
                            let _ = app_handle.opener().open_url(&url, None::<String>);
                        }
                        "status" => {
                            let sup: tauri::State<Supervisor> = app_handle.state();
                            let s = sup.status();
                            let lines: Vec<String> = s.into_iter()
                                .map(|(name, pid)| format!("{}: {}", name, pid.map(|p| format!("pid {}", p)).unwrap_or_else(|| "STOPPED".into())))
                                .collect();
                            // Open a window or print to console — for v0.1 just log
                            println!("[Anchor Status]\n{}", lines.join("\n"));
                        }
                        "restart" => {
                            let sup: tauri::State<Supervisor> = app_handle.state();
                            sup.stop_all();
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            sup.start_all();
                        }
                        "logs" => {
                            let sup: tauri::State<Supervisor> = app_handle.state();
                            let path = sup.log_dir.to_string_lossy().to_string();
                            let _ = app_handle.opener().open_path(path, None::<String>);
                        }
                        "quit" => {
                            let sup: tauri::State<Supervisor> = app_handle.state();
                            sup.stop_all();
                            app_handle.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { .. } = event {
                        // Optional: clicking the icon (not menu item) could show window
                        let _ = tray;
                    }
                })
                .build(app)?;

            Ok(())
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

// Note: ensure clean shutdown of children when app exits — Tauri's exit
// goes through .exit() above which calls stop_all() via the menu handler.
// For unexpected termination (kill -9), child processes will be inherited
// by init/launchd; supervisor pattern recommends adding a post-exit ctrlC
// handler in a follow-up version.
