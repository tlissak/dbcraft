//! Drive the real app headlessly (egui_kittest): open a table, edit a cell, add a record, filter,
//! and check the database. This is what a user does with the mouse and keyboard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use dbcraft_ui_egui::state::Tab;
use dbcraft_ui_egui::{DbCraftApp, StartupAction, icons};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn app(name: &str) -> (DbCraftApp, Session) {
    let db = std::env::temp_dir().join(format!("dbcraft-ui-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let mut s = Session::new();
    let cfg = json!({"id": "c", "kind": "sqlite", "name": "Films", "database": db.to_string_lossy()});
    s.dispatch("connection.save", &cfg).unwrap();
    s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "c", "sql": "create table film (id integer primary key, title text, year int); insert into film (title, year) values ('Metropolis', 1927), ('Nosferatu', 1922)"}))
        .unwrap();
    // A second session on the same file checks what really reached the database.
    let mut check = Session::new();
    check.dispatch("connection.save", &cfg).unwrap();
    check.dispatch("connection.open", &json!({"id": "c"})).unwrap();
    let mut a = DbCraftApp::new(s, None).waiting_for_tasks();
    a.startup(StartupAction::OpenTable { connection: "c".into(), database: "main".into(), schema: None, table: "film".into() });
    (a, check)
}

fn titles(check: &mut Session) -> Vec<String> {
    let r = check.dispatch("query.run", &json!({"connection": "c", "sql": "select title from film order by id"})).unwrap();
    r["result"]["rows"].as_array().unwrap().iter().map(|row| row[0]["v"].as_str().unwrap_or("NULL").to_string()).collect()
}

#[test]
fn edit_cell_and_apply_writes_to_the_database() {
    let (app, mut check) = app("edit");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app);
    h.run();

    // Double-click a cell, type, Enter: the edit is pending, not saved.
    let cell = h.get_by_label("Metropolis");
    cell.click();
    cell.click();
    h.run();
    h.key_press(egui::Key::End);
    h.event(egui::Event::Text(" (restored)".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(h.query_by_label("Metropolis (restored)").is_some(), "edited text shown in the grid");
    assert_eq!(titles(&mut check), ["Metropolis", "Nosferatu"], "not saved before Apply");
    assert!(h.query_by_label_contains("pending changes").is_some());

    // Apply (✓) writes it.
    h.get_by_label(icons::CHECK).click();
    h.run();
    assert_eq!(titles(&mut check), ["Metropolis (restored)", "Nosferatu"]);
    assert!(h.query_by_label_contains("pending changes").is_none());
}

#[test]
fn moving_to_another_row_saves_the_record_like_navicat() {
    let (app, mut check) = app("autosave");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app);
    h.run();
    let cell = h.get_by_label("Metropolis");
    cell.click();
    cell.click();
    h.run();
    h.event(egui::Event::Text("!".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(titles(&mut check), ["Metropolis", "Nosferatu"], "same row: still pending");
    h.get_by_label("Nosferatu").click();
    h.run();
    assert_eq!(titles(&mut check), ["Metropolis!", "Nosferatu"], "saved on leaving the row");

    // Insert adds a record; type its title, leave the row, and it is inserted.
    h.key_press(egui::Key::Insert);
    h.run();
    // New row cells read "(default)": id, title, year. Click the title cell twice to edit it.
    // (One click edits if the cell is already selected, otherwise it selects it first.)
    let editing = |h: &Harness<DbCraftApp>| matches!(h.state().state.tabs.get(1), Some(Tab::Table(t)) if t.editing.is_some());
    for _ in 0..2 {
        if editing(&h) {
            break;
        }
        let mut cells: Vec<_> = h.get_all_by_label("(default)").collect();
        cells.sort_by(|a, b| a.rect().left().total_cmp(&b.rect().left()));
        cells[1].click();
        h.run();
    }
    assert!(editing(&h));
    h.event(egui::Event::Text("Faust".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    h.get_by_label("Nosferatu").click();
    h.run();
    assert_eq!(titles(&mut check), ["Metropolis!", "Nosferatu", "Faust"]);
}

#[test]
fn discard_drops_pending_edits() {
    let (app, mut check) = app("discard");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app);
    h.run();
    let cell = h.get_by_label("Nosferatu");
    cell.click();
    cell.click();
    h.run();
    h.event(egui::Event::Text("!!!".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    // The bottom bar's ✗ (Discard), not a tab's close ✗.
    h.get_all_by_label(icons::X).max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    h.run();
    assert!(h.query_by_label("Nosferatu").is_some());
    assert_eq!(titles(&mut check), ["Metropolis", "Nosferatu"]);
}

#[test]
fn filter_bar_filters_rows() {
    let (app, _check) = app("filter");
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app);
    h.run();
    h.get_by_label_contains("Filter").click();
    h.run();
    // Default filter row: first column (id) "is equal to"; enter 2 and apply.
    h.get_by_role(egui::accesskit::Role::TextInput).click();
    h.run();
    h.event(egui::Event::Text("2".into()));
    h.run();
    h.get_by_label_contains("Apply Filter").click();
    h.run();
    assert!(h.query_by_label("Nosferatu").is_some());
    assert!(h.query_by_label("Metropolis").is_none(), "filtered out");
    assert!(h.query_by_label_contains("of 1").is_some(), "record count reflects the filter");
}
