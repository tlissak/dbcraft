//! Table Designer (Design Table / New Table): Fields with their properties, Indexes, Foreign
//! Keys, Uniques, Checks, Triggers, Options, Comment and SQL Preview, per engine like Navicat.
//! Editing changes only this tab's copy; Save (Ctrl+S) runs `table.save` with exactly the
//! statements SQL Preview shows.

use dbcraft_core::{CheckInfo, ColumnInfo, ForeignKeyInfo, GenericType, IndexInfo, TableSchema, TriggerInfo, UniqueInfo};
use dbcraft_engine::alter::Plan;
use dbcraft_sql::Dialect;
use egui::{RichText, Ui};
use egui_phosphor::regular as ph;
use serde::{Deserialize, Serialize};

use crate::app::Action;
use crate::icons;
use crate::tasks::Task;
use crate::theme::Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DesignerPage {
    #[default]
    Fields,
    Indexes,
    ForeignKeys,
    Uniques,
    Checks,
    Triggers,
    Options,
    Comment,
    SqlPreview,
}

impl DesignerPage {
    fn label(self) -> &'static str {
        match self {
            DesignerPage::Fields => "Fields",
            DesignerPage::Indexes => "Indexes",
            DesignerPage::ForeignKeys => "Foreign Keys",
            DesignerPage::Uniques => "Uniques",
            DesignerPage::Checks => "Checks",
            DesignerPage::Triggers => "Triggers",
            DesignerPage::Options => "Options",
            DesignerPage::Comment => "Comment",
            DesignerPage::SqlPreview => "SQL Preview",
        }
    }
}

/// The pages an engine has (Navicat shows only what the engine supports).
pub fn pages(d: Dialect) -> Vec<DesignerPage> {
    use DesignerPage::*;
    match d {
        Dialect::Mysql => vec![Fields, Indexes, ForeignKeys, Checks, Triggers, Options, Comment, SqlPreview],
        Dialect::Sqlite => vec![Fields, Indexes, ForeignKeys, Uniques, Checks, Triggers, Options, SqlPreview],
        _ => vec![Fields, Indexes, ForeignKeys, Uniques, Checks, Triggers, Options, Comment, SqlPreview],
    }
}

/// One designer tab.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DesignerTab {
    pub connection: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    /// The saved definition (`None`: a new table).
    pub original: Option<TableSchema>,
    /// The edited definition.
    pub table: TableSchema,
    /// The saved name of each edited field (`None`: added here).
    pub origins: Vec<Option<String>>,
    pub page: DesignerPage,
    /// Selected row on the current page.
    pub selected: usize,
    pub loaded: bool,
    pub error: Option<String>,
    pub status: Option<String>,
    /// SQL Preview, and the definition it was computed for.
    pub preview: Option<Result<Plan, String>>,
    pub preview_for: Option<TableSchema>,
    /// "Table Name" prompt for a new table's first save.
    pub name_prompt: Option<String>,
    pub close_after_save: bool,
    /// Loading or saving.
    #[serde(skip)]
    pub task: Option<Task>,
}

impl DesignerTab {
    /// Designer for an existing table (filled when `table.schema` returns).
    pub fn design(connection: String, database: Option<String>, schema: Option<String>, table: &str) -> Self {
        DesignerTab { connection, database, schema, table: TableSchema { name: table.to_string(), ..TableSchema::default() }, ..DesignerTab::default() }
    }

    /// Designer for a new table, with one starter field.
    pub fn new_table(connection: String, database: Option<String>, schema: Option<String>, d: Dialect) -> Self {
        let (ty, generic) = match d {
            Dialect::Mysql => ("int", GenericType::Integer),
            Dialect::Postgres => ("integer", GenericType::Integer),
            _ => ("INTEGER", GenericType::Integer),
        };
        let table = TableSchema { schema: schema.clone(), columns: vec![ColumnInfo::new("id", ty, generic, false, Some(1))], ..TableSchema::default() };
        DesignerTab { connection, database, schema, origins: vec![None], table, loaded: true, ..DesignerTab::default() }
    }

    /// Fill from a loaded definition (after opening or saving).
    pub fn set_saved(&mut self, t: TableSchema) {
        self.origins = t.columns.iter().map(|c| Some(c.name.clone())).collect();
        self.table = t.clone();
        self.original = Some(t);
        self.loaded = true;
        self.preview = None;
        self.preview_for = None;
        self.selected = self.selected.min(self.table.columns.len().saturating_sub(1));
    }

