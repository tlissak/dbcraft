//! Objects tab content for the Query, Backup and User toolbar buttons (Navicat lists saved
//! queries, backups or users in the Objects tab).

use dbcraft_engine::Session;
use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use egui_phosphor::regular as ph;

use crate::app::Action;
use crate::icons;
use crate::state::{ObjectsView, Selection};
use crate::theme::{self, Tokens};

fn toolbar(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(Tokens::toolbar_bg()).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(add);
    });
}

fn fmt_time(secs: u64) -> String {
    chrono_like(secs)
}

/// `YYYY-MM-DD HH:MM` (UTC) without pulling a date crate into the UI.
fn chrono_like(secs: u64) -> String {
    let days = secs / 86_400;
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}")
}

pub fn queries(ui: &mut Ui, session: &Session, sel: &Selection, view: &mut ObjectsView, actions: &mut Vec<Action>) {
    toolbar(ui, |ui| {
        if ui.button(format!("{} New Query", icons::FILE_SQL)).clicked() {
            actions.push(Action::NewQuery { connection: Some(sel.connection.clone()), database: sel.database.clone() });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::TextEdit::singleline(&mut view.search).hint_text(format!("{} Search", icons::MAGNIFYING_GLASS)).desired_width(180.0));
        });
    });
    let list: Vec<_> = session
        .workspace
        .queries
        .iter()
        .filter(|q| q.connection == sel.connection && (sel.database.is_none() || q.database == sel.database))
        .filter(|q| view.search.is_empty() || q.name.to_lowercase().contains(&view.search.to_lowercase()))
        .collect();
    if list.is_empty() {
        ui.add_space(12.0);
        ui.weak("No saved queries here. Write one in a query tab and press Ctrl+S.");
        return;
    }
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .column(Column::initial(260.0))
        .column(Column::initial(140.0))
        .column(Column::remainder())
        .header(22.0, |mut h| {
            for t in ["Name", "Modified", "SQL"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(22.0, list.len(), |mut row| {
                let Some(q) = list.get(row.index()) else { return };
                row.col(|ui| {
                    ui.label(format!("{}  {}", icons::FILE_SQL, q.name));
                });
                row.col(|ui| {
                    ui.label(fmt_time(q.modified));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(q.sql.replace('\n', " ")).color(Tokens::text_weak())).truncate());
                });
                let r = row.response();
                if r.double_clicked() {
                    actions.push(Action::OpenSavedQuery(q.id.clone()));
                }
                r.context_menu(|ui| {
                    if ui.button("Open Query").clicked() {
                        actions.push(Action::OpenSavedQuery(q.id.clone()));
                        ui.close();
                    }
                    if ui.button(format!("{} Delete Query", icons::TRASH)).clicked() {
                        actions.push(Action::Command("query.delete", serde_json::json!({ "id": q.id })));
                        ui.close();
                    }
                });
            });
        });
}

