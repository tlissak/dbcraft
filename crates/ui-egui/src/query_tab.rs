//! Query editor tab: SQL, Run (Ctrl+R), Save (Ctrl+S) / Save As, one result tab per statement
//! (Navicat: up to 20) plus Message.

use dbcraft_core::DbKind;
use dbcraft_engine::Session;
use dbcraft_sql::Dialect;
use egui::{RichText, Ui};
use egui_phosphor::regular as ph;

use crate::app::Action;
use crate::icons;
use crate::state::QueryTab;
use crate::theme::Tokens;

pub fn dialect_of(session: &Session, conn: Option<&str>) -> Dialect {
    match conn.and_then(|c| session.workspace.connection(c).ok()).map(|c| c.kind) {
        Some(DbKind::Postgres) => Dialect::Postgres,
        Some(DbKind::Mysql | DbKind::Mariadb) => Dialect::Mysql,
        _ => Dialect::Sqlite,
    }
}

pub fn show(ui: &mut Ui, idx: usize, tab: &mut QueryTab, conns: &[(String, String)], session: &Session, actions: &mut Vec<Action>) {
    let dbs = tab.connection.as_ref().and_then(|c| session.databases.get(c)).cloned().unwrap_or_default();
    egui::Frame::new().fill(Tokens::toolbar_bg()).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let save = ui.button(format!("{} Save", ph::FLOPPY_DISK)).on_hover_text("Save (Ctrl+S)").clicked()
                || ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
            if save {
                actions.push(Action::SaveQuery(idx));
            }
            if ui.button("Save As").clicked() {
                tab.save_prompt = Some(if tab.saved_id.is_some() { format!("{} copy", tab.title) } else { String::new() });
            }
            ui.separator();
            let current =
                tab.connection.as_ref().and_then(|id| conns.iter().find(|(c, _)| c == id)).map(|(_, n)| n.clone()).unwrap_or_else(|| "(no connection)".into());
            egui::ComboBox::from_id_salt(("qconn", idx)).width(170.0).selected_text(current).show_ui(ui, |ui| {
                for (id, name) in conns {
                    ui.selectable_value(&mut tab.connection, Some(id.clone()), name);
                }
            });
            egui::ComboBox::from_id_salt(("qdb", idx)).width(140.0).selected_text(tab.database.clone().unwrap_or_default()).show_ui(ui, |ui| {
                for db in &dbs {
                    ui.selectable_value(&mut tab.database, Some(db.clone()), db);
                }
            });
            let idle = tab.task.is_none();
            let run = ui
                .add_enabled(tab.connection.is_some() && idle, egui::Button::new(RichText::new(format!("{} Run", icons::PLAY)).color(Tokens::success())))
                .on_hover_text("Run (Ctrl+R)")
                .clicked()
                || ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R));
            if run && tab.connection.is_some() && idle {
                actions.push(Action::RunQuery(idx));
            }
            if let Some(task) = &tab.task
                && crate::tasks::running_bar(ui, task, "Running…")
            {
                actions.push(Action::StopTask(idx));
            }
            if tab.dirty() {
                ui.weak("● modified");
            }
        });
    });

    egui::Panel::bottom(egui::Id::new(("query_bottom", idx))).resizable(true).default_size(280.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (i, rs) in tab.results.iter().enumerate() {
                let label = if rs.columns.is_empty() { format!("Result {}", i + 1) } else { format!("Result {} ({})", i + 1, rs.rows.len()) };
                if ui.add(egui::Button::selectable(tab.result_idx == i, label)).clicked() {
                    tab.result_idx = i;
                    tab.grid = Default::default();
                }
            }
            let msg = tab.results.len();
            let text = if tab.error.is_some() { RichText::new("Message").color(Tokens::error()) } else { RichText::new("Message") };
            if ui.add(egui::Button::selectable(tab.result_idx >= msg, text)).clicked() {
                tab.result_idx = msg;
            }
        });
        ui.separator();
        match tab.results.get(tab.result_idx) {
            Some(rs) => {
                ui.weak(if rs.columns.is_empty() {
                    format!("{} rows affected   {} ms", rs.rows_affected, rs.elapsed_ms)
                } else {
                    format!("{} rows   {} ms", rs.rows.len(), rs.elapsed_ms)
                });
                let dialect = dialect_of(session, tab.connection.as_deref());
                crate::grid::result_grid(ui, egui::Id::new(("qgrid", idx, tab.result_idx)), rs, &mut tab.grid, dialect);
            }
            None => {
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    if let Some(e) = &tab.error {
                        ui.colored_label(Tokens::error(), e);
                        ui.separator();
                    }
                    for m in &tab.messages {
                        ui.monospace(m);
                        ui.separator();
                    }
                    if tab.messages.is_empty() && tab.error.is_none() {
                        ui.weak("Run a query to see results.");
                    }
                });
            }
        }
    });

    // Tools › Options › Editor: font size and word wrap.
    let (size, wrap) = (session.workspace.options.editor_font_size, session.workspace.options.word_wrap);
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let job = egui::text::LayoutJob::simple(
            text.as_str().to_string(),
            egui::FontId::monospace(size),
            ui.visuals().override_text_color.unwrap_or_else(Tokens::text),
            if wrap { wrap_width } else { f32::INFINITY },
        );
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let scroll = if wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
    scroll.id_salt(("qedit", idx)).show(ui, |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut tab.sql)
                .code_editor()
                .layouter(&mut layouter)
                .desired_width(f32::INFINITY)
                .desired_rows(18)
                .hint_text("SELECT * FROM …"),
        );
    });

    // Save As prompt.
    if let Some(name) = &mut tab.save_prompt {
        let mut open = true;
        let mut ok = false;
        egui::Window::new("Save Query As")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .show(ui.ctx(), |ui| {
                ui.label("Query Name:");
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(280.0));
                r.request_focus();
                ui.horizontal(|ui| {
                    ok = ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Save")).clicked()
                        || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !name.trim().is_empty());
                    if ui.button("Cancel").clicked() {
                        tab.close_after_save = false;
                        actions.push(Action::CancelSavePrompt(idx));
                    }
                });
            });
        if ok {
            actions.push(Action::SaveQueryAs(idx, name.trim().to_string()));
        } else if !open {
            actions.push(Action::CancelSavePrompt(idx));
        }
    }
}
