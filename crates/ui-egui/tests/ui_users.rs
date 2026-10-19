//! User manager through the UI against the docker PostgreSQL (opt-in):
//!   cargo test -p dbcraft-ui-egui --test ui_users -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_core::Grant;
use dbcraft_engine::Session;
use dbcraft_ui_egui::state::Tab;
use dbcraft_ui_egui::{DbCraftApp, StartupAction, icons};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn pg_session() -> Session {
    let mut s = Session::new();
    s.dispatch("connection.save", &json!({"id": "pg", "kind": "postgres", "name": "PG", "host": "127.0.0.1", "port": 15432, "user": "dbcraft", "password": "dbcraft", "database": "sakila_lite"}))
        .unwrap();
    s
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn create_grant_and_delete_a_user_from_the_ui() {
    let mut admin = pg_session();
    let _ = admin.dispatch("user.delete", &json!({"connection": "pg", "name": "ui_test_user"}));

    let mut app = DbCraftApp::new(pg_session(), None).waiting_for_tasks();
    app.startup(StartupAction::Select { connection: "pg".into(), database: None });
    app.startup(StartupAction::Mode("users".into()));
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 950.0)).build_eframe(|_| app);
    h.run();
    assert!(h.query_by_label_contains("dbcraft").is_some(), "existing users listed");

    h.get_by_label_contains("New User").click();
    h.run();
    {
        let Some(Tab::User(t)) = h.state_mut().state.tabs.get_mut(1) else { panic!("user tab") };
        t.user.name = "ui_test_user".into();
        t.password = "Pw-1234".into();
        t.confirm = "Pw-1234".into();
        t.user.grants.push(Grant { object: "TABLE public.film".into(), privileges: vec!["SELECT".into()], grant_option: false });
    }
    h.run();
    // SQL Preview shows what Save will run (password masked).
    h.get_by_label("SQL Preview").click();
    h.run();
    let Some(Tab::User(t)) = h.state().state.tabs.get(1) else { panic!() };
    assert!(t.preview.iter().any(|s| s.starts_with("CREATE ROLE \"ui_test_user\"") && s.contains("PASSWORD '******'")), "{:?}", t.preview);
    assert!(t.preview.iter().any(|s| s.contains("GRANT SELECT ON TABLE \"public\".\"film\"")));

    h.get_by_label(&format!("{} Save", egui_phosphor::regular::FLOPPY_DISK)).click();
    h.run();
    let Some(Tab::User(t)) = h.state().state.tabs.get(1) else { panic!() };
    assert!(t.saved, "{:?}", t.error);
    let got = admin.dispatch("user.get", &json!({"connection": "pg", "name": "ui_test_user"})).unwrap();
    assert_eq!(got["grants"][0]["privileges"], json!(["SELECT"]));

    // The new user can log in.
    let mut login = Session::new();
    login
        .dispatch("connection.save", &json!({"id": "u", "kind": "postgres", "name": "u", "host": "127.0.0.1", "port": 15432, "user": "ui_test_user", "password": "Pw-1234", "database": "sakila_lite"}))
        .unwrap();
    login.dispatch("connection.test", &json!({"kind": "postgres", "name": "u", "host": "127.0.0.1", "port": 15432, "user": "ui_test_user", "password": "Pw-1234", "database": "sakila_lite"})).unwrap();

    // Back to the list: select, Delete, confirm.
    h.state_mut().state.active_tab = 0;
    h.state_mut().state.objects.users = None;
    h.run();
    h.get_by_label(&format!("{}  ui_test_user", egui_phosphor::regular::USER)).click();
    h.run();
    h.get_by_label(&format!("{} Delete", icons::TRASH)).click();
    h.run();
    h.get_by_label("Delete").click();
    h.run();
    assert!(admin.dispatch("user.get", &json!({"connection": "pg", "name": "ui_test_user"})).is_err(), "deleted");
}
