//! Saved queries, result grid selection, Console, Data Transfer, Backups and Automation, driven
//! through the real UI headlessly (egui_kittest), checked against the databases.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use std::path::PathBuf;

use dbcraft_engine::Session;
use dbcraft_ui_egui::state::Tab;
use dbcraft_ui_egui::{DbCraftApp, StartupAction};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

/// Workspace file + two SQLite connections (`src` with data, empty `dst`).
fn setup(name: &str) -> (PathBuf, Session) {
    let d = std::env::temp_dir().join(format!("dbcraft-uitools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    for id in ["src", "dst"] {
        s.dispatch("connection.save", &json!({"id": id, "kind": "sqlite", "name": id, "database": d.join(format!("{id}.db")).to_string_lossy()})).unwrap();
    }
    s.dispatch("connection.open", &json!({"id": "src"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "src", "sql": "create table film (id integer primary key, title text); insert into film (title) values ('Metropolis'), ('Nosferatu'), ('Sunrise')"}))
        .unwrap();
    (d, s)
}

fn harness(app: DbCraftApp) -> Harness<'static, DbCraftApp> {
    Harness::builder().with_size(egui::vec2(1500.0, 950.0)).build_eframe(|_| app)
}

/// Step frames until `done` or ~10 s (background jobs).
fn wait(h: &mut Harness<DbCraftApp>, done: impl Fn(&DbCraftApp) -> bool) {
    for _ in 0..400 {
        h.step();
        if done(h.state()) {
            h.step();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("timed out waiting for a background job");
}

fn check(d: &std::path::Path) -> Session {
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.open", &json!({"id": "src"})).ok();
    s.dispatch("connection.open", &json!({"id": "dst"})).ok();
    s
}

#[test]
fn save_query_and_prompt_on_close() {
    let (d, s) = setup("query");
    let mut app = DbCraftApp::new(s, None).waiting_for_tasks();
    app.startup(StartupAction::NewQuery {
        connection: "src".into(),
        database: Some("main".into()),
        sql: "select title from film order by id; select 1 as one".into(),
    });
    let mut h = harness(app);
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
    h.run();
    assert!(h.query_by_label_contains("Result 1 (3)").is_some(), "one result tab per SELECT");
    assert!(h.query_by_label_contains("Result 2 (1)").is_some());
    assert!(h.query_by_label("Nosferatu").is_some());

    // Select row 2 by its row number and copy it.
    h.get_by_label("Row 2").click();
    h.step();
    h.event(egui::Event::Copy);
    h.step();
    let copied: Vec<String> =
        h.output().platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect();
    assert_eq!(copied, ["Nosferatu"]);

    // Ctrl+S on a new query asks for a name.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run();
    assert!(h.query_by_label("Save Query As").is_some());
    h.event(egui::Event::Text("Film titles".into()));
    h.run();
    h.get_by_label("Save").click();
    h.run();
    let mut c = check(&d);
    let saved = c.dispatch("query.list", &json!({"connection": "src"})).unwrap();
    assert_eq!(saved[0]["name"], "Film titles");
    assert!(h.query_by_label("Film titles").is_some(), "tab renamed");

    // Change it, close the tab: Save Changes? → Don't Save keeps the saved version.
    if let Some(Tab::Query(q)) = h.state_mut().state.tabs.get_mut(1) {
        q.sql.push_str(" -- changed");
    }
    h.run();
    assert!(h.query_by_label("Film titles *").is_some(), "modified marker");
    let close = h.get_all_by_label(dbcraft_ui_egui::icons::X).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap();
    close.click();
    h.run();
    assert!(h.query_by_label("Save Changes").is_some());
    h.get_by_label("Don't Save").click();
    h.run();
    assert_eq!(h.state().state.tabs.len(), 1, "closed");
    let saved = c.dispatch("query.list", &json!({"connection": "src"})).unwrap();
    assert!(!saved[0]["sql"].as_str().unwrap().contains("changed"));
}

#[test]
fn console_runs_statements_like_a_cli() {
    let (_d, s) = setup("console");
    let mut app = DbCraftApp::new(s, None).waiting_for_tasks();
    app.startup(StartupAction::Console { connection: "src".into(), database: None });
    let mut h = harness(app);
    h.run();
    for line in ["select count(*) as n", "from film;"] {
        h.event(egui::Event::Text(line.into()));
        h.key_press(egui::Key::Enter);
        h.run();
    }
    let Some(Tab::Console(c)) = h.state().state.tabs.get(1) else { panic!("console tab") };
    assert!(c.scrollback.contains("    -> from film;"), "continuation prompt:\n{}", c.scrollback);
    assert!(c.scrollback.contains("| 3 |"), "{}", c.scrollback);
    assert!(c.scrollback.contains("1 row in set"), "{}", c.scrollback);
    h.event(egui::Event::Text("selec oops;".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    let Some(Tab::Console(c)) = h.state().state.tabs.get(1) else { panic!() };
    assert!(c.scrollback.contains("ERROR:"));
}

#[test]
fn data_transfer_from_the_ui() {
    let (d, s) = setup("transfer");
    let mut app = DbCraftApp::new(s, None).waiting_for_tasks();
    app.startup(StartupAction::Select { connection: "src".into(), database: Some("main".into()) });
    app.startup(StartupAction::Tool("transfer".into()));
    let mut h = harness(app);
    h.run();
    assert!(h.query_by_label_contains("film").is_some(), "source tables listed");
    // Target connection: dst (egui combo boxes don't expose their text to accesskit, so set it).
    if let Some(Tab::Transfer(f)) = h.state_mut().state.tabs.get_mut(1) {
        f.target = Some("dst".into());
    }
    h.run();
    h.get_by_label_contains("Start").click();
    wait(&mut h, |a| matches!(a.state.tabs.get(1), Some(Tab::Transfer(f)) if f.status.is_some()));
    let Some(Tab::Transfer(f)) = h.state().state.tabs.get(1) else { panic!() };
    assert!(f.status.as_deref().unwrap_or("").contains("3 rows"), "{:?}", f.status);
    let mut c = check(&d);
    let r = c.dispatch("query.run", &json!({"connection": "dst", "sql": "select count(*) from film"})).unwrap();
    assert_eq!(r["result"]["rows"][0][0]["v"], 3);
}

#[test]
fn backup_and_automation_from_the_ui() {
    let (d, s) = setup("auto");
    let mut app = DbCraftApp::new(s, None).waiting_for_tasks();
    app.startup(StartupAction::Select { connection: "src".into(), database: Some("main".into()) });
    app.startup(StartupAction::Mode("backups".into()));
    let mut h = harness(app);
    h.run();
    assert!(h.query_by_label_contains("No backups").is_some());
    // The toolbar button, not the empty-state hint that mentions it.
    h.get_all_by_label_contains("New Backup").min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    wait(&mut h, |a| a.state.objects.backups.as_ref().is_some_and(|b| b.len() == 1));

    // Automation: New job → Backup step on src/main → Save → Start.
    h.state_mut().state.tabs.truncate(1);
    h.state_mut().startup(StartupAction::Tool("automation".into()));
    h.run();
    h.get_by_label(&format!("{} New", dbcraft_ui_egui::icons::PLUS)).click();
    h.run();
    if let Some(Tab::Automation(v)) = h.state_mut().state.tabs.get_mut(1) {
        v.step_connection = Some("src".into());
        v.step_database = Some("main".into());
    }
    h.run();
    h.get_by_label_contains(" Add").click();
    h.run();
    h.get_by_label_contains(" Save").click();
    h.run();
    assert_eq!(h.state().session.workspace.automations.len(), 1);
    h.get_by_label_contains(" Start").click();
    wait(&mut h, |a| matches!(a.state.tabs.get(1), Some(Tab::Automation(v)) if v.status.is_some()));
    let Some(Tab::Automation(v)) = h.state().state.tabs.get(1) else { panic!() };
    assert!(v.status.as_deref().unwrap_or("").contains("ok"), "{:?}", v.status);
    let mut c = check(&d);
    assert_eq!(c.dispatch("backup.list", &json!({"connection": "src", "database": "main"})).unwrap().as_array().unwrap().len(), 2);
    assert!(h.state().session.workspace.automations[0].last_run.is_some(), "last run recorded");
}
