//! New Connection dialog, driven headlessly: tabs, SSH section, Test Connection in the background.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use dbcraft_ui_egui::{DbCraftApp, StartupAction};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn ssh_tab_and_test_connection() {
    let mut app = DbCraftApp::new(Session::new(), None);
    app.startup(StartupAction::NewConnection);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| app);
    h.run();
    assert!(h.query_by_label_contains("New Connection - MySQL").is_some());
    for tab in ["General", "Advanced", "Databases", "SSL", "SSH"] {
        assert!(h.query_by_label(tab).is_some(), "{tab} tab");
    }

    h.get_by_label("SSH").click();
    h.run();
    h.get_by_label("Use SSH Tunnel").click();
    h.run();
    assert!(h.query_by_label("Authentication Method:").is_some());
    assert!(h.state().state.connection_form.as_ref().unwrap().ssh_enabled);

    // Back to General, point at a port nothing listens on, test: the dialog stays responsive and
    // reports the failure.
    h.get_by_label("General").click();
    h.run();
    let form = h.state_mut().state.connection_form.as_mut().unwrap();
    form.ssh_enabled = false;
    form.host = "127.0.0.1".into();
    form.port = "1".into();
    h.run(); // the layout changes (no SSH notice); let it settle before locating the button
    h.get_by_label_contains("Test Connection").click();
    h.step(); // the click starts the worker
    h.step(); // the next frame shows it
    assert!(h.query_by_label_contains("Connecting").is_some(), "spinner while the worker connects");
    let mut reported = false;
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.step();
        if h.state().state.connection_form.as_ref().unwrap().test_result.is_some() {
            reported = true;
            break;
        }
    }
    assert!(reported, "test connection finished");
    assert!(matches!(h.state().state.connection_form.as_ref().unwrap().test_result, Some(Err(_))));
    h.step();
    assert!(h.query_by_label_contains("refused").is_some() || h.query_by_label_contains("10061").is_some() || h.query_by_label_contains("error").is_some());
}
