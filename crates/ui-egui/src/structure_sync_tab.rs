//! Tools › Structure Synchronization: pick source and target, Compare, review each difference
//! and its SQL, tick what to deploy, Deploy (background job with a log). Compares again after.

use dbcraft_engine::Session;
use egui::{RichText, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::app::Action;
use crate::jobs::Job;
use crate::tasks::Task;
use crate::theme::Tokens;

/// One difference from `tools.structureSync`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    pub object: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub action: String,
    pub statements: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncForm {
    pub source: Option<String>,
    pub source_db: Option<String>,
    pub target: Option<String>,
    pub target_db: Option<String>,
    /// PostgreSQL schemas (default `public`).
    pub source_schema: String,
    pub target_schema: String,
    pub drop_extra: bool,
    pub views: bool,
    pub differences: Vec<Difference>,
    pub checked: Vec<bool>,
    pub identical: usize,
    pub compared: bool,
    pub selected: usize,
    pub error: Option<String>,
    pub status: Option<String>,
    #[serde(skip)]
    pub task: Option<Task>,
    #[serde(skip)]
    pub job: Job,
    /// The deploy job's result has been handled.
    pub job_seen: bool,
}

impl SyncForm {
    pub fn new(source: Option<String>, source_db: Option<String>) -> Self {
        SyncForm { target: source.clone(), source, source_db, views: true, ..SyncForm::default() }
    }

    /// `tools.structureSync` params (compare; `deploy` adds the ticked objects).
    pub fn params(&self, deploy: bool) -> Result<Json, String> {
        let side = |conn: &Option<String>, db: &Option<String>, schema: &str, what: &str| -> Result<Json, String> {
            let c = conn.clone().ok_or(format!("Choose a {what} connection"))?;
            let d = db.clone().ok_or(format!("Choose a {what} database"))?;
            let schema = schema.trim();
            Ok(json!({ "connection": c, "database": d, "schema": if schema.is_empty() { None } else { Some(schema) } }))
        };
        let source = side(&self.source, &self.source_db, &self.source_schema, "source")?;
        let target = side(&self.target, &self.target_db, &self.target_schema, "target")?;
        if source == target {
            return Err("Source and target are the same".into());
        }
        let mut p = json!({ "source": source, "target": target, "options": { "drop_extra": self.drop_extra, "views": self.views } });
        if deploy {
            let objects: Vec<&str> = self.differences.iter().zip(&self.checked).filter(|(_, c)| **c).map(|(d, _)| d.object.as_str()).collect();
            if objects.is_empty() {
                return Err("Tick at least one difference to deploy".into());
            }
            p["deploy"] = json!(true);
            p["objects"] = json!(objects);
        }
        Ok(p)
    }

    /// Fill from a compare result.
    pub fn set_result(&mut self, v: &Json) {
        self.differences = serde_json::from_value(v["differences"].clone()).unwrap_or_default();
        // Drops are opt-in, like Navicat.
        self.checked = self.differences.iter().map(|d| d.action != "drop").collect();
        self.identical = v["identical"].as_array().map_or(0, Vec::len);
        self.compared = true;
        self.selected = 0;
        self.error = None;
    }
}

fn pickers(ui: &mut Ui, id: &str, session: &Session, conn: &mut Option<String>, db: &mut Option<String>, schema: &mut String, actions: &mut Vec<Action>) {
    if crate::transfer_tab::conn_db_pickers(ui, id, session, conn, db)
        && let Some(c) = conn.as_ref()
        && !session.is_open(c)
    {
        actions.push(Action::Command("connection.open", json!({ "id": c })));
    }
    let pg = conn.as_ref().and_then(|c| session.workspace.connection(c).ok()).is_some_and(|c| c.kind == dbcraft_core::DbKind::Postgres);
    if pg {
        ui.horizontal(|ui| {
            ui.label("Schema:");
            ui.add(egui::TextEdit::singleline(schema).desired_width(200.0).hint_text("public"));
        });
    }
}

pub fn show(ui: &mut Ui, idx: usize, f: &mut SyncForm, session: &Session, actions: &mut Vec<Action>) {
    ui.add_space(6.0);
    ui.heading(format!("{} Structure Synchronization", egui_phosphor::regular::GIT_DIFF));
    ui.weak("Make the target's structure match the source (same kind of engine). Review the SQL, then deploy.");
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        if let Some(c) = cols.get_mut(0) {
            c.group(|ui| {
                ui.strong("Source");
                pickers(ui, "ss_src", session, &mut f.source, &mut f.source_db, &mut f.source_schema, actions);
            });
        }
        if let Some(c) = cols.get_mut(1) {
            c.group(|ui| {
                ui.strong("Target");
                pickers(ui, "ss_dst", session, &mut f.target, &mut f.target_db, &mut f.target_schema, actions);
            });
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.views, "Compare views");
        ui.checkbox(&mut f.drop_extra, "Drop objects that are not in the source");
    });
    let busy = f.task.is_some() || f.job.running();
    let ready = f.params(false);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy && ready.is_ok(), egui::Button::new(RichText::new(format!("{} Compare", egui_phosphor::regular::MAGNIFYING_GLASS)).strong()))
            .clicked()
        {
            actions.push(Action::SyncCompare(idx));
        }
        let n = f.checked.iter().filter(|c| **c).count();
        if ui.add_enabled(!busy && f.compared && n > 0, egui::Button::new(format!("{} Deploy ({n})", egui_phosphor::regular::ROCKET_LAUNCH))).clicked() {
            let what = f.target_db.clone().unwrap_or_default();
            actions.push(Action::Confirm {
                title: "Deploy".into(),
                message: format!("Run the {n} selected change(s) on `{what}`? Dropped objects and their data can't be recovered."),
                button: "Deploy".into(),
                action: Box::new(Action::SyncDeploy(idx)),
            });
        }
        if let Some(task) = &f.task
            && crate::tasks::running_bar(ui, task, "Comparing…")
        {
            actions.push(Action::StopTask(idx));
        }
        if let Err(e) = &ready {
            ui.weak(e);
        }
    });
    if let Some(e) = &f.error {
        ui.colored_label(Tokens::error(), e);
    }
    if let Some(s) = &f.status {
        ui.colored_label(Tokens::success(), s);
    }
    ui.separator();
    if f.compared {
        if f.differences.is_empty() {
            ui.colored_label(Tokens::success(), format!("{} The structures are identical ({} objects).", crate::icons::CHECK, f.identical));
        } else {
            ui.weak(format!("{} difference(s), {} identical object(s)", f.differences.len(), f.identical));
            egui::ScrollArea::vertical().id_salt(("ss_list", idx)).max_height(220.0).show(ui, |ui| {
                egui::Grid::new(("ss_grid", idx)).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label("");
                    ui.strong("Object");
                    ui.strong("Type");
                    ui.strong("Action");
                    ui.end_row();
                    for (i, d) in f.differences.iter().enumerate() {
                        if let Some(c) = f.checked.get_mut(i) {
                            ui.checkbox(c, "");
                        }
                        if ui.selectable_label(f.selected == i, &d.object).clicked() {
                            f.selected = i;
                        }
                        ui.label(&d.kind);
                        let color = match d.action.as_str() {
                            "create" => Tokens::success(),
                            "drop" => Tokens::error(),
                            _ => Tokens::accent(),
                        };
                        ui.colored_label(color, d.action.to_uppercase());
                        ui.end_row();
                    }
                });
            });
            if let Some(d) = f.differences.get(f.selected) {
                ui.separator();
                ui.strong(format!("{} {}", d.action.to_uppercase(), d.object));
                for w in &d.warnings {
                    ui.colored_label(Tokens::accent(), w);
                }
                let mut sql = d.statements.join(";\n\n") + ";";
                egui::ScrollArea::vertical().id_salt(("ss_sql", idx)).max_height(220.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut sql).code_editor().desired_width(f32::INFINITY));
                });
            }
        }
    }
    if f.job.started() {
        ui.separator();
        crate::jobs::show_log(ui, &f.job, 140.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_need_both_sides_and_ticks() {
        let mut f = SyncForm::new(Some("a".into()), Some("db1".into()));
        assert!(f.params(false).is_err(), "no target database");
        f.target_db = Some("db1".into());
        assert!(f.params(false).is_err(), "same as source");
        f.target_db = Some("db2".into());
        assert!(f.params(false).is_ok());
        f.set_result(&json!({"differences": [{"object": "t", "type": "table", "action": "alter", "statements": ["x"]}, {"object": "old", "type": "table", "action": "drop", "statements": ["y"]}], "identical": ["u"]}));
        assert_eq!(f.checked, [true, false], "drops are opt-in");
        assert_eq!(f.params(true).unwrap()["objects"], json!(["t"]));
        f.checked = vec![false, false];
        assert!(f.params(true).is_err());
    }
}
