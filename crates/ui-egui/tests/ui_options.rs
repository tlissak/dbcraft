//! Tools › Options (ROADMAP 1.4/1.5), driven headlessly: dark theme, records per page, display
//! formats, a rejected format, and persistence in the workspace.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use dbcraft_ui_egui::state::Tab;
use dbcraft_ui_egui::{DbCraftApp, StartupAction};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn setup(name: &str) -> (std::path::PathBuf, Session) {
    let d = std::env::temp_dir().join(format!("dbcraft-uiopts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.save", &json!({"id": "lite", "kind": "sqlite", "name": "lite", "database": d.join("lite.db").to_string_lossy()})).unwrap();
    s.dispatch("connection.open", &json!({"id": "lite"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "lite", "sql": "create table t (id integer primary key, born date, note text); insert into t (born, note) values ('1927-01-10', null), ('1931-03-11', 'x'), ('1936-02-05', null)"})).unwrap();
    (d, s)
}

fn harness(app: DbCraftApp) -> Harness<'static, DbCraftApp> {
    Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app)
}

#[test]
fn options_dialog_changes_theme_records_and_formats() {
    let (d, s) = setup("dialog");
    let mut h = harness(DbCraftApp::new(s, None).waiting_for_tasks());
    h.run();
    assert!(!dbcraft_ui_egui::theme::palette().dark);

    // Tools › Options.
    h.get_by_label("Tools").click();
    h.run();
    h.get_by_label("Options").click();
    h.run();
    assert!(h.state().state.options_form.is_some(), "dialog open");
    h.get_by_label("Dark").click();
    h.run();
    h.get_by_label("Records").click();
    h.run();
    {
        let form = h.state_mut().state.options_form.as_mut().unwrap();
        form.options.page_size = 2;
        form.options.null_text = "<none>".into();
        form.options.date_format = "%d.%m.%Y".into();
        form.options.big_icons = false;
    }
    h.run();
    h.get_by_label("OK").click();
    h.run();
    assert!(h.state().state.options_form.is_none(), "OK closes the dialog");
    let o = &h.state().session.workspace.options;
    assert_eq!((o.theme.as_str(), o.page_size, o.big_icons), ("dark", 2, false));
    assert!(dbcraft_ui_egui::theme::palette().dark, "dark theme applied");

    // A table opened now loads 2 rows per page and shows the formats.
    h.state_mut().startup(StartupAction::OpenTable { connection: "lite".into(), database: "main".into(), schema: None, table: "t".into() });
    h.run();
    match h.state().state.tabs.last() {
        Some(Tab::Table(t)) => {
            let page = t.page.as_ref().expect("page loaded");
            assert_eq!((page.result.rows.len(), page.total), (2, 3));
        }
        _ => panic!("no table tab"),
    }
    assert!(h.query_by_label("<none>").is_some(), "NULL text from the options");
    assert!(h.query_by_label("10.01.1927").is_some(), "date format from the options");

    // Saved in the workspace file.
    let (back, _) = Session::with_workspace_file(d.join("workspace.json"));
    assert_eq!(back.workspace.options.theme, "dark");
    assert_eq!(back.workspace.options.page_size, 2);
}

#[test]
fn a_bad_format_is_refused_and_the_dialog_stays_open() {
    let (_d, s) = setup("bad");
    let mut h = harness(DbCraftApp::new(s, None).waiting_for_tasks());
    h.run();
    h.state_mut().state.options_form = Some(dbcraft_ui_egui::options_dialog::OptionsForm::new(Default::default()));
    h.run();
    h.state_mut().state.options_form.as_mut().unwrap().options.time_format = "%Q".into();
    h.run();
    h.get_by_label("OK").click();
    h.run();
    let form = h.state().state.options_form.as_ref().expect("still open");
    assert!(form.error.as_deref().unwrap_or("").contains("not a valid date/time format"), "{:?}", form.error);
    assert_eq!(h.state().session.workspace.options.time_format, "", "nothing saved");
    h.get_by_label("Cancel").click();
    h.run();
    assert!(h.state().state.options_form.is_none());
}

/// Regression: the dialog grew 19 px per frame (a vertical separator in an auto-sized window), so
/// clicks landed on the row below the one aimed at.
#[test]
fn the_dialog_keeps_its_size() {
    let (_d, s) = setup("size");
    let mut h = harness(DbCraftApp::new(s, None).waiting_for_tasks());
    h.run();
    h.state_mut().state.options_form = Some(dbcraft_ui_egui::options_dialog::OptionsForm::new(Default::default()));
    h.run();
    let first = h.get_by_label("OK").rect();
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(h.get_by_label("OK").rect(), first);
    h.get_by_label("Editor").click();
    h.run();
    assert_eq!(h.state().state.options_form.as_ref().map(|f| f.page), Some(dbcraft_ui_egui::options_dialog::OptionsPage::Editor));
}

/// Regression (owner report): hovering a button added a border that grew it and pushed its
/// neighbours. Framed and frameless buttons must keep their size on hover.
#[test]
fn hover_never_moves_neighbours() {
    let (_d, s) = setup("hover");
    let mut app = DbCraftApp::new(s, None).waiting_for_tasks();
    app.startup(StartupAction::NewQuery { connection: "lite".into(), database: Some("main".into()), sql: String::new() });
    let mut h = harness(app);
    h.run();
    // Framed button: Save, its neighbour Save As.
    let before = h.get_by_label("Save As").rect();
    h.get_by_label(&format!("{} Save", egui_phosphor::regular::FLOPPY_DISK)).hover();
    h.run();
    assert_eq!(h.get_by_label("Save As").rect(), before, "framed button");
    // Frameless selectable buttons: the Designer's page strip.
    h.state_mut().state.tabs.push(Tab::Designer(Box::new(dbcraft_ui_egui::designer_tab::DesignerTab::new_table(
        "lite".into(),
        Some("main".into()),
        None,
        dbcraft_sql::Dialect::Sqlite,
    ))));
    let last = h.state().state.tabs.len() - 1;
    h.state_mut().state.active_tab = last;
    h.run();
    let before = h.get_by_label("Foreign Keys").rect();
    h.get_by_label("Indexes").hover();
    h.run();
    assert_eq!(h.get_by_label("Foreign Keys").rect(), before, "frameless button");
}
