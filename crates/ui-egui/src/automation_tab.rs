//! Tools › Automation: batch jobs (Backup, Query, Data Transfer steps), Start now, and a
//! Windows Task Scheduler schedule that runs `dbcraft-cli batchjob <id>`.

use dbcraft_engine::Session;
use dbcraft_model::{BatchJob, JobStep};
use egui::{RichText, Ui};
use egui_phosphor::regular as ph;
use serde_json::json;

use crate::app::Action;
use crate::icons;
use crate::state::AutomationView;
use crate::theme::Tokens;

#[derive(Debug, Clone)]
pub enum AutoAct {
    New,
    Edit(String),
    Save,
    Delete(String),
    Run,
    Schedule,
    Unschedule,
}

impl AutomationView {
    pub fn new() -> Self {
        AutomationView { frequency: "daily".into(), time: "02:00".into(), day: "MON".into(), step_kind: "backup".into(), ..AutomationView::default() }
    }

    /// The step the "Add" pickers describe.
    pub fn new_step(&self, session: &Session) -> Result<JobStep, String> {
        let conn = self.step_connection.clone().ok_or("Choose a connection")?;
        let conn_name = session.workspace.connection(&conn).map(|c| c.name.clone()).unwrap_or_else(|_| conn.clone());
        match self.step_kind.as_str() {
            "backup" => {
                let db = self.step_database.clone().ok_or("Choose a database")?;
                Ok(JobStep {
                    command: "backup.create".into(),
                    params: json!({"connection": conn, "database": db}),
                    label: format!("Backup {db} on {conn_name}"),
                })
            }
            "query" => {
                if self.step_sql.trim().is_empty() {
                    return Err("Enter the SQL to run".into());
                }
                let first: String = self.step_sql.trim().chars().take(40).collect();
                Ok(JobStep {
                    command: "query.run".into(),
                    params: json!({"connection": conn, "database": self.step_database, "sql": self.step_sql}),
                    label: format!("Query on {conn_name}: {first}"),
                })
            }
            other => Err(format!("unknown step type {other}")),
        }
    }
}

