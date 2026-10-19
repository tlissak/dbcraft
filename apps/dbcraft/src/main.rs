#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! dbcraft [--select <conn>[/<db>]] [--open-table <conn>/<db>/[<schema>.]<table>] [--new-connection]
//!         [--console <conn>[/<db>]] [--tool transfer|automation] [--mode queries|backups|users]
//!         [--control [port]]   control channel on 127.0.0.1 for agents (see `dbcraft-cli control`)

use std::path::PathBuf;

use dbcraft_engine::Session;
use dbcraft_ui_egui::{DbCraftApp, StartupAction};

/// `workspace.json` in the OS config dir, or `DBCRAFT_WORKSPACE` if set (tests, agents, portable).
fn workspace_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DBCRAFT_WORKSPACE") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("ai", "dbcraft", "DbCraft").map(|d| d.config_dir().join("workspace.json")).unwrap_or_else(|| PathBuf::from("workspace.json"))
}

/// Startup actions, and the control port when `--control` is given (0: any free port).
fn parse_args() -> (Vec<StartupAction>, Option<u16>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = Vec::new();
    let mut control = None;
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--control" => {
                let port = it.peek().and_then(|p| p.parse::<u16>().ok());
                if port.is_some() {
                    it.next();
                }
                control = Some(port.unwrap_or(0));
            }
            "--select" => {
                if let Some(v) = it.next() {
                    let (c, d) = v.split_once('/').map_or((v.as_str(), None), |(c, d)| (c, Some(d.to_string())));
                    out.push(StartupAction::Select { connection: c.to_string(), database: d });
                }
            }
            "--open-table" => {
                let parts: Vec<&str> = it.next().map(|v| v.splitn(3, '/').collect()).unwrap_or_default();
                if let [c, d, t] = parts.as_slice() {
                    let (schema, table) = t.split_once('.').map_or((None, *t), |(s, t)| (Some(s.to_string()), t));
                    out.push(StartupAction::OpenTable { connection: c.to_string(), database: d.to_string(), schema, table: table.to_string() });
                }
            }
            "--new-connection" => out.push(StartupAction::NewConnection),
            "--console" => {
                if let Some(v) = it.next() {
                    let (c, d) = v.split_once('/').map_or((v.as_str(), None), |(c, d)| (c, Some(d.to_string())));
                    out.push(StartupAction::Console { connection: c.to_string(), database: d });
                }
            }
            "--tool" => {
                if let Some(v) = it.next() {
                    out.push(StartupAction::Tool(v.clone()));
                }
            }
            "--mode" => {
                if let Some(v) = it.next() {
                    out.push(StartupAction::Mode(v.clone()));
                }
            }
            other => eprintln!("dbcraft: ignoring unknown argument `{other}`"),
        }
    }
    (out, control)
}

fn main() -> eframe::Result {
    env_logger::init();
    let (startup, control) = parse_args();
    let (mut session, err) = Session::with_workspace_file(workspace_path());
    // The user's own workspace keeps passwords in the OS keychain; an overridden one (dev, tests,
    // portable) keeps whatever mode its file says.
    let keychain_warning = session.use_os_keychain(std::env::var_os("DBCRAFT_WORKSPACE").is_none());
    let err = err.map(|e| e.to_string()).or(keychain_warning);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("DbCraft").with_inner_size([1360.0, 840.0]).with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DbCraft",
        options,
        Box::new(move |cc| {
            let mut app = DbCraftApp::new(session, err);
            for a in startup {
                app.startup(a);
            }
            if let Some(port) = control {
                match app.start_control(&cc.egui_ctx, port) {
                    Ok((addr, _)) => log::info!("control channel on {addr} (token in control.json next to the workspace)"),
                    Err(e) => app.state.status = Some(format!("control channel: {e}")),
                }
            }
            Ok(Box::new(app))
        }),
    )
}
