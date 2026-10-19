//! Tools › Data Transfer: source → target connection (any engine) or SQL file, table selection,
//! options, Start (background job with a live log), Save as Batch Job.

use dbcraft_engine::Session;
use egui::{RichText, Ui};
use serde_json::{Value as Json, json};

use crate::app::Action;
use crate::icons;
use crate::state::TransferForm;
use crate::theme::Tokens;

impl TransferForm {
    pub fn new() -> Self {
        TransferForm { create_tables: true, create_records: true, batch_size: "500".into(), dialect: "mysql".into(), ..TransferForm::default() }
    }

    /// `tools.dataTransfer` params, or why the form isn't ready.
    pub fn params(&self) -> Result<Json, String> {
        let src = self.source.clone().ok_or("Choose a source connection")?;
        let tables: Vec<Json> = self.tables.iter().filter(|t| t.2).map(|(s, n, _)| json!({"schema": s, "name": n})).collect();
        if tables.is_empty() {
            return Err("Select at least one table".into());
        }
        let target = if self.to_file {
            if self.file.trim().is_empty() {
                return Err("Choose a target file".into());
            }
            json!({"file": self.file.trim(), "dialect": self.dialect})
        } else {
            json!({"connection": self.target.clone().ok_or("Choose a target connection")?, "database": self.target_db})
        };
        let batch: u64 = self.batch_size.trim().parse().map_err(|_| "Records per batch must be a number".to_string())?;
        Ok(json!({
            "source": {"connection": src, "database": self.source_db},
            "target": target,
            "tables": tables,
            "options": {"create_tables": self.create_tables, "drop_existing": self.drop_existing, "create_records": self.create_records,
                        "continue_on_error": self.continue_on_error, "batch_size": batch},
        }))
    }
}

