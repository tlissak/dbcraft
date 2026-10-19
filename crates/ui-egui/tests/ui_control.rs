//! Control channel (ROADMAP 1.2): a client drives the app over 127.0.0.1 with the token while the
//! UI runs frames, and never sees a password.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use dbcraft_engine::Session;
use dbcraft_ui_egui::DbCraftApp;
use dbcraft_ui_egui::state::Tab;
use egui_kittest::Harness;
use serde_json::{Value as Json, json};

struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    token: String,
}

impl Client {
    fn connect(addr: std::net::SocketAddr, token: &str) -> Client {
        let s = TcpStream::connect(addr).unwrap();
        Client { reader: BufReader::new(s.try_clone().unwrap()), writer: s, token: token.into() }
    }

    fn raw(&mut self, line: &str) -> Json {
        writeln!(self.writer, "{line}").unwrap();
        let mut reply = String::new();
        self.reader.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    }

    fn call(&mut self, method: &str, mut fields: Json) -> Json {
        fields["method"] = json!(method);
        fields["token"] = json!(self.token);
        self.raw(&fields.to_string())
    }
}

#[test]
fn an_agent_drives_the_running_app() {
    let d = std::env::temp_dir().join(format!("dbcraft-uicontrol-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.save", &json!({"id": "lite", "kind": "sqlite", "name": "lite", "database": d.join("lite.db").to_string_lossy()})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| DbCraftApp::new(s, None).waiting_for_tasks());
    h.run();
    let ctx = h.ctx.clone();
    let (addr, token) = h.state_mut().start_control(&ctx, 0).unwrap();
    assert!(addr.ip().is_loopback());
    let info: Json = serde_json::from_str(&std::fs::read_to_string(d.join("control.json")).unwrap()).unwrap();
    assert_eq!(info["token"], json!(token));

    // The agent's script runs on its own thread; the UI keeps drawing frames meanwhile.
    let agent = std::thread::spawn(move || {
        let mut c = Client::connect(addr, &token);
        let mut log = Vec::new();
        log.push(("ping", c.call("ping", json!({}))));
        log.push(("bad token", c.raw(&json!({"method": "ping", "token": "nope"}).to_string())));
        log.push(("no token", c.raw(r#"{"method":"ping"}"#)));
        log.push(("junk", c.raw("not json")));
        log.push(("open", c.call("dispatch", json!({"id": "connection.open", "params": {"id": "lite"}}))));
        log.push((
            "create",
            c.call("dispatch", json!({"id": "query.run", "params": {"connection": "lite", "sql": "create table t (x int); insert into t values (1), (2)"}})),
        ));
        log.push(("open table", c.call("action", json!({"action": {"type": "open_table", "connection": "lite", "database": "main", "table": "t"}}))));
        log.push(("idle", c.call("wait_idle", json!({"timeout_ms": 10000}))));
        log.push((
            "new query",
            c.call("action", json!({"action": {"type": "new_query", "connection": "lite", "database": "main", "sql": "select sum(x) as s from t"}})),
        ));
        log.push(("idle2", c.call("wait_idle", json!({}))));
        log.push(("run", c.call("input", json!({"events": [{"key": "R", "ctrl": true}]}))));
        log.push(("idle3", c.call("wait_idle", json!({}))));
        log.push(("dialog", c.call("action", json!({"action": {"type": "new_connection"}}))));
        log.push(("idle4", c.call("wait_idle", json!({}))));
        log.push(("state", c.call("state", json!({}))));
        log.push(("unknown", c.call("fly", json!({}))));
        log.push(("not ui", c.call("ui", json!({"id": "connection.delete"}))));
        log
    });
    let mut log = None;
    for _ in 0..2000 {
        // What eframe's raw_input_hook does in the real app.
        for e in h.state_mut().take_injected() {
            h.event(e);
        }
        h.step();
        if agent.is_finished() {
            log = Some(agent.join().unwrap());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let log: std::collections::HashMap<&str, Json> = log.expect("agent finished").into_iter().collect();
    assert_eq!(log["ping"]["app"], "DbCraft");
    for k in ["bad token", "no token", "junk", "unknown", "not ui"] {
        assert_eq!(log[k]["ok"], false, "{k}: {}", log[k]);
    }
    for k in ["open", "create", "open table", "idle", "new query", "idle2", "run", "idle3", "dialog", "idle4", "state"] {
        assert_eq!(log[k]["ok"], true, "{k}: {}", log[k]);
    }
    // The UI shows what the agent did.
    let app = h.state();
    assert!(app.state.tabs.iter().any(|t| matches!(t, Tab::Table(tt) if tt.table == "t" && tt.page.as_ref().is_some_and(|p| p.total == 2))));
    let q = app.state.tabs.iter().find_map(|t| if let Tab::Query(q) = t { Some(q) } else { None }).expect("query tab");
    assert_eq!(q.results.first().map(|r| r.rows[0][0].clone()), Some(dbcraft_core::Value::Int(3)), "Ctrl+R sent as input ran the query");
    // State is the real UI state, minus secrets.
    let state = &log["state"];
    assert_eq!(state["open_connections"], json!(["lite"]));
    assert!(state["ui"]["connection_form"].is_object(), "the dialog the agent opened");
    assert!(!state.to_string().contains(&info["token"].as_str().unwrap().to_string()));
}

#[test]
fn state_never_contains_a_password() {
    let d = std::env::temp_dir().join(format!("dbcraft-uicontrol-pw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (s, _) = Session::with_workspace_file(d.join("workspace.json"));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| DbCraftApp::new(s, None).waiting_for_tasks());
    h.run();
    let ctx = h.ctx.clone();
    let (addr, token) = h.state_mut().start_control(&ctx, 0).unwrap();
    // The user is typing a password in the connection dialog.
    h.state_mut().state.connection_form = Some(dbcraft_ui_egui::state::ConnectionForm::new(dbcraft_core::DbKind::Postgres));
    if let Some(f) = h.state_mut().state.connection_form.as_mut() {
        f.password = "hunter2-db".into();
        f.ssh_password = "hunter2-ssh".into();
    }
    h.run();
    let agent = std::thread::spawn(move || Client::connect(addr, &token).call("state", json!({})));
    while !agent.is_finished() {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let state = agent.join().unwrap().to_string();
    assert!(state.contains("\"ok\":true"));
    assert!(!state.contains("hunter2"), "{state}");
}
