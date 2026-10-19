//! Tools › Structure Synchronization through the UI: open from the menu, Compare, review,
//! Deploy with confirmation, and the automatic compare after shows the structures match.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use dbcraft_ui_egui::DbCraftApp;
use dbcraft_ui_egui::state::Tab;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn form(h: &Harness<DbCraftApp>) -> dbcraft_ui_egui::structure_sync_tab::SyncForm {
    h.state().state.tabs.iter().find_map(|t| if let Tab::StructureSync(f) = t { Some((**f).clone()) } else { None }).expect("sync tab")
}

#[test]
fn compare_and_deploy_from_the_ui() {
    let d = std::env::temp_dir().join(format!("dbcraft-uissync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    for id in ["src", "dst"] {
        s.dispatch("connection.save", &json!({"id": id, "kind": "sqlite", "name": id, "database": d.join(format!("{id}.db")).to_string_lossy()})).unwrap();
        s.dispatch("connection.open", &json!({"id": id})).unwrap();
    }
    s.dispatch("query.run", &json!({"connection": "src", "sql": "create table film (id integer primary key, title text not null, year int); create index film_year on film (year)"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "dst", "sql": "create table film (id integer primary key, title text not null)"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| DbCraftApp::new(s, None).waiting_for_tasks());
    h.run();
    h.get_by_label("Tools").click();
    h.run();
    h.get_by_label("Structure Synchronization").click();
    h.run();
    if let Some(Tab::StructureSync(f)) = h.state_mut().state.tabs.last_mut() {
        f.source = Some("src".into());
        f.source_db = Some("main".into());
        f.target = Some("dst".into());
        f.target_db = Some("main".into());
    }
    h.run();
    h.get_by_label(&format!("{} Compare", dbcraft_ui_egui::icons::MAGNIFYING_GLASS)).click();
    h.run();
    let f = form(&h);
    assert_eq!(f.differences.len(), 1, "{:?}", f.error);
    assert_eq!((f.differences[0].object.as_str(), f.differences[0].action.as_str()), ("film", "alter"));
    assert!(h.query_by_label_contains("ADD COLUMN").is_some() || f.differences[0].statements.iter().any(|s| s.contains("ADD COLUMN")));

    h.get_by_label_contains("Deploy (1)").click();
    h.run();
    // The confirmation window: its title and its button are both "Deploy"; the button is lower.
    h.get_all_by_label("Deploy").max_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    h.step();
    for _ in 0..400 {
        h.step();
        let f = form(&h);
        if f.status.is_some() && f.task.is_none() && f.compared && f.differences.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let f = form(&h);
    assert!(f.status.as_deref().unwrap_or("").starts_with("Deployed 1"), "{:?} {:?}", f.status, f.error);
    assert!(f.differences.is_empty(), "compared again: identical");
    let mut check = Session::new();
    check.dispatch("connection.save", &json!({"id": "dst", "kind": "sqlite", "name": "dst", "database": d.join("dst.db").to_string_lossy()})).unwrap();
    check.dispatch("connection.open", &json!({"id": "dst"})).unwrap();
    let r = check.dispatch("query.run", &json!({"connection": "dst", "sql": "select count(*) from sqlite_master where name = 'film_year'"})).unwrap();
    assert_eq!(r["result"]["rows"][0][0]["v"], 1);
}