    pub fn dirty(&self) -> bool {
        match &self.original {
            Some(o) => *o != self.table,
            None => true,
        }
    }

    pub fn title(&self) -> String {
        let name = if self.table.name.is_empty() { "Untitled".to_string() } else { self.table.name.clone() };
        match &self.database {
            Some(db) => format!("{name} @{db} (Design)"),
            None => format!("{name} (Design)"),
        }
    }

    /// `table.designSql` / `table.save` params.
    pub fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "connection": self.connection, "database": self.database, "schema": self.schema,
            "table": self.table, "original": self.original, "origins": self.origins,
        })
    }
}

/// `varchar(100) unsigned` -> (`varchar`, `100`, ``, `unsigned`).
pub fn split_type(t: &str) -> (String, String, String, String) {
    let t = t.trim();
    let Some(open) = t.find('(') else {
        // `double precision`, `int unsigned`: the last word may be a modifier.
        for m in [" unsigned zerofill", " unsigned", " zerofill"] {
            if let Some(base) = t.to_ascii_lowercase().strip_suffix(m).map(|b| t.get(..b.len()).unwrap_or(b).to_string()) {
                return (base, String::new(), String::new(), m.trim().to_string());
            }
        }
        return (t.to_string(), String::new(), String::new(), String::new());
    };
    let close = t.get(open..).and_then(|r| r.find(')')).map_or(t.len(), |c| open + c);
    let args = t.get(open + 1..close).unwrap_or("");
    let (len, dec) = match args.split_once(',') {
        Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
        None => (args.trim().to_string(), String::new()),
    };
    let base = t.get(..open).unwrap_or("").trim().to_string();
    let suffix = t.get(close + 1..).unwrap_or("").trim().to_string();
    (base, len, dec, suffix)
}

pub fn join_type(base: &str, len: &str, dec: &str, suffix: &str) -> String {
    let mut t = base.trim().to_string();
    if !len.trim().is_empty() {
        if dec.trim().is_empty() {
            t.push_str(&format!("({})", len.trim()));
        } else {
            t.push_str(&format!("({},{})", len.trim(), dec.trim()));
        }
    }
    if !suffix.trim().is_empty() {
        t.push(' ');
        t.push_str(suffix.trim());
    }
    t
}

fn type_names(d: Dialect) -> &'static [&'static str] {
    match d {
        Dialect::Mysql => &[
            "int",
            "bigint",
            "smallint",
            "tinyint",
            "mediumint",
            "decimal",
            "float",
            "double",
            "bit",
            "char",
            "varchar",
            "tinytext",
            "text",
            "mediumtext",
            "longtext",
            "binary",
            "varbinary",
            "blob",
            "mediumblob",
            "longblob",
            "date",
            "time",
            "datetime",
            "timestamp",
            "year",
            "json",
            "enum",
            "set",
        ],
        Dialect::Postgres => &[
            "integer",
            "bigint",
            "smallint",
            "numeric",
            "real",
            "double precision",
            "boolean",
            "character varying",
            "character",
            "text",
            "bytea",
            "date",
            "time",
            "timestamp",
            "timestamptz",
            "interval",
            "json",
            "jsonb",
            "uuid",
            "inet",
        ],
        _ => &["INTEGER", "TEXT", "REAL", "BLOB", "NUMERIC", "BOOLEAN", "DATE", "DATETIME", "VARCHAR"],
    }
}

const FK_ACTIONS: [&str; 6] = ["", "CASCADE", "SET NULL", "SET DEFAULT", "RESTRICT", "NO ACTION"];

/// A list of field names chosen from `all`, kept in the order they were ticked.
fn fields_picker(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, chosen: &mut Vec<String>, all: &[String]) {
    let text = if chosen.is_empty() { "(choose)".to_string() } else { chosen.join(", ") };
    egui::ComboBox::from_id_salt(id).width(200.0).selected_text(text).show_ui(ui, |ui| {
        for name in all {
            let mut on = chosen.contains(name);
            if ui.checkbox(&mut on, name).changed() {
                if on {
                    chosen.push(name.clone());
                } else {
                    chosen.retain(|c| c != name);
                }
            }
        }
    });
}