pub fn backups(ui: &mut Ui, sel: &Selection, view: &mut ObjectsView, actions: &mut Vec<Action>) {
    let Some(db) = &sel.database else {
        ui.add_space(12.0);
        ui.weak("Select a database to see its backups.");
        return;
    };
    let running = view.job.running();
    toolbar(ui, |ui| {
        ui.add(egui::TextEdit::singleline(&mut view.backup_comment).hint_text("Comment (optional)").desired_width(200.0));
        if ui.add_enabled(!running, egui::Button::new(format!("{} New Backup", ph::ARCHIVE))).clicked() {
            actions.push(Action::BackupCreate);
        }
        let chosen = view.selected_backup.clone();
        if ui.add_enabled(chosen.is_some() && !running, egui::Button::new(format!("{} Restore Backup", ph::CLOCK_COUNTER_CLOCKWISE))).clicked() {
            view.confirm_restore = chosen.clone();
        }
        if ui.add_enabled(chosen.is_some() && !running, egui::Button::new(format!("{} Delete Backup", icons::TRASH))).clicked() {
            view.confirm_delete_backup = chosen.clone();
        }
        if ui.button(format!("{} Open Folder", ph::FOLDER_OPEN)).clicked() {
            actions.push(Action::BackupOpenFolder);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(icons::ARROWS_CLOCKWISE).on_hover_text("Refresh").clicked() {
                actions.push(Action::LoadBackups);
            }
        });
    });
    let Some(list) = &view.backups else {
        actions.push(Action::LoadBackups);
        ui.spinner();
        return;
    };
    if list.is_empty() {
        ui.add_space(12.0);
        ui.weak(format!("No backups of {db} yet. Click New Backup."));
    } else {
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .column(Column::initial(200.0))
            .column(Column::initial(150.0))
            .column(Column::initial(80.0))
            .column(Column::initial(80.0))
            .column(Column::initial(90.0))
            .column(Column::remainder())
            .header(22.0, |mut h| {
                for t in ["Backup", "Created", "Tables", "Rows", "Size", "Comment"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, list.len(), |mut row| {
                    let Some(b) = list.get(row.index()) else { return };
                    row.set_selected(view.selected_backup.as_deref() == Some(b.path.as_str()));
                    row.col(|ui| {
                        ui.label(format!("{}  {}", ph::ARCHIVE, b.file.trim_end_matches(".sql")));
                    });
                    row.col(|ui| {
                        ui.label(&b.created);
                    });
                    row.col(|ui| {
                        ui.label(b.tables.to_string());
                    });
                    row.col(|ui| {
                        ui.label(b.rows.to_string());
                    });
                    row.col(|ui| {
                        ui.label(theme::bytes(b.size));
                    });
                    row.col(|ui| {
                        ui.label(&b.comment);
                    });
                    let r = row.response();
                    if r.clicked() {
                        view.selected_backup = Some(b.path.clone());
                    }
                    if r.double_clicked() {
                        view.confirm_restore = Some(b.path.clone());
                    }
                });
            });
    }
    ui.add_space(6.0);
    crate::jobs::show_log(ui, &view.job, 160.0);

    if let Some(path) = view.confirm_delete_backup.clone() {
        let mut open = true;
        egui::Window::new("Delete Backup")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .show(ui.ctx(), |ui| {
                ui.label("Delete this backup file? This cannot be undone.");
                ui.weak(&path);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Delete").color(Tokens::error())).clicked() {
                        actions.push(Action::BackupDelete(path.clone()));
                        view.confirm_delete_backup = None;
                    }
                    if ui.button("Cancel").clicked() {
                        view.confirm_delete_backup = None;
                    }
                });
            });
        if !open {
            view.confirm_delete_backup = None;
        }
    }

    if let Some(path) = view.confirm_restore.clone() {
        let mut open = true;
        egui::Window::new("Restore Backup")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .show(ui.ctx(), |ui| {
                ui.label(format!("Restore this backup into `{db}`?"));
                ui.weak(&path);
                ui.colored_label(Tokens::error(), "Tables in the backup are dropped and recreated: their current data is replaced.");
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Restore").strong()).clicked() {
                        actions.push(Action::BackupRestore(path.clone()));
                        view.confirm_restore = None;
                    }
                    if ui.button("Cancel").clicked() {
                        view.confirm_restore = None;
                    }
                });
            });
        if !open {
            view.confirm_restore = None;
        }
    }
}