pub(crate) fn conn_db_pickers(ui: &mut Ui, id: &str, session: &Session, conn: &mut Option<String>, db: &mut Option<String>) -> bool {
    let mut changed = false;
    let name = conn.as_ref().and_then(|c| session.workspace.connection(c).ok()).map(|c| c.name.clone()).unwrap_or_else(|| "(choose)".into());
    ui.horizontal(|ui| {
        ui.label("Connection:");
        egui::ComboBox::from_id_salt((id, "conn")).width(200.0).selected_text(name).show_ui(ui, |ui| {
            for c in &session.workspace.connections {
                if ui.selectable_value(conn, Some(c.id.clone()), &c.name).changed() {
                    *db = None;
                    changed = true;
                }
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label("Database:");
        let dbs = conn.as_ref().and_then(|c| session.databases.get(c)).cloned().unwrap_or_default();
        let hint = if conn.as_ref().is_some_and(|c| !session.is_open(c)) { "(opens on use)" } else { "" };
        egui::ComboBox::from_id_salt((id, "db")).width(200.0).selected_text(db.clone().unwrap_or_else(|| hint.into())).show_ui(ui, |ui| {
            for d in &dbs {
                if ui.selectable_value(db, Some(d.clone()), d).changed() {
                    changed = true;
                }
            }
        });
    });
    changed
}

pub fn show(ui: &mut Ui, idx: usize, form: &mut TransferForm, session: &Session, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().id_salt(("xfer", idx)).show(ui, |ui| {
        ui.add_space(6.0);
        ui.heading(format!("{} Data Transfer", egui_phosphor::regular::ARROWS_LEFT_RIGHT));
        ui.weak("Copy tables between connections (any engines) or into a SQL file. Columns, NOT NULL and primary keys are carried over.");
        ui.add_space(8.0);
        ui.columns(2, |cols| {
            cols[0].group(|ui| {
                ui.strong("Source");
                if conn_db_pickers(ui, "xsrc", session, &mut form.source, &mut form.source_db) {
                    form.tables.clear();
                    actions.push(Action::TransferLoadTables(idx));
                }
            });
            cols[1].group(|ui| {
                ui.horizontal(|ui| {
                    ui.strong("Target");
                    ui.radio_value(&mut form.to_file, false, "Connection");
                    ui.radio_value(&mut form.to_file, true, "File");
                });
                if form.to_file {
                    ui.horizontal(|ui| {
                        ui.label("SQL dialect:");
                        egui::ComboBox::from_id_salt(("xdialect", idx)).selected_text(&form.dialect).show_ui(ui, |ui| {
                            for d in ["mysql", "postgres", "sqlite"] {
                                ui.selectable_value(&mut form.dialect, d.to_string(), d);
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("File:");
                        ui.add(egui::TextEdit::singleline(&mut form.file).desired_width(220.0));
                        if ui.button("…").clicked()
                            && let Some(p) = rfd::FileDialog::new().add_filter("SQL", &["sql"]).set_file_name("transfer.sql").save_file()
                        {
                            form.file = p.to_string_lossy().into_owned();
                        }
                    });
                } else {
                    conn_db_pickers(ui, "xdst", session, &mut form.target, &mut form.target_db);
                }
            });
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Objects");
                if ui.small_button("Select All").clicked() {
                    form.tables.iter_mut().for_each(|t| t.2 = true);
                }
                if ui.small_button("Unselect All").clicked() {
                    form.tables.iter_mut().for_each(|t| t.2 = false);
                }
                ui.weak(format!("{} of {} selected", form.tables.iter().filter(|t| t.2).count(), form.tables.len()));
            });
            if form.tables.is_empty() {
                ui.weak(if form.source_db.is_some() { "No tables (or still loading)." } else { "Choose a source database." });
            }
            egui::ScrollArea::vertical().id_salt(("xtables", idx)).max_height(180.0).show(ui, |ui| {
                for (schema, name, on) in &mut form.tables {
                    let label = match schema {
                        Some(s) if s != "public" => format!("{s}.{name}"),
                        _ => name.clone(),
                    };
                    ui.checkbox(on, RichText::new(format!("{}  {label}", icons::TABLE)));
                }
            });
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong("Options");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut form.create_tables, "Create tables");
                ui.checkbox(&mut form.drop_existing, "Drop target objects before create");
                ui.checkbox(&mut form.create_records, "Create records");
                ui.checkbox(&mut form.continue_on_error, "Continue on error");
            });
            ui.horizontal(|ui| {
                ui.label("Records per batch:");
                ui.add(egui::TextEdit::singleline(&mut form.batch_size).desired_width(70.0));
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let running = form.job.running();
            let ready = form.params();
            let start =
                ui.add_enabled(!running && ready.is_ok(), egui::Button::new(RichText::new(format!("{} Start", icons::PLAY)).strong().color(Tokens::success())));
            if start.clicked() {
                actions.push(Action::TransferStart(idx));
            }
            if ui.add_enabled(ready.is_ok(), egui::Button::new(format!("{} Save as Batch Job", egui_phosphor::regular::ROBOT))).clicked() {
                actions.push(Action::TransferSaveJob(idx));
            }
            if let Err(why) = ready {
                ui.weak(why);
            }
            if let Some(s) = &form.status {
                ui.colored_label(Tokens::accent(), s);
            }
        });
        ui.add_space(6.0);
        crate::jobs::show_log(ui, &form.job, 220.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_need_source_tables_and_target() {
        let mut f = TransferForm::new();
        assert!(f.params().is_err());
        f.source = Some("a".into());
        f.source_db = Some("main".into());
        f.tables = vec![(None, "film".into(), true), (None, "tag".into(), false)];
        assert_eq!(f.params().unwrap_err(), "Choose a target connection");
        f.to_file = true;
        f.file = "out.sql".into();
        let p = f.params().unwrap();
        assert_eq!(p["tables"], json!([{"schema": null, "name": "film"}]));
        assert_eq!(p["target"], json!({"file": "out.sql", "dialect": "mysql"}));
        f.batch_size = "lots".into();
        assert!(f.params().is_err());
    }
}