fn opt_text(ui: &mut Ui, value: &mut Option<String>, width: f32, hint: &str) {
    let mut s = value.clone().unwrap_or_default();
    // Fixed size: inside a Grid a TextEdit otherwise shrinks to last frame's column width.
    if ui.add_sized([width, 20.0], egui::TextEdit::singleline(&mut s).hint_text(hint)).changed() {
        *value = if s.trim().is_empty() { None } else { Some(s) };
    }
}

/// Renumber primary-key positions in field order after toggling.
fn renumber_pk(t: &mut TableSchema) {
    let mut n = 0u32;
    for c in &mut t.columns {
        if c.pk.is_some() {
            n += 1;
            c.pk = Some(n);
        }
    }
}

/// Context for drawing: the engine and the other tables (for foreign keys).
pub struct Ctx<'a> {
    pub dialect: Dialect,
    pub tables: &'a [String],
}

pub fn show(ui: &mut Ui, idx: usize, tab: &mut DesignerTab, cx: &Ctx, actions: &mut Vec<Action>) {
    let d = cx.dialect;
    // Toolbar.
    egui::Frame::new().fill(Tokens::toolbar_bg()).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let busy = tab.task.is_some();
            let save = ui.add_enabled(!busy && tab.loaded, egui::Button::new(format!("{} Save", ph::FLOPPY_DISK))).on_hover_text("Save (Ctrl+S)").clicked()
                || ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
            if save && !busy && tab.loaded {
                actions.push(Action::DesignerSave(idx));
            }
            ui.separator();
            page_toolbar(ui, tab, d);
            if let Some(task) = &tab.task
                && crate::tasks::running_bar(ui, task, "Working…")
            {
                actions.push(Action::StopTask(idx));
            }
            if tab.dirty() && tab.loaded {
                ui.weak("● modified");
            }
        });
    });
    if !tab.loaded {
        if let Some(e) = &tab.error {
            ui.colored_label(Tokens::error(), e);
        } else {
            ui.spinner();
        }
        return;
    }
    // Page strip.
    ui.horizontal(|ui| {
        for p in pages(d) {
            if ui.add(egui::Button::selectable(tab.page == p, p.label())).clicked() && tab.page != p {
                tab.page = p;
                tab.selected = 0;
            }
        }
    });
    ui.separator();
    if let Some(e) = &tab.error {
        ui.colored_label(Tokens::error(), format!("{} {e}", icons::X));
    }
    if let Some(s) = &tab.status {
        ui.colored_label(Tokens::success(), format!("{} {s}", icons::CHECK));
    }
    let names: Vec<String> = tab.table.columns.iter().map(|c| c.name.clone()).collect();
    egui::ScrollArea::both().id_salt(("designer", idx)).auto_shrink([false, false]).show(ui, |ui| match tab.page {
        DesignerPage::Fields => fields_page(ui, idx, tab, d),
        DesignerPage::Indexes => indexes_page(ui, idx, tab, d, &names),
        DesignerPage::ForeignKeys => fks_page(ui, idx, tab, cx, &names),
        DesignerPage::Uniques => uniques_page(ui, idx, tab, &names),
        DesignerPage::Checks => checks_page(ui, tab),
        DesignerPage::Triggers => triggers_page(ui, idx, tab, d),
        DesignerPage::Options => options_page(ui, tab, d),
        DesignerPage::Comment => {
            ui.label("Comment");
            let mut c = tab.table.comment.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::multiline(&mut c).desired_width(f32::INFINITY).desired_rows(8)).changed() {
                tab.table.comment = if c.is_empty() { None } else { Some(c) };
            }
        }
        DesignerPage::SqlPreview => {
            if tab.preview_for.as_ref() != Some(&tab.table) {
                actions.push(Action::DesignerPreview(idx));
            }
            match &tab.preview {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => {
                    ui.colored_label(Tokens::error(), e);
                }
                Some(Ok(plan)) => {
                    for w in &plan.warnings {
                        ui.colored_label(Tokens::accent(), format!("{} {w}", icons::INFO));
                    }
                    if plan.rebuild {
                        ui.weak("SQLite can't change this in place: the table is rebuilt (new table, copy rows, swap), with foreign keys off.");
                    }
                    let mut sql = if plan.statements.is_empty() { "-- No changes".to_string() } else { plan.statements.join(";\n\n") + ";" };
                    ui.add(egui::TextEdit::multiline(&mut sql).code_editor().desired_width(f32::INFINITY).interactive(true));
                }
            }
        }
    });

    // "Table Name" prompt (first save of a new table).
    if let Some(name) = &mut tab.name_prompt {
        let mut open = true;
        let mut ok = false;
        egui::Window::new("Table Name")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .show(ui.ctx(), |ui| {
                ui.label("Enter table name");
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(260.0).id(egui::Id::new(("tname", idx))));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                ok = (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button(RichText::new("OK").strong()).clicked();
            });
        if ok && !name.trim().is_empty() {
            actions.push(Action::DesignerSaveAs(idx, name.trim().to_string()));
        } else if !open {
            tab.name_prompt = None;
            tab.close_after_save = false;
        }
    }
}

