//! Objects tab: the contents of the database selected in the navigation pane, as a detail list
//! (Navicat's Objects tab), plus the information pane on the right.

use dbcraft_core::{DbKind, TableInfo};
use dbcraft_engine::Session;
use dbcraft_model::ObjectKind;
use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use serde_json::json;

use crate::app::Action;
use crate::icons;
use crate::state::{ObjectsMode, ObjectsView, Selection};
use crate::theme::{self, Tokens};

/// Optional columns, shown only when at least one row has a value (engines differ).
type Cell = fn(&TableInfo) -> Option<String>;

const OPTIONAL: &[(&str, Cell)] = &[
    ("Rows", |t| t.rows.map(|r| r.to_string())),
    ("Data Length", |t| t.data_length.map(theme::bytes)),
    ("Engine", |t| t.engine.clone()),
    ("Owner", |t| t.owner.clone()),
    ("Created Date", |t| t.created.clone()),
    ("Modified Date", |t| t.modified.clone()),
    ("Collation", |t| t.collation.clone()),
    ("Comment", |t| t.comment.clone()),
];

pub fn show(ui: &mut Ui, session: &Session, sel: Option<&Selection>, view: &mut ObjectsView, show_info: bool, actions: &mut Vec<Action>) {
    let Some(sel) = sel else {
        ui.centered_and_justified(|ui| ui.weak("Select a connection or database in the navigation pane."));
        return;
    };
    match view.mode {
        ObjectsMode::Queries => return crate::objects_modes::queries(ui, session, sel, view, actions),
        ObjectsMode::Backups => return crate::objects_modes::backups(ui, sel, view, actions),
        ObjectsMode::Users => return crate::objects_modes::users(ui, sel, view, actions),
        ObjectsMode::Objects => {}
    }
    let Some(db) = &sel.database else {
        connection_summary(ui, session, sel);
        return;
    };
    let Some(cfg) = session.workspace.connection(&sel.connection).ok() else { return };
    let key = (sel.connection.clone(), db.clone());
    let table_like = matches!(sel.group, ObjectKind::Table | ObjectKind::View);
    let rows: Vec<&TableInfo> = session
        .tables
        .get(&key)
        .map(|v| {
            v.iter()
                .filter(|t| (sel.group == ObjectKind::View) == (t.kind == "view"))
                .filter(|t| view.search.is_empty() || t.name.to_lowercase().contains(&view.search.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();
    let selected_info = view.selected.as_ref().and_then(|(s, n)| rows.iter().find(|t| &t.name == n && &t.schema == s).copied());

    // Sub-toolbar.
    let group_word = if sel.group == ObjectKind::View { "View" } else { "Table" };
    egui::Frame::new().fill(Tokens::toolbar_bg()).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let can_open = selected_info.is_some();
            if ui.add_enabled(can_open, egui::Button::new(format!("{} Open {group_word}", icons::TABLE))).clicked()
                && let Some(t) = selected_info
            {
                actions.push(Action::OpenTable { connection: sel.connection.clone(), database: db.clone(), schema: t.schema.clone(), table: t.name.clone() });
            }
            let tables = sel.group != ObjectKind::View;
            let picked = selected_info.filter(|_| tables).map(|t| crate::table_menu::TableRef {
                connection: &sel.connection,
                database: db,
                schema: t.schema.as_deref(),
                name: &t.name,
            });
            if ui.add_enabled(picked.is_some(), egui::Button::new(format!("{} Design {group_word}", icons::PENCIL_SIMPLE))).clicked()
                && let Some(t) = &picked
            {
                actions.push(crate::table_menu::design_action(t));
            }
            if ui.add_enabled(tables, egui::Button::new(format!("{} New {group_word}", icons::PLUS))).clicked() {
                let schema = if cfg.kind == DbKind::Postgres { Some("public") } else { None };
                actions.push(crate::table_menu::new_table_action(&sel.connection, db, schema));
            }
            if ui.add_enabled(picked.is_some(), egui::Button::new(format!("{} Delete {group_word}", icons::TRASH))).clicked()
                && let Some(t) = &picked
            {
                actions.push(crate::table_menu::delete_action(t));
            }
            ui.separator();
            ui.add_enabled(false, egui::Button::new(format!("{} Import Wizard", icons::DOWNLOAD_SIMPLE))).on_disabled_hover_text("Coming soon (ROADMAP.md)");
            ui.add_enabled(false, egui::Button::new(format!("{} Export Wizard", icons::UPLOAD_SIMPLE))).on_disabled_hover_text("Coming soon (ROADMAP.md)");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(icons::ARROWS_CLOCKWISE).on_hover_text("Refresh").clicked() {
                    actions.push(Action::Command("database.tables", json!({ "connection": sel.connection, "database": db })));
                }
                ui.add(egui::TextEdit::singleline(&mut view.search).hint_text(format!("{} Search", icons::MAGNIFYING_GLASS)).desired_width(180.0));
            });
        });
    });

    if !table_like {
        let objs: Vec<_> = session.objects.get(&key).into_iter().flatten().filter(|o| o.kind == sel.group).collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for o in objs {
                ui.horizontal(|ui| {
                    ui.label(icons::object(o.kind));
                    ui.label(&o.name);
                });
            }
        });
        return;
    }

    if !session.tables.contains_key(&key) {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.spinner());
        return;
    }

    if show_info {
        egui::Panel::right("objects_info").resizable(true).default_size(320.0).show(ui, |ui| info_pane(ui, cfg.kind, selected_info, view, sel, db, actions));
    }

    let cols: Vec<_> = OPTIONAL.iter().filter(|(_, f)| rows.iter().any(|t| f(t).is_some())).collect();
    let mut open = None;
    let mut select = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .column(Column::initial(200.0).at_least(80.0).clip(true))
            .columns(Column::initial(110.0).at_least(50.0).clip(true), cols.len())
            .header(22.0, |mut h| {
                h.col(|ui| {
                    ui.strong("Name");
                });
                for (name, _) in &cols {
                    h.col(|ui| {
                        ui.strong(*name);
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, rows.len(), |mut row| {
                    let Some(t) = rows.get(row.index()).copied() else { return };
                    let is_sel = view.selected.as_ref().is_some_and(|(s, n)| n == &t.name && s == &t.schema);
                    row.set_selected(is_sel);
                    row.col(|ui| {
                        let name = match (&t.schema, cfg.kind) {
                            (Some(s), DbKind::Postgres) if s != "public" => format!("{s}.{}", t.name),
                            _ => t.name.clone(),
                        };
                        ui.horizontal(|ui| {
                            ui.label(icons::object(if t.kind == "view" { ObjectKind::View } else { ObjectKind::Table }));
                            ui.label(name);
                        });
                    });
                    for (_, f) in &cols {
                        row.col(|ui| {
                            ui.label(f(t).unwrap_or_default());
                        });
                    }
                    let r = row.response();
                    if r.clicked() {
                        select = Some(t);
                    }
                    if r.double_clicked() {
                        open = Some(t);
                    }
                    r.context_menu(|ui| {
                        if t.kind == "view" {
                            if ui.button(format!("{} Open {group_word}", icons::TABLE)).clicked() {
                                open = Some(t);
                                ui.close();
                            }
                            if ui.button(format!("{} Copy Name", icons::CODE)).clicked() {
                                ui.ctx().copy_text(t.name.clone());
                                ui.close();
                            }
                        } else {
                            let tr = crate::table_menu::TableRef { connection: &sel.connection, database: db, schema: t.schema.as_deref(), name: &t.name };
                            crate::table_menu::items(ui, &tr, actions);
                        }
                    });
                });
            });
    });
    ui.weak(format!("{} {}", rows.len(), if sel.group == ObjectKind::View { "views" } else { "tables" }));

    if let Some(t) = select {
        let new = Some((t.schema.clone(), t.name.clone()));
        if view.selected != new {
            view.selected = new;
            view.ddl = None;
        }
    }
    if let Some(t) = open {
        actions.push(Action::OpenTable { connection: sel.connection.clone(), database: db.clone(), schema: t.schema.clone(), table: t.name.clone() });
    }
}