pub fn users(ui: &mut Ui, sel: &Selection, view: &mut ObjectsView, actions: &mut Vec<Action>) {
    toolbar(ui, |ui| {
        if ui.button(format!("{} New User", ph::USER_PLUS)).clicked() {
            actions.push(Action::NewUser { connection: sel.connection.clone(), role: false });
        }
        if ui.button(format!("{} New Role", ph::USERS_THREE)).clicked() {
            actions.push(Action::NewUser { connection: sel.connection.clone(), role: true });
        }
        let chosen = view.selected_user.clone();
        if ui.add_enabled(chosen.is_some(), egui::Button::new(format!("{} Edit", icons::PENCIL_SIMPLE))).clicked()
            && let Some((name, host)) = chosen.clone()
        {
            actions.push(Action::OpenUser { connection: sel.connection.clone(), name, host });
        }
        if ui.add_enabled(chosen.is_some(), egui::Button::new(format!("{} Delete", icons::TRASH))).clicked() {
            view.confirm_delete_user = chosen.clone();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(icons::ARROWS_CLOCKWISE).on_hover_text("Refresh").clicked() {
                actions.push(Action::LoadUsers);
            }
            ui.add(egui::TextEdit::singleline(&mut view.search).hint_text(format!("{} Search", icons::MAGNIFYING_GLASS)).desired_width(160.0));
        });
    });
    let list = match &view.users {
        None => {
            actions.push(Action::LoadUsers);
            ui.spinner();
            return;
        }
        Some(Err(e)) => {
            ui.colored_label(Tokens::error(), e);
            if e.contains("denied") || e.contains("permission") {
                ui.weak("Managing users needs an administrator account (for example root on MySQL/MariaDB, or a superuser on PostgreSQL). Edit the connection to use one.");
            }
            return;
        }
        Some(Ok(l)) => l.iter().filter(|u| view.search.is_empty() || u.name.to_lowercase().contains(&view.search.to_lowercase())).cloned().collect::<Vec<_>>(),
    };
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .column(Column::initial(220.0))
        .column(Column::initial(120.0))
        .column(Column::initial(70.0))
        .column(Column::remainder())
        .header(22.0, |mut h| {
            for t in ["Name", "Host", "Type", "Details"] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|body| {
            body.rows(22.0, list.len(), |mut row| {
                let Some(u) = list.get(row.index()) else { return };
                let key = (u.name.clone(), u.host.clone());
                row.set_selected(view.selected_user.as_ref() == Some(&key));
                row.col(|ui| {
                    let icon = if u.is_role { ph::USERS_THREE } else { ph::USER };
                    ui.label(format!("{icon}  {}", u.name));
                });
                row.col(|ui| {
                    ui.label(u.host.clone().unwrap_or_default());
                });
                row.col(|ui| {
                    ui.label(if u.is_role { "Role" } else { "User" });
                });
                row.col(|ui| {
                    let mut parts: Vec<String> = u.attributes.iter().filter(|(_, on)| **on).map(|(k, _)| k.clone()).collect();
                    parts.extend(u.settings.iter().map(|(k, v)| format!("{k}={v}")));
                    if !u.member_of.is_empty() {
                        parts.push(format!("member of {}", u.member_of.join(", ")));
                    }
                    ui.add(egui::Label::new(RichText::new(parts.join(" · ")).color(Tokens::text_weak())).truncate());
                });
                let r = row.response();
                if r.clicked() {
                    view.selected_user = Some(key.clone());
                }
                if r.double_clicked() {
                    actions.push(Action::OpenUser { connection: sel.connection.clone(), name: key.0.clone(), host: key.1.clone() });
                }
            });
        });

    if let Some((name, host)) = view.confirm_delete_user.clone() {
        let mut open = true;
        egui::Window::new("Delete User")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .show(ui.ctx(), |ui| {
                ui.label(format!("Delete `{}`{}? This cannot be undone.", name, host.as_ref().map(|h| format!("@{h}")).unwrap_or_default()));
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Delete").color(Tokens::error())).clicked() {
                        actions.push(Action::DeleteUser { connection: sel.connection.clone(), name: name.clone(), host: host.clone() });
                        view.confirm_delete_user = None;
                    }
                    if ui.button("Cancel").clicked() {
                        view.confirm_delete_user = None;
                    }
                });
            });
        if !open {
            view.confirm_delete_user = None;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_unix_time() {
        assert_eq!(super::chrono_like(0), "1970-01-01 00:00");
        assert_eq!(super::chrono_like(1_791_965_552), "2026-10-14 08:12");
    }
}