/// Add / Insert / Delete / Primary Key / Move for the current page.
fn page_toolbar(ui: &mut Ui, tab: &mut DesignerTab, d: Dialect) {
    let t = &mut tab.table;
    let sel = tab.selected;
    match tab.page {
        DesignerPage::Fields => {
            let new_field = |n: usize| {
                let (ty, g) = match d {
                    Dialect::Mysql => ("varchar(255)", GenericType::Varchar { len: 255 }),
                    Dialect::Postgres => ("character varying(255)", GenericType::Varchar { len: 255 }),
                    _ => ("TEXT", GenericType::Text),
                };
                ColumnInfo::new(format!("field{n}"), ty, g, true, None)
            };
            if ui.button(format!("{} Add Field", ph::PLUS)).clicked() {
                let n = t.columns.len() + 1;
                t.columns.push(new_field(n));
                tab.origins.push(None);
                tab.selected = t.columns.len() - 1;
            }
            if ui.button(format!("{} Insert Field", ph::ARROW_LINE_DOWN)).clicked() {
                let at = sel.min(t.columns.len());
                let n = t.columns.len() + 1;
                t.columns.insert(at, new_field(n));
                tab.origins.insert(at.min(tab.origins.len()), None);
            }
            let has = sel < t.columns.len();
            if ui.add_enabled(has && t.columns.len() > 1, egui::Button::new(format!("{} Delete Field", ph::MINUS))).clicked() {
                t.columns.remove(sel);
                if sel < tab.origins.len() {
                    tab.origins.remove(sel);
                }
                renumber_pk(t);
                tab.selected = sel.min(t.columns.len().saturating_sub(1));
            }
            if ui.add_enabled(has, egui::Button::new(format!("{} Primary Key", icons::KEY))).clicked()
                && let Some(c) = t.columns.get_mut(sel)
            {
                c.pk = if c.pk.is_some() { None } else { Some(u32::MAX) };
                if c.pk.is_some() {
                    c.nullable = false;
                }
                renumber_pk(t);
            }
            if ui.add_enabled(has && sel > 0, egui::Button::new(format!("{} Move Up", ph::ARROW_UP))).clicked() {
                t.columns.swap(sel, sel - 1);
                if sel < tab.origins.len() {
                    tab.origins.swap(sel, sel - 1);
                }
                renumber_pk(t);
                tab.selected = sel - 1;
            }
            if ui.add_enabled(has && sel + 1 < t.columns.len(), egui::Button::new(format!("{} Move Down", ph::ARROW_DOWN))).clicked() {
                t.columns.swap(sel, sel + 1);
                if sel + 1 < tab.origins.len() {
                    tab.origins.swap(sel, sel + 1);
                }
                renumber_pk(t);
                tab.selected = sel + 1;
            }
        }
        DesignerPage::Indexes => add_delete(ui, "Index", &mut t.indexes, &mut tab.selected, |n| IndexInfo {
            name: format!("{}_idx{n}", t_name(&tab.original)),
            ..IndexInfo::default()
        }),
        DesignerPage::ForeignKeys => add_delete(ui, "Foreign Key", &mut t.foreign_keys, &mut tab.selected, |n| ForeignKeyInfo {
            name: format!("fk_{}_{n}", t_name(&tab.original)),
            ..ForeignKeyInfo::default()
        }),
        DesignerPage::Uniques => add_delete(ui, "Unique", &mut t.uniques, &mut tab.selected, |n| UniqueInfo {
            name: format!("uq_{}_{n}", t_name(&tab.original)),
            columns: Vec::new(),
        }),
        DesignerPage::Checks => add_delete(ui, "Check", &mut t.checks, &mut tab.selected, |n| CheckInfo {
            name: format!("ck_{}_{n}", t_name(&tab.original)),
            expression: String::new(),
        }),
        DesignerPage::Triggers => add_delete(ui, "Trigger", &mut t.triggers, &mut tab.selected, |n| TriggerInfo {
            name: format!("trg_{}_{n}", t_name(&tab.original)),
            timing: "BEFORE".into(),
            events: vec!["INSERT".into()],
            for_each_row: true,
            condition: None,
            body: match d {
                Dialect::Postgres => "EXECUTE FUNCTION my_trigger_function()".into(),
                Dialect::Mysql => "BEGIN\n  -- SET NEW.col = ...;\nEND".into(),
                _ => "BEGIN\n  SELECT 1;\nEND".into(),
            },
        }),
        _ => {}
    }
}