pub fn show(ui: &mut Ui, idx: usize, view: &mut AutomationView, session: &Session, actions: &mut Vec<Action>) {
    egui::Panel::left(egui::Id::new(("auto_list", idx))).resizable(true).default_size(240.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong(format!("{} Batch Jobs", ph::ROBOT));
            // Left-aligned: a right-aligned button would sit under the panel's resize handle.
            if ui.button(format!("{} New", icons::PLUS)).clicked() {
                actions.push(Action::Automation(idx, AutoAct::New));
            }
        });
        ui.separator();
        for j in &session.workspace.automations {
            let selected = view.editing.as_ref().is_some_and(|e| e.id == j.id);
            let mut text = format!("{}  {}", ph::ROBOT, j.name);
            if j.schedule.is_some() {
                text.push_str(&format!("  {}", ph::CLOCK));
            }
            let r = ui.selectable_label(selected, text);
            if r.clicked() {
                actions.push(Action::Automation(idx, AutoAct::Edit(j.id.clone())));
            }
            r.context_menu(|ui| {
                if ui.button(format!("{} Delete", icons::TRASH)).clicked() {
                    actions.push(Action::Automation(idx, AutoAct::Delete(j.id.clone())));
                    ui.close();
                }
            });
            if let Some(last) = &j.last_run {
                ui.weak(format!("    last run: {last}"));
            }
        }
        if session.workspace.automations.is_empty() {
            ui.weak("No batch jobs yet. Click New.");
        }
    });

    // Take the job out while drawing so the closures below can also borrow `view`.
    let Some(mut job_owned) = view.editing.take() else {
        ui.centered_and_justified(|ui| ui.weak("Select a batch job, or create one with New."));
        return;
    };
    let job = &mut job_owned;
    let saved = session.workspace.automations.iter().find(|j| j.id == job.id).cloned();
    egui::ScrollArea::vertical().id_salt(("auto_edit", idx)).show(ui, |ui| {
        ui.add_space(6.0);
        egui::Grid::new(("auto_head", idx)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut job.name).desired_width(300.0));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut job.continue_on_error, "Continue on error");
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong("Selected Jobs");
            let mut remove = None;
            let mut swap = None;
            let n = job.steps.len();
            for (i, st) in job.steps.iter().enumerate() {
                ui.horizontal(|ui| {
                    let icon = match st.command.as_str() {
                        "backup.create" => ph::ARCHIVE,
                        "query.run" => icons::FILE_SQL,
                        "tools.dataTransfer" => ph::ARROWS_LEFT_RIGHT,
                        _ => ph::GEAR,
                    };
                    ui.label(format!("{}. {icon}  {}", i + 1, if st.label.is_empty() { &st.command } else { &st.label }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(icons::TRASH).on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new(ph::ARROW_DOWN).small()).clicked() {
                            swap = Some((i, i + 1));
                        }
                        if ui.add_enabled(i > 0, egui::Button::new(ph::ARROW_UP).small()).clicked() {
                            swap = Some((i - 1, i));
                        }
                    });
                });
            }
            if let Some(i) = remove {
                job.steps.remove(i);
            }
            if let Some((a, b)) = swap {
                job.steps.swap(a, b);
            }
            if job.steps.is_empty() {
                ui.weak("No steps yet: add a Backup or a Query below (Data Transfer: use Save as Batch Job in its tab).");
            }
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong("Add Job");
            ui.horizontal(|ui| {
                ui.radio_value(&mut view.step_kind, "backup".to_string(), format!("{} Backup", ph::ARCHIVE));
                ui.radio_value(&mut view.step_kind, "query".to_string(), format!("{} Query", icons::FILE_SQL));
            });
            ui.horizontal(|ui| {
                let name = view
                    .step_connection
                    .as_ref()
                    .and_then(|c| session.workspace.connection(c).ok())
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "(connection)".into());
                egui::ComboBox::from_id_salt(("auto_conn", idx)).width(180.0).selected_text(name).show_ui(ui, |ui| {
                    for c in &session.workspace.connections {
                        ui.selectable_value(&mut view.step_connection, Some(c.id.clone()), &c.name);
                    }
                });
                let dbs = view.step_connection.as_ref().and_then(|c| session.databases.get(c)).cloned().unwrap_or_default();
                egui::ComboBox::from_id_salt(("auto_db", idx))
                    .width(160.0)
                    .selected_text(view.step_database.clone().unwrap_or_else(|| "(database)".into()))
                    .show_ui(ui, |ui| {
                        for d in &dbs {
                            ui.selectable_value(&mut view.step_database, Some(d.clone()), d);
                        }
                    });
                if dbs.is_empty() && view.step_connection.is_some() {
                    ui.weak("open the connection to list databases");
                }
            });
            if view.step_kind == "query" {
                ui.add(egui::TextEdit::multiline(&mut view.step_sql).code_editor().desired_rows(3).desired_width(f32::INFINITY).hint_text("SQL to run"));
            }
            let step = view.new_step(session);
            ui.horizontal(|ui| {
                if ui.add_enabled(step.is_ok(), egui::Button::new(format!("{} Add", icons::PLUS))).clicked()
                    && let Ok(st) = step.clone()
                {
                    job.steps.push(st);
                }
                if let Err(why) = &step {
                    ui.weak(why);
                }
            });
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong(format!("{} Schedule", ph::CLOCK));
            match saved.as_ref().and_then(|j| j.schedule.as_ref()) {
                Some(s) => ui.label(format!(
                    "Runs {} at {}{}  (Windows Task Scheduler)",
                    s.frequency,
                    s.time,
                    s.day.as_ref().map(|d| format!(" on {d}")).unwrap_or_default()
                )),
                None => ui.weak("Not scheduled."),
            };
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("auto_freq", idx)).width(90.0).selected_text(&view.frequency).show_ui(ui, |ui| {
                    for f in ["once", "hourly", "daily", "weekly"] {
                        ui.selectable_value(&mut view.frequency, f.to_string(), f);
                    }
                });
                ui.label("at");
                ui.add(egui::TextEdit::singleline(&mut view.time).desired_width(50.0).hint_text("HH:MM"));
                if view.frequency == "weekly" {
                    egui::ComboBox::from_id_salt(("auto_day", idx)).width(60.0).selected_text(&view.day).show_ui(ui, |ui| {
                        for d in ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"] {
                            ui.selectable_value(&mut view.day, d.to_string(), d);
                        }
                    });
                }
                let is_saved = saved.is_some();
                if ui.add_enabled(is_saved, egui::Button::new("Set Task Schedule")).on_disabled_hover_text("Save the job first").clicked() {
                    actions.push(Action::Automation(idx, AutoAct::Schedule));
                }
                if ui.add_enabled(saved.as_ref().is_some_and(|j| j.schedule.is_some()), egui::Button::new("Delete Task Schedule")).clicked() {
                    actions.push(Action::Automation(idx, AutoAct::Unschedule));
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let changed = saved.as_ref() != Some(&*job);
            if ui.add(egui::Button::new(RichText::new(format!("{} Save", ph::FLOPPY_DISK)).strong())).clicked() {
                actions.push(Action::Automation(idx, AutoAct::Save));
            }
            let can_run = !job.steps.is_empty() && !view.job.running();
            if ui.add_enabled(can_run, egui::Button::new(RichText::new(format!("{} Start", icons::PLAY)).color(Tokens::SUCCESS))).clicked() {
                actions.push(Action::Automation(idx, AutoAct::Run));
            }
            if changed {
                ui.weak("● unsaved changes");
            }
            if let Some(s) = &view.status {
                ui.colored_label(Tokens::ACCENT, s);
            }
        });
        ui.add_space(6.0);
        crate::jobs::show_log(ui, &view.job, 220.0);
    });
    view.editing = Some(job_owned);
}

/// A new, unsaved job.
pub fn blank_job(session: &Session) -> BatchJob {
    let id = dbcraft_model::next_id("job", session.workspace.automations.iter().map(|j| j.id.as_str()));
    BatchJob { id, name: "New Batch Job".into(), steps: Vec::new(), continue_on_error: true, schedule: None, last_run: None }
}
