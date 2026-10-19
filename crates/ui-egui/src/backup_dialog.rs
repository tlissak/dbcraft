//! The Backup window (Navicat: Tools › Backup › New Backup): General (comment), Object Selection
//! (all during execution, or custom tables and views), Advanced (lock all tables, single
//! transaction, specified file name), and the Message Log while it runs.

use dbcraft_core::TableInfo;
use dbcraft_sql::Dialect;
use egui::RichText;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app::Action;
use crate::jobs::Job;
use crate::theme::Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackupPage {
    #[default]
    General,
    Objects,
    Advanced,
    Log,
}

/// One object in Object Selection: (schema, name, ticked).
pub type Pick = (Option<String>, String, bool);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupForm {
    pub connection: String,
    pub database: String,
    pub page: BackupPage,
    pub comment: String,
    /// Object Selection: `false` = all objects at the time it runs.
    pub custom: bool,
    pub tables: Vec<Pick>,
    pub views: Vec<Pick>,
    pub lock_tables: bool,
    pub single_transaction: bool,
    pub use_file_name: bool,
    pub file_name: String,
    #[serde(skip)]
    pub job: Job,
}

impl BackupForm {
    pub fn new(connection: String, database: String, objects: &[TableInfo]) -> Self {
        let pick = |kind: &str| objects.iter().filter(|t| t.kind == kind).map(|t| (t.schema.clone(), t.name.clone(), true)).collect();
        BackupForm { connection, database, tables: pick("table"), views: pick("view"), ..BackupForm::default() }
    }

    /// `backup.create` params.
    pub fn params(&self) -> Result<serde_json::Value, String> {
        let mut p = json!({
            "connection": self.connection, "database": self.database, "comment": self.comment,
            "lock_tables": self.lock_tables, "single_transaction": self.single_transaction,
        });
        if self.custom {
            let tables: Vec<serde_json::Value> = self.tables.iter().filter(|t| t.2).map(|t| json!({ "schema": t.0, "name": t.1 })).collect();
            if tables.is_empty() {
                return Err("Choose at least one table in Object Selection.".into());
            }
            p["tables"] = json!(tables);
            p["views"] = json!(self.views.iter().filter(|v| v.2).map(|v| v.1.clone()).collect::<Vec<_>>());
        }
        if self.use_file_name {
            if self.file_name.trim().is_empty() {
                return Err("Enter a file name, or untick \"Use specified file name\".".into());
            }
            p["file_name"] = json!(self.file_name.trim());
        }
        Ok(p)
    }
}

pub enum Outcome {
    Open,
    Close,
}

fn checklist(ui: &mut egui::Ui, title: &str, items: &mut [Pick]) {
    ui.horizontal(|ui| {
        ui.strong(title);
        if ui.small_button("Select All").clicked() {
            items.iter_mut().for_each(|i| i.2 = true);
        }
        if ui.small_button("Unselect All").clicked() {
            items.iter_mut().for_each(|i| i.2 = false);
        }
    });
    if items.is_empty() {
        ui.weak("(none)");
    }
    for (schema, name, on) in items.iter_mut() {
        let label = match schema {
            Some(s) if s != "public" => format!("{s}.{name}"),
            _ => name.clone(),
        };
        ui.checkbox(on, label);
    }
}

pub fn show(ctx: &egui::Context, form: &mut BackupForm, dialect: Dialect, actions: &mut Vec<Action>) -> Outcome {
    let mut out = Outcome::Open;
    let mut open = true;
    let running = form.job.running();
    egui::Window::new(format!("Backup - {}", form.database))
        .id(egui::Id::new("backup_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        // Fixed, like Navicat's: an auto-sized window with growing content keeps resizing.
        .fixed_size([620.0, 400.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (p, label) in [
                    (BackupPage::General, "General"),
                    (BackupPage::Objects, "Object Selection"),
                    (BackupPage::Advanced, "Advanced"),
                    (BackupPage::Log, "Message Log"),
                ] {
                    if ui.add(egui::Button::selectable(form.page == p, label)).clicked() {
                        form.page = p;
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, false]).show(ui, |ui| match form.page {
                BackupPage::General => {
                    ui.label("Comment");
                    ui.add(
                        egui::TextEdit::multiline(&mut form.comment)
                            .desired_width(f32::INFINITY)
                            .desired_rows(8)
                            .hint_text("What this backup is for (optional)"),
                    );
                }
                BackupPage::Objects => {
                    ui.radio_value(&mut form.custom, false, "All during execution");
                    ui.radio_value(&mut form.custom, true, "Custom");
                    if form.custom {
                        ui.add_space(4.0);
                        ui.columns(2, |cols| {
                            if let Some(c) = cols.get_mut(0) {
                                checklist(c, "Tables", &mut form.tables);
                            }
                            if let Some(c) = cols.get_mut(1) {
                                checklist(c, "Views", &mut form.views);
                            }
                        });
                    } else {
                        ui.weak("Every table and view of the database when the backup runs.");
                    }
                }
                BackupPage::Advanced => {
                    ui.checkbox(&mut form.lock_tables, "Lock All Tables");
                    ui.weak("Other sessions can read but not write while the backup runs (consistent copy).");
                    let label = if dialect == Dialect::Mysql { "Use Single Transaction (InnoDB only)" } else { "Use Single Transaction" };
                    ui.checkbox(&mut form.single_transaction, label);
                    ui.weak("Reads everything in one snapshot transaction, without blocking writers.");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut form.use_file_name, "Use specified file name");
                        ui.add_enabled(form.use_file_name, egui::TextEdit::singleline(&mut form.file_name).desired_width(240.0).hint_text("YYYYMMDDhhmmss"));
                    });
                }
                BackupPage::Log => {
                    if form.job.started() {
                        crate::jobs::show_log(ui, &form.job, 250.0);
                    } else {
                        ui.weak("The backup's progress appears here.");
                    }
                }
            });
            ui.separator();
            let error = form.params().err();
            ui.horizontal(|ui| {
                if let Some(e) = &error {
                    ui.colored_label(Tokens::error(), e);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        out = Outcome::Close;
                    }
                    if ui.add_enabled(!running && error.is_none(), egui::Button::new(RichText::new("Backup").strong())).clicked() {
                        actions.push(Action::BackupStart);
                        form.page = BackupPage::Log;
                    }
                });
            });
        });
    if !open {
        out = Outcome::Close;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_follow_the_tabs() {
        let objects = vec![
            TableInfo { name: "film".into(), kind: "table".into(), ..TableInfo::default() },
            TableInfo { name: "actor".into(), kind: "table".into(), ..TableInfo::default() },
            TableInfo { name: "titles".into(), kind: "view".into(), ..TableInfo::default() },
        ];
        let mut f = BackupForm::new("c".into(), "db".into(), &objects);
        let p = f.params().unwrap();
        assert!(p.get("tables").is_none(), "all during execution");
        f.custom = true;
        f.tables[1].2 = false;
        f.views[0].2 = false;
        let p = f.params().unwrap();
        assert_eq!(p["tables"], json!([{"schema": null, "name": "film"}]));
        assert_eq!(p["views"], json!([]));
        f.tables[0].2 = false;
        assert!(f.params().is_err(), "nothing chosen");
        f.tables[0].2 = true;
        f.use_file_name = true;
        assert!(f.params().is_err(), "empty file name");
        f.file_name = "before upgrade".into();
        f.lock_tables = true;
        let p = f.params().unwrap();
        assert_eq!((p["file_name"].as_str(), p["lock_tables"].as_bool()), (Some("before upgrade"), Some(true)));
    }
}