fn info_pane(ui: &mut Ui, kind: DbKind, t: Option<&TableInfo>, view: &mut ObjectsView, sel: &Selection, db: &str, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.add(egui::Button::selectable(!view.info_ddl, format!("{} General", icons::INFO))).clicked() {
            view.info_ddl = false;
        }
        if ui.add(egui::Button::selectable(view.info_ddl, format!("{} DDL", icons::CODE))).clicked() {
            view.info_ddl = true;
        }
    });
    ui.separator();
    let Some(t) = t else {
        ui.weak("Select an object to see its details.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(icons::object(if t.kind == "view" { ObjectKind::View } else { ObjectKind::Table }).size(22.0));
        ui.heading(&t.name);
    });
    ui.add_space(4.0);
    if view.info_ddl {
        match &view.ddl {
            Some(ddl) => {
                egui::ScrollArea::both().show(ui, |ui| {
                    let mut text = ddl.as_str();
                    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
                });
            }
            None => {
                actions.push(Action::LoadDdl { connection: sel.connection.clone(), database: db.to_string(), schema: t.schema.clone(), table: t.name.clone() });
                ui.spinner();
            }
        }
        return;
    }
    egui::Grid::new("info_general").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        let mut row = |k: &str, v: Option<String>| {
            if let Some(v) = v {
                ui.label(RichText::new(k).color(Tokens::text_weak()));
                ui.label(v);
                ui.end_row();
            }
        };
        row("Type", Some(if t.kind == "view" { "View".into() } else { "Table".into() }));
        row("Schema", t.schema.clone().filter(|_| kind == DbKind::Postgres));
        row("Rows", t.rows.map(|r| r.to_string()));
        row("Data Length", t.data_length.map(theme::bytes));
        row("Engine", t.engine.clone());
        row("Owner", t.owner.clone());
        row("Created Date", t.created.clone());
        row("Modified Date", t.modified.clone());
        row("Collation", t.collation.clone());
        row("Comment", t.comment.clone());
    });
}

fn connection_summary(ui: &mut Ui, session: &Session, sel: &Selection) {
    let Ok(cfg) = session.workspace.connection(&sel.connection) else { return };
    let open = session.is_open(&cfg.id);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.label(icons::connection(cfg.kind, open).size(28.0));
        ui.vertical(|ui| {
            ui.heading(&cfg.name);
            ui.weak(if open { "Connected" } else { "Not connected" });
        });
    });
    ui.add_space(8.0);
    egui::Grid::new("conn_summary").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        let mut row = |k: &str, v: String| {
            if !v.is_empty() {
                ui.label(RichText::new(k).color(Tokens::text_weak()));
                ui.label(v);
                ui.end_row();
            }
        };
        row("Type", cfg.kind.label().into());
        if cfg.kind == DbKind::Sqlite {
            row("File", cfg.database.clone());
        } else {
            row("Host", cfg.host.clone());
            row("Port", cfg.port.map(|p| p.to_string()).unwrap_or_default());
            row("User", cfg.user.clone());
        }
        if let Some(ssh) = &cfg.ssh {
            row("SSH", format!("{}@{}:{}", ssh.user, ssh.host, ssh.port));
        }
        if let Some(dbs) = session.databases.get(&cfg.id) {
            row("Databases", dbs.len().to_string());
        }
    });
}
