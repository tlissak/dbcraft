//! Background execution (ROADMAP 1.1), driven headlessly without waiting for tasks: the window
//! keeps drawing while a query runs, Stop (button or Ctrl+T) ends it, and connecting doesn't block.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use dbcraft_ui_egui::state::Tab;
use dbcraft_ui_egui::{DbCraftApp, StartupAction};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const ENDLESS: &str = "with recursive n(x) as (select 1 union all select x + 1 from n) select count(*) from n";

fn session(name: &str) -> Session {
    let d = std::env::temp_dir().join(format!("dbcraft-uiasync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.save", &json!({"id": "lite", "kind": "sqlite", "name": "lite", "database": d.join("lite.db").to_string_lossy()})).unwrap();
    s.dispatch("connection.open", &json!({"id": "lite"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "lite", "sql": "create table t (x int); insert into t values (1), (2), (3)"})).unwrap();
    s
}

fn harness(app: DbCraftApp) -> Harness<'static, DbCraftApp> {
    Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app)
}

fn query_tab<'a>(h: &'a Harness<'_, DbCraftApp>) -> &'a dbcraft_ui_egui::state::QueryTab {
    match h.state().state.tabs.last() {
        Some(Tab::Query(q)) => q,
        _ => panic!("no query tab"),
    }
}

/// Step frames (with real time passing) until `done`, or fail after ~10 s.
fn until(h: &mut Harness<DbCraftApp>, what: &str, done: impl Fn(&DbCraftApp) -> bool) {
    for _ in 0..400 {
        h.step();
        if done(h.state()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn endless_query_runs_in_the_background_and_ctrl_t_stops_it() {
    let mut app = DbCraftApp::new(session("stop"), None);
    app.startup(StartupAction::NewQuery { connection: "lite".into(), database: Some("main".into()), sql: ENDLESS.into() });
    let mut h = harness(app);
    h.step();
    h.step();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.step();
    h.step();
    assert!(query_tab(&h).task.is_some(), "query runs in the background");
    // The window keeps drawing: frames go by and the running bar shows.
    for _ in 0..5 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(h.query_by_label_contains("Running…").is_some());
    assert!(h.query_by_label_contains("Stop").is_some());

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::T);
    until(&mut h, "the query to stop", |a| matches!(a.state.tabs.last(), Some(Tab::Query(q)) if q.task.is_none()));
    let q = query_tab(&h);
    assert_eq!(q.error.as_deref(), Some("Stopped"));

    // The connection is free again: a normal query runs to completion.
    if let Some(Tab::Query(q)) = h.state_mut().state.tabs.last_mut() {
        q.sql = "select count(*) as n from t".into();
    }
    h.step();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    until(&mut h, "the second query", |a| matches!(a.state.tabs.last(), Some(Tab::Query(q)) if q.task.is_none() && !q.results.is_empty()));
    assert_eq!(query_tab(&h).results[0].rows[0][0], dbcraft_core::Value::Int(3));
}

#[test]
fn stop_button_and_closing_the_tab_cancel() {
    let mut app = DbCraftApp::new(session("button"), None);
    app.startup(StartupAction::NewQuery { connection: "lite".into(), database: Some("main".into()), sql: ENDLESS.into() });
    let mut h = harness(app);
    h.step();
    h.step();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.step();
    h.step();
    h.get_by_label_contains("Stop").click();
    until(&mut h, "Stop button", |a| matches!(a.state.tabs.last(), Some(Tab::Query(q)) if q.task.is_none()));

    // Run again, then close the tab: the worker is cancelled, not left running.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.step();
    h.step();
    let task = query_tab(&h).task.clone().expect("running");
    let i = h.state().state.tabs.len() - 1;
    h.state_mut().close_tab(i, true);
    assert!(dbcraft_ui_egui::tasks::wait(&task, std::time::Duration::from_secs(5)), "worker ended after the tab closed");
}

#[test]
fn opening_a_table_from_the_command_line_connects_in_the_background() {
    let s = session("open");
    let path = s.workspace.connection("lite").unwrap().database.clone();
    // A fresh app: the connection is saved but not open.
    let d = std::path::PathBuf::from(&path);
    let (mut fresh, _) = Session::with_workspace_file(d.with_file_name("workspace.json"));
    assert!(!fresh.is_open("lite"));
    fresh.messages.clear();
    let mut app = DbCraftApp::new(fresh, None);
    app.startup(StartupAction::OpenTable { connection: "lite".into(), database: "main".into(), schema: None, table: "t".into() });
    let mut h = harness(app);
    until(&mut h, "the table page", |a| matches!(a.state.tabs.last(), Some(Tab::Table(t)) if t.page.is_some()));
    assert!(h.state().session.is_open("lite"));
    match h.state().state.tabs.last() {
        Some(Tab::Table(t)) => assert_eq!(t.page.as_ref().unwrap().total, 3),
        _ => panic!("no table tab"),
    }
}

/// Expanding a connection whose server never answers shows "Connecting…" without freezing, and
/// its Stop button gives up at once (no error in the status bar: the user asked for it).
#[test]
fn connecting_shows_progress_and_can_be_stopped() {
    // Accepts TCP connections but never speaks the protocol: the login would hang until timeout.
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let mut s = session("connecting");
    s.dispatch("connection.save", &json!({"id": "slow", "kind": "postgres", "name": "slow", "host": "127.0.0.1", "port": port, "user": "x"})).unwrap();
    let mut h = harness(DbCraftApp::new(s, None));
    h.step();
    h.get_by_label_contains("slow").click_secondary();
    h.step();
    h.get_by_label("Open Connection").click();
    h.step();
    h.step();
    let id = egui::Id::new(("conn", "slow"));
    let mut st = egui::collapsing_header::CollapsingState::load_with_default_open(&h.ctx, id, false);
    st.set_open(true);
    st.store(&h.ctx);
    h.step();
    h.step();
    assert!(h.query_by_label_contains("Connecting").is_some(), "progress in the tree");
    let start = std::time::Instant::now();
    h.get_by_label("Stop").click();
    until(&mut h, "the attempt to stop", |a| !a.is_busy());
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "stopped in {:?}", start.elapsed());
    assert!(!h.state().session.is_open("slow"));
    assert!(h.state().state.status.is_none(), "a Stop is not an error: {:?}", h.state().state.status);
    drop(silent);
}