fn t_name(original: &Option<TableSchema>) -> String {
    original.as_ref().map(|t| t.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "table".into())
}

fn add_delete<T>(ui: &mut Ui, what: &str, list: &mut Vec<T>, selected: &mut usize, make: impl FnOnce(usize) -> T) {
    if ui.button(format!("{} Add {what}", ph::PLUS)).clicked() {
        list.push(make(list.len() + 1));
        *selected = list.len() - 1;
    }
    if ui.add_enabled(*selected < list.len(), egui::Button::new(format!("{} Delete {what}", ph::MINUS))).clicked() {
        list.remove(*selected);
        *selected = selected.saturating_sub(1);
    }
}

fn row_marker(ui: &mut Ui, selected: &mut usize, i: usize) {
    let mark = if *selected == i { RichText::new(ph::CARET_RIGHT).color(Tokens::accent()) } else { RichText::new(" ") };
    if ui.add(egui::Button::new(mark).frame(false).min_size(egui::vec2(14.0, 0.0))).clicked() {
        *selected = i;
    }
}

fn fields_page(ui: &mut Ui, idx: usize, tab: &mut DesignerTab, d: Dialect) {
    let mut select = None;
    egui::Grid::new(("fields", idx)).striped(true).num_columns(8).spacing([6.0, 3.0]).show(ui, |ui| {
        for h in ["", "Name", "Type", "Length", "Decimals", "Not null", "Key", "Comment"] {
            ui.strong(h);
        }
        ui.end_row();
        let mut pk_changed = false;
        for (i, c) in tab.table.columns.iter_mut().enumerate() {
            let mut sel = tab.selected;
            row_marker(ui, &mut sel, i);
            if sel != tab.selected {
                select = Some(i);
            }
            if ui.add_sized([150.0, 20.0], egui::TextEdit::singleline(&mut c.name)).gained_focus() {
                select = Some(i);
            }
            let (mut base, mut len, mut dec, suffix) = split_type(&c.native_type);
            let mut changed = false;
            ui.horizontal(|ui| {
                changed |= ui.add_sized([120.0, 20.0], egui::TextEdit::singleline(&mut base)).changed();
                ui.menu_button(ph::CARET_DOWN, |ui| {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for t in type_names(d) {
                            if ui.button(*t).clicked() {
                                base = (*t).to_string();
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                });
            });
            changed |= ui.add_sized([50.0, 20.0], egui::TextEdit::singleline(&mut len)).changed();
            changed |= ui.add_sized([50.0, 20.0], egui::TextEdit::singleline(&mut dec)).changed();
            if changed {
                c.native_type = join_type(&base, &len, &dec, &suffix);
            }
            let mut not_null = !c.nullable;
            if ui.checkbox(&mut not_null, "").changed() {
                c.nullable = !not_null;
            }
            let key = if c.pk.is_some() { RichText::new(format!("{} {}", icons::KEY, c.pk.unwrap_or(0))).color(Tokens::accent()) } else { RichText::new("  ") };
            if ui.add(egui::Button::new(key).frame(false)).on_hover_text("Primary key (click to toggle)").clicked() {
                c.pk = if c.pk.is_some() { None } else { Some(u32::MAX) };
                if c.pk.is_some() {
                    c.nullable = false;
                }
                pk_changed = true;
            }
            if d != Dialect::Sqlite {
                opt_text(ui, &mut c.comment, 200.0, "");
            } else {
                ui.label("");
            }
            ui.end_row();
        }
        if pk_changed {
            renumber_pk(&mut tab.table);
        }
    });
    if let Some(i) = select {
        tab.selected = i;
    }
    // Field properties (Navicat shows them under the grid).
    let Some(c) = tab.table.columns.get_mut(tab.selected) else { return };
    ui.add_space(8.0);
    ui.separator();
    ui.strong(format!("Field properties: {}", c.name));
    egui::Grid::new(("field_props", idx)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Default");
        opt_text(ui, &mut c.default, 260.0, "e.g. 0, 'text', CURRENT_TIMESTAMP");
        ui.end_row();
        ui.label("Auto Increment");
        ui.checkbox(
            &mut c.auto_increment,
            match d {
                Dialect::Postgres => "Identity (GENERATED BY DEFAULT)",
                Dialect::Sqlite => "INTEGER PRIMARY KEY (rowid)",
                _ => "AUTO_INCREMENT",
            },
        );
        ui.end_row();
        if d != Dialect::Sqlite {
            ui.label("Collation");
            opt_text(ui, &mut c.collation, 200.0, "table default");
            ui.end_row();
        }
        if d == Dialect::Mysql {
            let (base, len, dec, suffix) = split_type(&c.native_type);
            let mut unsigned = suffix.contains("unsigned");
            ui.label("Unsigned");
            if ui.checkbox(&mut unsigned, "").changed() {
                let s = if unsigned { "unsigned".to_string() } else { suffix.replace("unsigned", "").trim().to_string() };
                c.native_type = join_type(&base, &len, &dec, &s);
            }
            ui.end_row();
            let mut on_update = c.on_update.is_some();
            ui.label("On Update Current_Timestamp");
            if ui.checkbox(&mut on_update, "").changed() {
                c.on_update = on_update.then(|| "CURRENT_TIMESTAMP".to_string());
            }
            ui.end_row();
        }
    });
}

fn indexes_page(ui: &mut Ui, idx: usize, tab: &mut DesignerTab, d: Dialect, names: &[String]) {
    egui::Grid::new(("indexes", idx)).striped(true).spacing([6.0, 3.0]).show(ui, |ui| {
        for h in ["", "Name", "Fields", "Index Type", "Index Method", "Comment"] {
            ui.strong(h);
        }
        ui.end_row();
        for (i, x) in tab.table.indexes.iter_mut().enumerate() {
            row_marker(ui, &mut tab.selected, i);
            ui.add_sized([160.0, 20.0], egui::TextEdit::singleline(&mut x.name));
            if x.native_ddl.is_some() && x.columns.is_empty() {
                ui.weak("(expression)").on_hover_text(x.native_ddl.clone().unwrap_or_default());
            } else {
                fields_picker(ui, ("ixf", idx, i), &mut x.columns, names);
            }
            let current = match (&x.kind, x.unique) {
                (Some(k), _) => k.clone(),
                (None, true) => "UNIQUE".into(),
                _ => "NORMAL".into(),
            };
            let kinds: &[&str] = if d == Dialect::Mysql { &["NORMAL", "UNIQUE", "FULLTEXT", "SPATIAL"] } else { &["NORMAL", "UNIQUE"] };
            egui::ComboBox::from_id_salt(("ixk", idx, i)).width(100.0).selected_text(current.clone()).show_ui(ui, |ui| {
                for k in kinds {
                    if ui.selectable_label(current == *k, *k).clicked() {
                        x.unique = *k == "UNIQUE";
                        x.kind = matches!(*k, "FULLTEXT" | "SPATIAL").then(|| (*k).to_string());
                    }
                }
            });
            let methods: &[&str] = match d {
                Dialect::Mysql => &["", "BTREE", "HASH"],
                Dialect::Postgres => &["", "BTREE", "HASH", "GIST", "GIN", "BRIN", "SPGIST"],
                _ => &[""],
            };
            let m = x.method.clone().unwrap_or_default();
            egui::ComboBox::from_id_salt(("ixm", idx, i)).width(90.0).selected_text(m.clone()).show_ui(ui, |ui| {
                for opt in methods {
                    if ui.selectable_label(m == *opt, if opt.is_empty() { "(default)" } else { opt }).clicked() {
                        x.method = (!opt.is_empty()).then(|| (*opt).to_string());
                    }
                }
            });
            if d == Dialect::Sqlite {
                ui.label("");
            } else {
                opt_text(ui, &mut x.comment, 180.0, "");
            }
            ui.end_row();
        }
    });
}

fn fks_page(ui: &mut Ui, idx: usize, tab: &mut DesignerTab, cx: &Ctx, names: &[String]) {
    egui::Grid::new(("fks", idx)).striped(true).spacing([6.0, 3.0]).show(ui, |ui| {
        for h in ["", "Name", "Fields", "Referenced Table", "Referenced Fields", "On Delete", "On Update"] {
            ui.strong(h);
        }
        ui.end_row();
        for (i, f) in tab.table.foreign_keys.iter_mut().enumerate() {
            row_marker(ui, &mut tab.selected, i);
            ui.add_sized([150.0, 20.0], egui::TextEdit::singleline(&mut f.name));
            fields_picker(ui, ("fkf", idx, i), &mut f.columns, names);
            egui::ComboBox::from_id_salt(("fkt", idx, i)).width(150.0).selected_text(f.ref_table.clone()).show_ui(ui, |ui| {
                for t in cx.tables {
                    ui.selectable_value(&mut f.ref_table, t.clone(), t);
                }
            });
            let mut refs = f.ref_columns.join(", ");
            if ui.add_sized([150.0, 20.0], egui::TextEdit::singleline(&mut refs).hint_text("id")).changed() {
                f.ref_columns = refs.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            for (k, action) in [("del", &mut f.on_delete), ("upd", &mut f.on_update)] {
                let cur = action.clone().unwrap_or_default();
                egui::ComboBox::from_id_salt(("fka", k, idx, i)).width(100.0).selected_text(cur.clone()).show_ui(ui, |ui| {
                    for a in FK_ACTIONS {
                        if ui.selectable_label(cur == a, if a.is_empty() { "(none)" } else { a }).clicked() {
                            *action = (!a.is_empty() && a != "NO ACTION").then(|| a.to_string());
                        }
                    }
                });
            }
            ui.end_row();
        }
    });
}

fn uniques_page(ui: &mut Ui, idx: usize, tab: &mut DesignerTab, names: &[String]) {
    egui::Grid::new(("uniques", idx)).striped(true).spacing([6.0, 3.0]).show(ui, |ui| {
        for h in ["", "Name", "Fields"] {
            ui.strong(h);
        }
        ui.end_row();
        for (i, u) in tab.table.uniques.iter_mut().enumerate() {
            row_marker(ui, &mut tab.selected, i);
            ui.add_sized([180.0, 20.0], egui::TextEdit::singleline(&mut u.name));
            fields_picker(ui, ("uqf", idx, i), &mut u.columns, names);
            ui.end_row();
        }
    });
}

fn checks_page(ui: &mut Ui, tab: &mut DesignerTab) {
    egui::Grid::new("checks").striped(true).spacing([6.0, 3.0]).show(ui, |ui| {
        for h in ["", "Name", "Check"] {
            ui.strong(h);
        }
        ui.end_row();
        for (i, c) in tab.table.checks.iter_mut().enumerate() {
            row_marker(ui, &mut tab.selected, i);
            ui.add_sized([160.0, 20.0], egui::TextEdit::singleline(&mut c.name));
            ui.add_sized([420.0, 20.0], egui::TextEdit::singleline(&mut c.expression).hint_text("price > 0").font(egui::TextStyle::Monospace));
            ui.end_row();
        }
    });
}

fn triggers_page(ui: &mut Ui, idx: usize, tab: &mut DesignerTab, d: Dialect) {
    egui::Grid::new(("triggers", idx)).striped(true).spacing([6.0, 3.0]).show(ui, |ui| {
        for h in ["", "Name", "Fires", "Insert", "Update", "Delete", "For Each Row"] {
            ui.strong(h);
        }
        ui.end_row();
        for (i, t) in tab.table.triggers.iter_mut().enumerate() {
            row_marker(ui, &mut tab.selected, i);
            ui.add_sized([160.0, 20.0], egui::TextEdit::singleline(&mut t.name));
            egui::ComboBox::from_id_salt(("trt", idx, i)).width(100.0).selected_text(t.timing.clone()).show_ui(ui, |ui| {
                let timings: &[&str] = if d == Dialect::Mysql { &["BEFORE", "AFTER"] } else { &["BEFORE", "AFTER", "INSTEAD OF"] };
                for x in timings {
                    ui.selectable_value(&mut t.timing, (*x).to_string(), *x);
                }
            });
            for ev in ["INSERT", "UPDATE", "DELETE"] {
                let mut on = t.events.iter().any(|e| e.starts_with(ev));
                if ui.checkbox(&mut on, "").changed() {
                    if d != Dialect::Postgres {
                        // MySQL and SQLite triggers fire on exactly one event.
                        t.events.clear();
                    }
                    t.events.retain(|e| !e.starts_with(ev));
                    if on {
                        t.events.push(ev.to_string());
                    }
                }
            }
            if d == Dialect::Mysql {
                ui.label("✓");
            } else {
                ui.checkbox(&mut t.for_each_row, "");
            }
            ui.end_row();
        }
    });
    let Some(t) = tab.table.triggers.get_mut(tab.selected) else { return };
    ui.add_space(8.0);
    ui.separator();
    if d != Dialect::Mysql {
        ui.horizontal(|ui| {
            ui.label("When");
            opt_text(ui, &mut t.condition, 400.0, "new.price > 0");
        });
    }
    ui.label(if d == Dialect::Postgres { "Definition (EXECUTE FUNCTION …)" } else { "Definition" });
    ui.add(egui::TextEdit::multiline(&mut t.body).code_editor().desired_width(f32::INFINITY).desired_rows(10));
}

type Opts = std::collections::BTreeMap<String, String>;

fn opt_field(ui: &mut Ui, o: &mut Opts, label: &str, key: &str, hint: &str) {
    ui.label(label);
    let mut v = o.get(key).cloned().unwrap_or_default();
    if ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut v).hint_text(hint)).changed() {
        if v.trim().is_empty() {
            o.remove(key);
        } else {
            o.insert(key.to_string(), v);
        }
    }
    ui.end_row();
}

