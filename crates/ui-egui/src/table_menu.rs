//! The table context menu (navigation tree and Objects tab): Open, Design, New, Delete, Empty,
//! Truncate, Duplicate, Rename. Destructive actions ask first.

use egui::Ui;
use serde_json::json;

use crate::app::Action;
use crate::icons;

/// Which table the menu is for.
pub struct TableRef<'a> {
    pub connection: &'a str,
    pub database: &'a str,
    pub schema: Option<&'a str>,
    pub name: &'a str,
}

impl TableRef<'_> {
    fn params(&self) -> serde_json::Value {
        json!({ "connection": self.connection, "database": self.database, "schema": self.schema, "table": self.name })
    }
}

/// Delete Table, after confirmation.
pub fn delete_action(t: &TableRef) -> Action {
    let params = json!({ "connection": t.connection, "database": t.database, "schema": t.schema, "tables": [t.name] });
    Action::Confirm {
        title: "Delete Table".into(),
        message: format!("Do you really want to delete the table `{}`? Its data is lost.", t.name),
        button: "Delete".into(),
        action: Box::new(Action::TableOp { command: "table.delete", params }),
    }
}

pub fn new_table_action(connection: &str, database: &str, schema: Option<&str>) -> Action {
    Action::OpenDesigner { connection: connection.into(), database: database.into(), schema: schema.map(str::to_string), table: None }
}

pub fn design_action(t: &TableRef) -> Action {
    Action::OpenDesigner { connection: t.connection.into(), database: t.database.into(), schema: t.schema.map(str::to_string), table: Some(t.name.into()) }
}

/// The menu items; call inside `Response::context_menu`.
pub fn items(ui: &mut Ui, t: &TableRef, actions: &mut Vec<Action>) {
    let mut push = |ui: &mut Ui, a: Action| {
        actions.push(a);
        ui.close();
    };
    if ui.button(format!("{} Open Table", icons::TABLE)).clicked() {
        push(
            ui,
            Action::OpenTable { connection: t.connection.into(), database: t.database.into(), schema: t.schema.map(str::to_string), table: t.name.into() },
        );
    }
    if ui.button(format!("{} Design Table", icons::PENCIL_SIMPLE)).clicked() {
        push(ui, design_action(t));
    }
    if ui.button(format!("{} New Table", icons::PLUS)).clicked() {
        push(ui, new_table_action(t.connection, t.database, t.schema));
    }
    ui.separator();
    if ui.button(format!("{} Delete Table", icons::TRASH)).clicked() {
        push(ui, delete_action(t));
    }
    if ui.button("Empty Table").clicked() {
        push(
            ui,
            Action::Confirm {
                title: "Empty Table".into(),
                message: format!("Delete all records of `{}`? (DELETE: triggers fire, auto-increment is kept)", t.name),
                button: "Empty".into(),
                action: Box::new(Action::TableOp { command: "table.empty", params: t.params() }),
            },
        );
    }
    if ui.button("Truncate Table").clicked() {
        push(
            ui,
            Action::Confirm {
                title: "Truncate Table".into(),
                message: format!("Truncate `{}`? All records are removed and auto-increment restarts.", t.name),
                button: "Truncate".into(),
                action: Box::new(Action::TableOp { command: "table.truncate", params: t.params() }),
            },
        );
    }
    if ui.button("Duplicate Table").clicked() {
        push(ui, Action::TableOp { command: "table.duplicate", params: t.params() });
    }
    if ui.button("Rename").clicked() {
        push(
            ui,
            Action::RenameTable { connection: t.connection.into(), database: t.database.into(), schema: t.schema.map(str::to_string), table: t.name.into() },
        );
    }
    ui.separator();
    if ui.button(format!("{} Copy Name", icons::CODE)).clicked() {
        ui.ctx().copy_text(t.name.to_string());
        ui.close();
    }
}
