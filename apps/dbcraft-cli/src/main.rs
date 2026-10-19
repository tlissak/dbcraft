//! dbcraft-cli: the engine without a window.
//!
//!   dbcraft-cli commands                         list every command with its params
//!   dbcraft-cli cmd <id> [json-params]           run one command, print the JSON result
//!   dbcraft-cli sql <connection-id> <sql> [db]   open a saved connection and run SQL
//!   dbcraft-cli mcp                              MCP server on stdio
//!   dbcraft-cli batchjob <id|name>               run an Automation batch job (what scheduled tasks call)
//!   dbcraft-cli control <method> [json-fields]   drive a running `dbcraft --control` (ping, state,
//!                                                dispatch, action, ui, input, wait_idle, snapshot)
//!
//! Uses the same workspace as the desktop app (override with `--workspace <file>` or
//! DBCRAFT_WORKSPACE).

use std::path::PathBuf;
use std::process::ExitCode;

use dbcraft_engine::{Session, commands};
use serde_json::{Value as Json, json};

fn workspace_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DBCRAFT_WORKSPACE") {
        return PathBuf::from(p);
    }
    directories::ProjectDirs::from("ai", "dbcraft", "DbCraft").map(|d| d.config_dir().join("workspace.json")).unwrap_or_else(|| PathBuf::from("workspace.json"))
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: dbcraft-cli [--workspace <file>] commands | cmd <id> [json] | sql <connection-id> <sql> [database] | mcp | batchjob <id|name> | control <method> [json]"
    );
    ExitCode::from(2)
}

fn print(r: dbcraft_engine::CommandResult) -> ExitCode {
    match r {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut ws = workspace_path();
    let mut overridden = std::env::var_os("DBCRAFT_WORKSPACE").is_some();
    if let Some(i) = args.iter().position(|a| a == "--workspace") {
        let Some(path) = args.get(i + 1).cloned() else { return usage() };
        ws = PathBuf::from(path);
        overridden = true;
        args.drain(i..=i + 1);
    }
    if args.first().map(String::as_str) == Some("control") {
        return control(&ws, args.get(1).map(String::as_str), args.get(2).map(String::as_str));
    }
    let (mut session, err) = Session::with_workspace_file(ws);
    if let Some(e) = err {
        eprintln!("warning: {e}");
    }
    if let Some(w) = session.use_os_keychain(!overridden) {
        eprintln!("warning: {w}");
    }
    match args.first().map(String::as_str) {
        Some("commands") => {
            for c in commands() {
                println!("{:<22} {:<28} {}", c.id, c.label, c.params_doc);
            }
            ExitCode::SUCCESS
        }
        Some("cmd") => {
            let Some(id) = args.get(1) else { return usage() };
            let params = match args.get(2).map(|s| serde_json::from_str::<Json>(s)) {
                None => json!({}),
                Some(Ok(p)) => p,
                Some(Err(e)) => {
                    eprintln!("error: params are not valid JSON: {e}");
                    return ExitCode::from(2);
                }
            };
            print(session.dispatch(id, &params))
        }
        Some("sql") => {
            let (Some(conn), Some(sql)) = (args.get(1), args.get(2)) else { return usage() };
            if let Err(e) = session.dispatch("connection.open", &json!({ "id": conn })) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
            print(session.dispatch("query.run", &json!({ "connection": conn, "sql": sql, "database": args.get(3) })))
        }
        Some("batchjob") => {
            let Some(which) = args.get(1) else { return usage() };
            let Some(job) = session.workspace.automations.iter().find(|j| &j.id == which || j.name.eq_ignore_ascii_case(which)).cloned() else {
                eprintln!("error: no batch job `{which}`");
                return ExitCode::FAILURE;
            };
            // Scheduled runs have no console: also append progress to <data dir>/logs/<job>.log.
            let log_path = session.data_dir.as_ref().map(|d| d.join("logs").join(format!("{}.log", job.id)));
            if let Some(dir) = log_path.as_ref().and_then(|p| p.parent()) {
                let _ = std::fs::create_dir_all(dir);
            }
            let log = log_path.clone();
            session.progress = Some(std::sync::Arc::new(move |line: &str| {
                println!("{line}");
                if let Some(p) = &log
                    && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
                {
                    use std::io::Write;
                    let _ = writeln!(f, "{line}");
                }
            }));
            match session.dispatch("automation.run", &json!({ "id": job.id })) {
                Ok(r) if r["ok"] == true => ExitCode::SUCCESS,
                Ok(_) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("mcp") => match dbcraft_automation::serve_stdio(&mut session) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}

/// Send one request to a running `dbcraft --control` (address and token from `control.json` next
/// to the workspace) and print the reply.
fn control(ws: &std::path::Path, method: Option<&str>, fields: Option<&str>) -> ExitCode {
    use std::io::{BufRead, BufReader, Write};
    let Some(method) = method else { return usage() };
    let info_path = ws.parent().map(|d| d.join("control.json")).unwrap_or_else(|| PathBuf::from("control.json"));
    let info: Json = match std::fs::read_to_string(&info_path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {}: {e} (is `dbcraft --control` running with this workspace?)", info_path.display());
            return ExitCode::FAILURE;
        }
    };
    let mut req = match fields.map(serde_json::from_str::<Json>).transpose() {
        Ok(Some(Json::Object(o))) => Json::Object(o),
        Ok(None) => json!({}),
        Ok(Some(_)) | Err(_) => {
            eprintln!("error: the request fields must be a JSON object");
            return ExitCode::from(2);
        }
    };
    req["method"] = json!(method);
    req["token"] = info["token"].clone();
    let addr = info["addr"].as_str().unwrap_or_default();
    let result = std::net::TcpStream::connect(addr).and_then(|mut s| {
        writeln!(s, "{req}")?;
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line)?;
        Ok(line)
    });
    match result.map(|l| serde_json::from_str::<Json>(&l)) {
        Ok(Ok(reply)) => {
            println!("{}", serde_json::to_string_pretty(&reply).unwrap_or_default());
            if reply["ok"] == json!(true) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Ok(Err(e)) => {
            eprintln!("error: bad reply: {e}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: {addr}: {e}");
            ExitCode::FAILURE
        }
    }
}