fn opt_flag(ui: &mut Ui, o: &mut Opts, label: &str, key: &str) {
    let mut on = o.get(key).is_some_and(|v| v == "true");
    ui.label(label);
    if ui.checkbox(&mut on, "").changed() {
        if on {
            o.insert(key.to_string(), "true".into());
        } else {
            o.remove(key);
        }
    }
    ui.end_row();
}

fn options_page(ui: &mut Ui, tab: &mut DesignerTab, d: Dialect) {
    let o = &mut tab.table.options;
    egui::Grid::new("table_options").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| match d {
        Dialect::Mysql => {
            opt_field(ui, o, "Engine", "engine", "InnoDB");
            opt_field(ui, o, "Default Character Set", "charset", "utf8mb4");
            opt_field(ui, o, "Default Collation", "collation", "utf8mb4_0900_ai_ci");
            opt_field(ui, o, "Auto Increment", "auto_increment", "");
            opt_field(ui, o, "Row Format", "row_format", "Dynamic");
        }
        Dialect::Postgres => {
            opt_field(ui, o, "Tablespace", "tablespace", "pg_default");
            opt_flag(ui, o, "Unlogged", "unlogged");
        }
        _ => {
            opt_flag(ui, o, "WITHOUT ROWID", "without_rowid");
            opt_flag(ui, o, "STRICT", "strict");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_split_and_join() {
        assert_eq!(split_type("varchar(100)"), ("varchar".into(), "100".into(), String::new(), String::new()));
        assert_eq!(split_type("decimal(10, 2) unsigned"), ("decimal".into(), "10".into(), "2".into(), "unsigned".into()));
        assert_eq!(split_type("int unsigned"), ("int".into(), String::new(), String::new(), "unsigned".into()));
        assert_eq!(split_type("double precision"), ("double precision".into(), String::new(), String::new(), String::new()));
        assert_eq!(join_type("decimal", "10", "2", "unsigned"), "decimal(10,2) unsigned");
        assert_eq!(join_type("text", "", "", ""), "text");
        assert_eq!(split_type("weird(("), ("weird".into(), "(".into(), String::new(), String::new()));
    }
}
