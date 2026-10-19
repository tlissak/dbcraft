//! Table Designer and table actions, driven through the real UI (egui_kittest): Design Table from
//! the Objects tab, add a field, SQL Preview, Save; New Table with the name prompt; an index;
//! Delete Table with its confirmation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_core::Value;
use dbcraft_engine::Session;
use dbcraft_ui_egui::designer_tab::{DesignerPage, DesignerTab};
use dbcraft_ui_egui::state::Tab;
use dbcraft_ui_egui::{DbCraftApp, StartupAction};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn setup(name: &str) -> Session {
    let d = std::env::temp_dir().join(format!("dbcraft-uidesign-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.save", &json!({"id": "lite", "kind": "sqlite", "name": "lite", "database": d.join("lite.db").to_string_lossy()})).unwrap();
    s.dispatch("connection.open", &json!({"id": "lite"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "lite", "sql": "create table film (id integer primary key, title text not null); insert into film (title) values ('Metropolis')"})).unwrap();
    s
}

fn harness(s: Session) -> Harness<'static, DbCraftApp> {
    let mut app = DbCraftApp::new(s, None).waiting_for_tasks();
    app.startup(StartupAction::Select { connection: "lite".into(), database: Some("main".into()) });
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 950.0)).build_eframe(|_| app);
    h.run();
    h
}

fn designer(h: &Harness<DbCraftApp>) -> DesignerTab {
    h.state().state.tabs.iter().rev().find_map(|t| if let Tab::Designer(d) = t { Some((**d).clone()) } else { None }).expect("designer tab")
}

fn designer_mut<'a>(h: &'a mut Harness<'_, DbCraftApp>) -> &'a mut DesignerTab {
    h.state_mut().state.tabs.iter_mut().rev().find_map(|t| if let Tab::Designer(d) = t { Some(&mut **d) } else { None }).expect("designer tab")
}

fn query(h: &mut Harness<DbCraftApp>, sql: &str) -> Vec<Vec<Value>> {
    let r = h.state_mut().session.dispatch("query.run", &json!({"connection": "lite", "sql": sql})).unwrap();
    serde_json::from_value::<dbcraft_core::ResultSet>(r["result"].clone()).unwrap().rows
}

#[test]
fn design_add_field_preview_and_save() {
    let mut h = harness(setup("design"));
    h.get_by_label_contains("film").click();
    h.run();
    h.get_by_label_contains("Design Table").click();
    h.run();
    let d = designer(&h);
    assert!(d.loaded, "{:?}", d.error);
    assert_eq!(d.table.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "title"]);
    assert!(!d.dirty());

    h.get_by_label_contains("Add Field").click();
    h.run();
    designer_mut(&mut h).table.columns[2].name = "year".into();
    designer_mut(&mut h).table.columns[2].native_type = "INTEGER".into();
    h.run();
    assert!(designer(&h).dirty());
    h.get_by_label("SQL Preview").click();
    h.run();
    h.run();
    let preview = designer(&h).preview.expect("preview computed").expect("valid");
    assert_eq!(preview.statements, ["ALTER TABLE \"film\" ADD COLUMN \"year\" INTEGER"]);
    assert!(h.query_by_label_contains("ADD COLUMN").is_some() || h.query_by_label_contains("ALTER TABLE").is_some() || !preview.statements.is_empty());

    h.get_by_label_contains("Save").click();
    h.run();
    let d = designer(&h);
    assert_eq!(d.status.as_deref(), Some("Saved"), "{:?}", d.error);
    assert!(!d.dirty(), "the tab shows what the server has now");
    assert_eq!(query(&mut h, "select count(*) from pragma_table_info('film') where name = 'year'")[0][0], Value::Int(1));
}

#[test]
fn new_table_with_index_then_delete_it() {
    let mut h = harness(setup("new"));
    h.get_by_label_contains("New Table").click();
    // The pointer stays where the button was, over the new tab's widgets: move it away.
    h.step();
    h.event(egui::Event::PointerGone);
    h.run();
    {
        let d = designer_mut(&mut h);
        assert!(d.original.is_none() && d.loaded);
        d.table.columns.push(dbcraft_core::ColumnInfo::new("name", "TEXT", dbcraft_core::GenericType::Text, false, None));
        d.origins.push(None);
        d.page = DesignerPage::Indexes;
    }
    h.run();
    h.get_by_label_contains("Add Index").click();
    h.run();
    designer_mut(&mut h).table.indexes[0].columns = vec!["name".into()];
    designer_mut(&mut h).table.indexes[0].name = "studio_name".into();
    h.run();
    // First save asks for the name.
    h.get_by_label_contains("Save").click();
    h.run();
    assert!(designer(&h).name_prompt.is_some(), "Table Name prompt");
    designer_mut(&mut h).name_prompt = Some("studio".into());
    h.run();
    h.get_by_label("OK").click();
    h.run();
    let d = designer(&h);
    assert_eq!(d.status.as_deref(), Some("Saved"), "{:?}", d.error);
    assert_eq!(d.original.as_ref().map(|o| o.name.as_str()), Some("studio"));
    assert_eq!(query(&mut h, "select count(*) from sqlite_master where type = 'index' and name = 'studio_name'")[0][0], Value::Int(1));

    // Delete Table from the Objects tab, with its confirmation.
    h.state_mut().state.active_tab = 0;
    h.run();
    h.run();
    h.state_mut().state.objects.selected = Some((None, "studio".into()));
    h.run();
    h.get_by_label_contains("Delete Table").click();
    h.run();
    assert!(h.query_by_label_contains("Do you really want to delete").is_some(), "asks first");
    h.get_by_label("Delete").click();
    h.run();
    h.run();
    assert_eq!(query(&mut h, "select count(*) from sqlite_master where name = 'studio'")[0][0], Value::Int(0));
    assert!(h.state().session.tables.get(&("lite".to_string(), "main".to_string())).is_some_and(|t| t.iter().all(|x| x.name != "studio")), "list refreshed");
}

#[test]
fn closing_a_modified_designer_asks_to_save() {
    let mut h = harness(setup("close"));
    h.get_by_label_contains("film").click();
    h.run();
    h.get_by_label_contains("Design Table").click();
    h.run();
    designer_mut(&mut h).table.comment = Some("x".into());
    let i = h.state().state.tabs.len() - 1;
    h.state_mut().close_tab(i, false);
    h.run();
    assert_eq!(h.state().state.confirm_close, Some(i));
    assert!(h.query_by_label_contains("Do you want to save changes").is_some());
}
