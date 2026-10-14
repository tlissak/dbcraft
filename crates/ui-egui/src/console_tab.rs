//! Tools › Console: a command-line session on one connection, like the mysql/psql/sqlite3
//! clients. Statements end with `;` (several lines are fine); `use db` / `\c db` switch the
//! database, `clear` clears the screen, Up/Down browse history.

use dbcraft_core::DbKind;
use dbcraft_engine::Session;
use egui::{RichText, Ui};

use crate::app::Action;
use crate::state::ConsoleTab;
use crate::theme::Tokens;

pub fn prompt(kind: Option<DbKind>, tab: &ConsoleTab) -> String {
    if !tab.pending.is_empty() {
        return "    -> ".into();
    }
    match kind {
        Some(DbKind::Postgres) => format!("{}=# ", tab.database.as_deref().unwrap_or("postgres")),
        Some(DbKind::Mysql) => "mysql> ".into(),
        Some(DbKind::Mariadb) => format!("MariaDB [{}]> ", tab.database.as_deref().unwrap_or("(none)")),
        _ => "sqlite> ".into(),
    }
}

/// Whether the input completes a command to run now.
pub fn complete(pending: &str, input: &str) -> bool {
    let all = format!("{pending}{input}");
    let t = all.trim();
    let lower = t.to_ascii_lowercase();
    t.ends_with(';') || matches!(lower.as_str(), "clear" | "exit" | "quit" | "help") || lower.starts_with("use ") || lower.starts_with("\\c ")
}

pub fn show(ui: &mut Ui, idx: usize, tab: &mut ConsoleTab, session: &Session, actions: &mut Vec<Action>) {
    let cfg = session.workspace.connection(&tab.connection).ok();
    let kind = cfg.map(|c| c.kind);
    egui::Frame::new().fill(Tokens::TOOLBAR_BG).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(egui_phosphor::regular::TERMINAL_WINDOW).color(Tokens::ACCENT));
            ui.strong(cfg.map(|c| c.name.as_str()).unwrap_or("?"));
            ui.weak(format!("database: {}", tab.database.as_deref().unwrap_or("(default)")));
            if ui.button("Clear").clicked() {
                tab.scrollback.clear();
            }
        });
    });
    let bg = egui::Color32::from_rgb(0x1e, 0x1f, 0x22);
    let fg = egui::Color32::from_rgb(0xdc, 0xdf, 0xe4);
    egui::Frame::new().fill(bg).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.visuals_mut().override_text_color = Some(fg);
        ui.visuals_mut().extreme_bg_color = bg;
        let avail = ui.available_height() - 28.0;
        egui::ScrollArea::vertical().id_salt(("console", idx)).max_height(avail.max(60.0)).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            let mut text = tab.scrollback.as_str();
            ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).frame(egui::Frame::NONE).desired_width(f32::INFINITY).text_color(fg));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(prompt(kind, tab)).monospace().color(egui::Color32::from_rgb(0x7f, 0xd9, 0x62)));
            let r = ui.add(
                egui::TextEdit::singleline(&mut tab.input)
                    .font(egui::TextStyle::Monospace)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .text_color(fg)
                    .id(egui::Id::new(("console_in", idx))),
            );
            // Check Enter before re-taking focus: requesting focus first would cancel `lost_focus`.
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if entered {
                actions.push(Action::ConsoleEnter(idx));
            }
            if r.has_focus() {
                let (up, down) =
                    ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)));
                if up && tab.history_pos > 0 {
                    tab.history_pos -= 1;
                    tab.input = tab.history.get(tab.history_pos).cloned().unwrap_or_default();
                }
                if down && tab.history_pos < tab.history.len() {
                    tab.history_pos += 1;
                    tab.input = tab.history.get(tab.history_pos).cloned().unwrap_or_default();
                }
            } else if entered || !ui.ctx().egui_wants_keyboard_input() {
                r.request_focus();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_end_with_semicolon_or_are_meta_commands() {
        assert!(complete("", "select 1;"));
        assert!(!complete("", "select 1"));
        assert!(complete("select\n", "1 ;  "));
        assert!(complete("", "use shop"));
        assert!(complete("", "\\c shop"));
        assert!(complete("", "CLEAR"));
    }
}
