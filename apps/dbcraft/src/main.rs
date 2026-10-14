#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! dbcraft [--select <conn>[/<db>]] [--open-table <conn>/<db>/[<schema>.]<table>] [--new-connection]
//!         [--console <conn>[/<db>]] [--tool transfer|automation] [--mode queries|backups|users]

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

fn parse_args() -> Vec<StartupAction> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
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
    out
}

fn main() -> eframe::Result {
    env_logger::init();
    let startup = parse_args();
    let (session, err) = Session::with_workspace_file(workspace_path());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("DbCraft").with_inner_size([1360.0, 840.0]).with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DbCraft",
        options,
        Box::new(move |_cc| {
            let mut app = DbCraftApp::new(session, err.map(|e| e.to_string()));
            for a in startup {
                app.startup(a);
            }
            Ok(Box::new(app))
        }),
    )
}
