//! Table viewer: Navicat-style grid with live editing (double-click a cell, Enter to keep, Esc to
//! cancel; Apply writes all pending changes in one transaction), a filter wizard bar, sortable
//! headers and paging.

use dbcraft_core::Value;
use dbcraft_engine::FILTER_OPS;
use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use serde_json::{Map, Value as Json, json};

use crate::app::Action;
use crate::grid::GridSel;
use crate::icons;
use crate::state::{FilterRow, TableTab};
use crate::theme::Tokens;

const NO_VALUE_OPS: &[&str] = &["is_null", "is_not_null", "is_empty", "is_not_empty"];

/// Params for `table.data` from the tab's paging, sort and applied filters.
pub fn data_params(tab: &TableTab) -> Json {
    let filters: Vec<Json> = tab
        .applied_filters
        .iter()
        .filter(|f| f.enabled && !f.column.is_empty() && !f.op.is_empty())
        .map(|f| {
            if NO_VALUE_OPS.contains(&f.op.as_str()) {
                json!({"column": f.column, "op": f.op})
            } else {
                json!({"column": f.column, "op": f.op, "value": f.value})
            }
        })
        .collect();
    let sort: Vec<Json> = tab.sort.iter().map(|(c, desc)| json!({"column": c, "desc": desc})).collect();
    json!({
        "connection": tab.connection, "database": tab.database, "schema": tab.schema, "table": tab.table,
        "filters": filters, "match": if tab.match_any { "any" } else { "all" }, "sort": sort,
        "limit": tab.limit.max(1), "offset": tab.offset,
    })
}

/// Params for `table.applyEdits` from the pending edits. Rows are addressed by primary key, or by
/// every column when the table has none (the engine then insists each change hits one row).
pub fn edit_params(tab: &TableTab) -> Option<Json> {
    let page = tab.page.as_ref()?;
    let names: Vec<&str> = page.result.columns.iter().map(|c| c.name.as_str()).collect();
    let mut key_cols: Vec<usize> = page.primary_key.iter().filter_map(|k| names.iter().position(|n| n == k)).collect();
    if key_cols.is_empty() || key_cols.len() != page.primary_key.len() {
        key_cols = (0..names.len()).collect();
    }
    let key = |row: usize| -> Option<Json> {
        let r = page.result.rows.get(row)?;
        let mut m = Map::new();
        for &c in &key_cols {
            m.insert((*names.get(c)?).to_string(), serde_json::to_value(r.get(c)?).ok()?);
        }
        Some(Json::Object(m))
    };
    let cell = |v: &Option<String>| v.as_ref().map_or(Json::Null, |s| Json::String(s.clone()));
    let mut changes = Vec::new();
    let mut updates: std::collections::BTreeMap<usize, Map<String, Json>> = Default::default();
    for ((r, c), v) in &tab.edits {
        if !tab.deleted.contains(r)
            && let Some(name) = names.get(*c)
        {
            updates.entry(*r).or_default().insert((*name).to_string(), cell(v));
        }
    }
    for (r, set) in updates {
        changes.push(json!({"op": "update", "key": key(r)?, "set": set}));
    }
    for r in &tab.deleted {
        changes.push(json!({"op": "delete", "key": key(*r)?}));
    }
    for row in &tab.inserted {
        let values: Map<String, Json> = row.iter().filter_map(|(c, v)| Some(((*names.get(*c)?).to_string(), cell(v)))).collect();
        changes.push(json!({"op": "insert", "values": values}));
    }
    Some(json!({"connection": tab.connection, "database": tab.database, "schema": tab.schema, "table": tab.table, "changes": changes}))
}

fn shown(v: &Value) -> (String, bool) {
    match v {
        Value::Null => ("(NULL)".into(), true),
        other => (other.display(), false),
    }
}

pub fn show(ui: &mut Ui, idx: usize, tab: &mut TableTab, dialect: dbcraft_sql::Dialect, actions: &mut Vec<Action>) {
    let Some(page) = tab.page.clone() else {
        if let Some(e) = &tab.error {
            ui.colored_label(Tokens::ERROR, e);
            if ui.button(format!("{} Retry", icons::ARROWS_CLOCKWISE)).clicked() {
                actions.push(Action::ReloadTable(idx));
            }
        } else if let Some(task) = &tab.task {
            if crate::tasks::running_bar(ui, task, "Loading…") {
                actions.push(Action::StopTask(idx));
            }
        } else {
            ui.spinner();
        }
        return;
    };
    let names: Vec<String> = page.result.columns.iter().map(|c| c.name.clone()).collect();
    let dirty = tab.dirty();

    // Toolbar.
    egui::Frame::new().fill(Tokens::TOOLBAR_BG).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_enabled(false, egui::Button::new("Begin Transaction")).on_disabled_hover_text("Changes are applied in one transaction by Apply");
            let filter_label =
                if tab.applied_filters.iter().any(|f| f.enabled) { format!("{} Filter •", icons::FUNNEL) } else { format!("{} Filter", icons::FUNNEL) };
            if ui.add(egui::Button::selectable(tab.show_filter, filter_label)).clicked() {
                tab.show_filter ^= true;
                if tab.show_filter && tab.filters.is_empty() {
                    tab.filters.push(FilterRow { enabled: true, column: names.first().cloned().unwrap_or_default(), op: "=".into(), value: String::new() });
                }
            }
            ui.add_enabled(false, egui::Button::new(format!("{} Import", icons::DOWNLOAD_SIMPLE))).on_disabled_hover_text("Coming soon");
            ui.add_enabled(false, egui::Button::new(format!("{} Export", icons::UPLOAD_SIMPLE))).on_disabled_hover_text("Coming soon");
            if dirty {
                ui.colored_label(Tokens::ACCENT, format!("{} pending changes", tab.edits.len() + tab.deleted.len() + tab.inserted.len()));
            }
        });
    });

    if tab.show_filter {
        filter_bar(ui, idx, tab, &names, dirty, actions);
    }
    if let Some(e) = &tab.error {
        ui.colored_label(Tokens::ERROR, format!("{} {e}", icons::X));
    }
    if let Some(task) = &tab.task
        && crate::tasks::running_bar(ui, task, "Loading…")
    {
        actions.push(Action::StopTask(idx));
    }

    // Bottom bar first so the grid takes the remaining space.
    egui::Panel::bottom(egui::Id::new(("table_bottom", idx))).show(ui, |ui| bottom_bar(ui, idx, tab, &page, dirty, actions));

    let loaded = page.result.rows.len();
    let total_rows = loaded + tab.inserted.len();
    let prev_row = tab.selected.map(|s| s.0);
    let prev_sel = tab.selected;
    keyboard(ui, tab, loaded);
    if tab.selected != prev_sel
        && let Some(s) = tab.selected
    {
        tab.grid = GridSel::Cells { anchor: s, focus: s };
    }
    crate::grid::keyboard(ui, &page.result, &mut tab.grid);
    let mut sort_set: Option<Option<(String, bool)>> = None;
    let mut copy_action: Option<&'static str> = None;
    let mut delete_rows: Vec<usize> = Vec::new();
    let mut start_edit = None;
    let mut commit = false;
    let mut cancel = false;
    let mut set_cell: Option<(usize, usize, Option<String>)> = None;
    let mut toggle_delete = None;
    let mut sort_click = None;

    egui::ScrollArea::horizontal().id_salt(("grid", idx)).show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt(("grid_table", idx))
            .striped(true)
            .resizable(true)
            .min_scrolled_height(0.0)
            .column(Column::exact(44.0))
            .columns(Column::initial(130.0).at_least(40.0).clip(true), names.len())
            .header(24.0, |mut h| {
                h.col(|ui| {
                    let rect = ui.max_rect();
                    ui.painter().rect_filled(rect, 0.0, Tokens::GRID_HEADER);
                    if ui.allocate_rect(rect, egui::Sense::click()).on_hover_text("Select All (Ctrl+A)").clicked() {
                        tab.grid = GridSel::All;
                    }
                });
                for (c, name) in names.iter().enumerate() {
                    h.col(|ui| {
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(rect, 0.0, if tab.grid.col_fully_selected(c) { Tokens::SELECTION_STROKE } else { Tokens::GRID_HEADER });
                        let mut label = String::new();
                        if page.primary_key.contains(name) {
                            label.push_str(icons::KEY);
                            label.push(' ');
                        }
                        label.push_str(name);
                        let sorted = tab.sort.as_ref().filter(|(s, _)| s == name).map(|(_, d)| *d);
                        let ty = page.result.columns.get(c).map(|c| c.type_name.clone()).unwrap_or_default();
                        ui.horizontal(|ui| {
                            // Sort toggle (asc → desc → none); clicking the name selects the column.
                            let arrow = match sorted {
                                Some(true) => RichText::new(icons::SORT_DESCENDING).color(Tokens::ACCENT),
                                Some(false) => RichText::new(icons::SORT_ASCENDING).color(Tokens::ACCENT),
                                None => RichText::new(icons::SORT_ASCENDING).color(Tokens::CLOSED),
                            };
                            if ui.add(egui::Button::new(arrow).frame(false).small()).on_hover_text("Sort").clicked() {
                                sort_click = Some(name.clone());
                            }
                            let r = ui.add(egui::Label::new(RichText::new(label).strong()).truncate().sense(egui::Sense::click()));
                            let r = r.on_hover_text(format!("{ty}\nClick: select column · right-click: sort"));
                            if r.clicked() {
                                let m = ui.input(|i| i.modifiers);
                                tab.grid.click_col(c, m);
                                tab.selected = None;
                            }
                            r.context_menu(|ui| {
                                if ui.button(format!("{} Sort Ascending", icons::SORT_ASCENDING)).clicked() {
                                    sort_set = Some(Some((name.clone(), false)));
                                    ui.close();
                                }
                                if ui.button(format!("{} Sort Descending", icons::SORT_DESCENDING)).clicked() {
                                    sort_set = Some(Some((name.clone(), true)));
                                    ui.close();
                                }
                                if ui.add_enabled(tab.sort.is_some(), egui::Button::new("Remove Sort")).clicked() {
                                    sort_set = Some(None);
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Copy Field Name").clicked() {
                                    ui.ctx().copy_text(name.clone());
                                    ui.close();
                                }
                            });
                        });
                    });
                }
            })
            .body(|body| {
                body.rows(22.0, total_rows, |mut row| {
                    let r = row.index();
                    let inserted = r >= loaded;
                    let deleted = tab.deleted.contains(&r);
                    row.col(|ui| {
                        let mark = if inserted {
                            "+".to_string()
                        } else if deleted {
                            "−".to_string()
                        } else {
                            (tab.offset + r as u64 + 1).to_string()
                        };
                        if crate::grid::row_header(ui, &mark, r, &mut tab.grid).clicked() {
                            tab.selected = None;
                        }
                    });
                    for c in 0..names.len() {
                        row.col(|ui| {
                            let original = if inserted { None } else { page.result.rows.get(r).and_then(|x| x.get(c)) };
                            let pending: Option<&Option<String>> =
                                if inserted { tab.inserted.get(r - loaded).and_then(|m| m.get(&c)) } else { tab.edits.get(&(r, c)) };
                            let bg = if deleted {
                                Some(Tokens::DELETED)
                            } else if inserted {
                                Some(Tokens::INSERTED)
                            } else if pending.is_some() {
                                Some(Tokens::EDITED)
                            } else {
                                None
                            };
                            crate::grid::paint_cell(ui, &tab.grid, r, c, bg);
                            if let Some((er, ec, text)) = &mut tab.editing
                                && *er == r
                                && *ec == c
                            {
                                let resp = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
                                // Enter or clicking away keeps the value; Esc cancels.
                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    cancel = true;
                                } else if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    commit = true;
                                } else if !resp.has_focus() {
                                    resp.request_focus();
                                }
                                return;
                            }
                            let (text, weak) = match pending {
                                Some(None) => ("(NULL)".to_string(), true),
                                Some(Some(s)) => (s.clone(), false),
                                None if inserted => ("(default)".to_string(), true),
                                None => original.map(shown).unwrap_or_default(),
                            };
                            let mut rt = RichText::new(text);
                            if weak {
                                rt = rt.color(Tokens::TEXT_WEAK).italics();
                            }
                            if deleted {
                                rt = rt.strikethrough();
                            }
                            let resp = ui.add(egui::Label::new(rt).truncate().sense(egui::Sense::click()));
                            let is_blob = matches!(original, Some(Value::Bytes(_)));
                            let editable = !deleted && !is_blob;
                            let is_selected = tab.selected == Some((r, c));
                            // Navicat: double-click, or click an already-selected cell, or F2/Enter on it, to edit.
                            let key_edit =
                                is_selected && tab.editing.is_none() && ui.input(|i| i.key_pressed(egui::Key::F2) || i.key_pressed(egui::Key::Enter));
                            let mods = ui.input(|i| i.modifiers);
                            let plain = !mods.shift && !mods.command;
                            let wants_edit = plain && (resp.double_clicked() || resp.triple_clicked() || (resp.clicked() && is_selected) || key_edit);
                            if resp.clicked() {
                                tab.grid.click_cell(r, c, mods);
                                if plain {
                                    tab.selected = Some((r, c));
                                }
                            }
                            if resp.secondary_clicked() && !tab.grid.contains(r, c) {
                                tab.grid.click_cell(r, c, egui::Modifiers::NONE);
                                tab.selected = Some((r, c));
                            }
                            if wants_edit && editable {
                                let start = match pending {
                                    Some(v) => v.clone().unwrap_or_default(),
                                    None => original.filter(|v| !matches!(v, Value::Null)).map(Value::display).unwrap_or_default(),
                                };
                                start_edit = Some((r, c, start));
                            }
                            resp.context_menu(|ui| {
                                if ui.add_enabled(!deleted && !is_blob, egui::Button::new("Set to NULL")).clicked() {
                                    set_cell = Some((r, c, None));
                                    ui.close();
                                }
                                if ui.add_enabled(!deleted && !is_blob, egui::Button::new("Set to Empty String")).clicked() {
                                    set_cell = Some((r, c, Some(String::new())));
                                    ui.close();
                                }
                                ui.separator();
                                for (label, act) in [("Copy", "copy"), ("Copy with Headers", "copy_headers"), ("Copy as INSERT Statement", "copy_insert")] {
                                    if ui.button(label).clicked() {
                                        copy_action = Some(act);
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                let rows: Vec<usize> = match &tab.grid {
                                    GridSel::Rows { rows, .. } if rows.contains(&r) => rows.iter().copied().collect(),
                                    _ => vec![r],
                                };
                                let label = if deleted {
                                    "Undo Delete Record".to_string()
                                } else if rows.len() > 1 {
                                    format!("Delete {} Records", rows.len())
                                } else {
                                    "Delete Record".to_string()
                                };
                                if ui.button(format!("{} {label}", icons::TRASH)).clicked() {
                                    if rows.len() > 1 {
                                        delete_rows = rows;
                                    } else {
                                        toggle_delete = Some(r);
                                    }
                                    ui.close();
                                }
                            });
                        });
                    }
                });
            });
    });

    if let Some(set) = sort_set {
        if dirty {
            tab.error = Some("Apply or discard your changes before sorting.".into());
        } else {
            tab.sort = set;
            tab.offset = 0;
            actions.push(Action::ReloadTable(idx));
        }
    }
    match copy_action {
        Some("copy") => ui.ctx().copy_text(crate::grid::selection_tsv(&page.result, &tab.grid, false)),
        Some("copy_headers") => ui.ctx().copy_text(crate::grid::selection_tsv(&page.result, &tab.grid, true)),
        Some("copy_insert") => ui.ctx().copy_text(crate::grid::selection_inserts(&page.result, &tab.grid, dialect, &tab.table)),
        _ => {}
    }
    for r in delete_rows {
        if r < loaded {
            tab.deleted.insert(r);
        }
    }
    if let Some(name) = sort_click {
        if dirty {
            tab.error = Some("Apply or discard your changes before sorting.".into());
        } else {
            tab.sort = match &tab.sort {
                Some((s, false)) if *s == name => Some((name, true)),
                Some((s, true)) if *s == name => None,
                _ => Some((name, false)),
            };
            tab.offset = 0;
            actions.push(Action::ReloadTable(idx));
        }
    }
    if commit && let Some((r, c, text)) = tab.editing.take() {
        set_cell = Some((r, c, Some(text)));
    }
    if cancel {
        tab.editing = None;
    }
    if let Some((r, c, v)) = set_cell {
        if r >= loaded {
            if let Some(m) = tab.inserted.get_mut(r - loaded) {
                m.insert(c, v);
            }
        } else {
            let original = page.result.rows.get(r).and_then(|x| x.get(c));
            let same = match (&v, original) {
                (None, Some(Value::Null)) => true,
                (Some(s), Some(o)) if !matches!(o, Value::Null) => *s == o.display(),
                _ => false,
            };
            if same {
                tab.edits.remove(&(r, c));
            } else {
                tab.edits.insert((r, c), v);
            }
        }
    }
    if let Some(r) = toggle_delete {
        if r >= loaded {
            if r - loaded < tab.inserted.len() {
                tab.inserted.remove(r - loaded);
            }
        } else if !tab.deleted.remove(&r) {
            tab.deleted.insert(r);
        }
    }
    if let Some(e) = start_edit {
        tab.selected = Some((e.0, e.1));
        tab.editing = Some(e);
    }

    // Navicat saves a record when you move to another one.
    let new_row = tab.selected.map(|s| s.0);
    if let Some(prev) = prev_row
        && new_row != Some(prev)
        && tab.editing.is_none()
        && row_dirty(tab, prev, loaded)
    {
        actions.push(Action::ApplyTable(idx));
    }
}

/// Whether a row has unsaved changes (edited cells, or a new record with values).
pub fn row_dirty(tab: &TableTab, row: usize, loaded: usize) -> bool {
    if row >= loaded {
        return tab.inserted.get(row - loaded).is_some_and(|m| !m.is_empty());
    }
    tab.edits.keys().any(|(r, _)| *r == row)
}

/// Grid shortcuts (Navicat's): Insert / Ctrl+N add a record, Ctrl+Delete deletes the selected one,
/// Esc discards the selected record's changes, arrows move the selection.
fn keyboard(ui: &Ui, tab: &mut TableTab, loaded: usize) {
    if tab.editing.is_some() || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (add, delete, discard, up, down, left, right) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Insert) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::N),
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Delete),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
        )
    });
    let total = loaded + tab.inserted.len();
    if add {
        tab.inserted.push(Default::default());
        tab.selected = Some((total, tab.selected.map_or(0, |s| s.1)));
    }
    if let Some((r, c)) = tab.selected {
        if delete {
            if r >= loaded {
                if r - loaded < tab.inserted.len() {
                    tab.inserted.remove(r - loaded);
                }
                tab.selected = None;
            } else if !tab.deleted.remove(&r) {
                tab.deleted.insert(r);
            }
        }
        if discard {
            if r >= loaded {
                if r - loaded < tab.inserted.len() {
                    tab.inserted.remove(r - loaded);
                }
                tab.selected = None;
            } else {
                tab.edits.retain(|(er, _), _| *er != r);
                tab.deleted.remove(&r);
            }
        }
        let total = loaded + tab.inserted.len();
        if total > 0 && (up || down || left || right) {
            let r = if up {
                r.saturating_sub(1)
            } else if down {
                (r + 1).min(total - 1)
            } else {
                r
            };
            let c = if left {
                c.saturating_sub(1)
            } else if right {
                c + 1
            } else {
                c
            };
            tab.selected = Some((r, c));
        }
    }
}

fn filter_bar(ui: &mut Ui, idx: usize, tab: &mut TableTab, names: &[String], dirty: bool, actions: &mut Vec<Action>) {
    egui::Frame::new().fill(Tokens::PANEL_BG).stroke(egui::Stroke::new(1.0, Tokens::BORDER)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        let mut remove = None;
        for (i, f) in tab.filters.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.enabled, "");
                egui::ComboBox::from_id_salt(("fcol", idx, i)).width(150.0).selected_text(&f.column).show_ui(ui, |ui| {
                    for n in names {
                        ui.selectable_value(&mut f.column, n.clone(), n);
                    }
                });
                let op_label = FILTER_OPS.iter().find(|(id, _)| *id == f.op).map(|(_, l)| *l).unwrap_or("");
                egui::ComboBox::from_id_salt(("fop", idx, i)).width(170.0).selected_text(op_label).show_ui(ui, |ui| {
                    for (id, label) in FILTER_OPS {
                        ui.selectable_value(&mut f.op, (*id).to_string(), *label);
                    }
                });
                if !NO_VALUE_OPS.contains(&f.op.as_str()) {
                    ui.add(egui::TextEdit::singleline(&mut f.value).desired_width(200.0).hint_text("value"));
                }
                if ui.small_button(icons::X).on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            tab.filters.remove(i);
        }
        ui.horizontal(|ui| {
            if ui.button(format!("{} Add", icons::PLUS)).clicked() {
                tab.filters.push(FilterRow { enabled: true, column: names.first().cloned().unwrap_or_default(), op: "=".into(), value: String::new() });
            }
            ui.label("Match");
            egui::ComboBox::from_id_salt(("fmatch", idx)).width(60.0).selected_text(if tab.match_any { "any" } else { "all" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut tab.match_any, false, "all");
                ui.selectable_value(&mut tab.match_any, true, "any");
            });
            ui.label("of the conditions");
            ui.separator();
            let apply = ui
                .add_enabled(!dirty, egui::Button::new(RichText::new(format!("{} Apply Filter", icons::FUNNEL)).strong()))
                .on_disabled_hover_text("Apply or discard your changes first");
            let ctrl_r = !dirty && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R));
            if apply.clicked() || ctrl_r || (!dirty && ui.input(|i| i.key_pressed(egui::Key::Enter)) && tab.editing.is_none()) {
                tab.applied_filters = tab.filters.clone();
                tab.offset = 0;
                actions.push(Action::ReloadTable(idx));
            }
            if ui.add_enabled(!dirty, egui::Button::new("Clear")).clicked() {
                tab.filters.clear();
                tab.applied_filters.clear();
                tab.offset = 0;
                actions.push(Action::ReloadTable(idx));
            }
        });
    });
}

fn bottom_bar(ui: &mut Ui, idx: usize, tab: &mut TableTab, page: &dbcraft_engine::TablePage, dirty: bool, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.button(icons::PLUS).on_hover_text("Add Record").clicked() {
            tab.inserted.push(Default::default());
        }
        let selected_rows: Vec<usize> = match &tab.grid {
            GridSel::Rows { rows, .. } => rows.iter().copied().collect(),
            _ => Vec::new(),
        };
        let can_del = tab.selected.is_some() || !selected_rows.is_empty();
        let del = ui.add_enabled(can_del, egui::Button::new(icons::MINUS)).on_hover_text("Delete Record (Ctrl+Delete)").clicked();
        if del && !selected_rows.is_empty() {
            let loaded = page.result.rows.len();
            for r in selected_rows.into_iter().filter(|r| *r < loaded) {
                tab.deleted.insert(r);
            }
        } else if del && let Some((r, _)) = tab.selected {
            let loaded = page.result.rows.len();
            if r >= loaded {
                if r - loaded < tab.inserted.len() {
                    tab.inserted.remove(r - loaded);
                }
                tab.selected = None;
            } else if !tab.deleted.remove(&r) {
                tab.deleted.insert(r);
            }
        }
        let apply = ui
            .add_enabled(dirty, egui::Button::new(RichText::new(icons::CHECK).color(if dirty { Tokens::SUCCESS } else { Tokens::TEXT_WEAK })))
            .on_hover_text("Apply Changes (Ctrl+S)");
        if apply.clicked() || (dirty && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S))) {
            tab.editing = None;
            actions.push(Action::ApplyTable(idx));
        }
        if ui
            .add_enabled(dirty, egui::Button::new(RichText::new(icons::X).color(if dirty { Tokens::ERROR } else { Tokens::TEXT_WEAK })))
            .on_hover_text("Discard Changes")
            .clicked()
        {
            tab.discard();
            tab.error = None;
        }
        if ui.add_enabled(!dirty, egui::Button::new(icons::ARROWS_CLOCKWISE)).on_hover_text("Refresh").clicked() {
            actions.push(Action::ReloadTable(idx));
        }
        ui.separator();
        ui.add(egui::Label::new(RichText::new(&page.sql).color(Tokens::TEXT_WEAK).small()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let limit = tab.limit.max(1);
            let last_offset = page.total.saturating_sub(1) / limit * limit;
            let shown_to = (tab.offset + page.result.rows.len() as u64).min(page.total);
            let off = tab.offset;
            let mut target = None;
            let mut go = |ui: &mut Ui, label: &str, enabled: bool, to: u64, tip: &str| {
                if ui.add_enabled(enabled && !dirty, egui::Button::new(label)).on_hover_text(tip).clicked() {
                    target = Some(to);
                }
            };
            go(ui, icons::CARET_DOUBLE_RIGHT, off < last_offset, last_offset, "Last Page");
            go(ui, icons::CARET_RIGHT, off < last_offset, off + limit, "Next Page");
            ui.label(format!("{}", off / limit + 1));
            go(ui, icons::CARET_LEFT, off > 0, off.saturating_sub(limit), "Previous Page");
            go(ui, icons::CARET_DOUBLE_LEFT, off > 0, 0, "First Page");
            ui.label(if page.total == 0 { "No records".to_string() } else { format!("Records {}–{} of {}", off + 1, shown_to, page.total) });
            if let Some(t) = target {
                tab.offset = t;
                actions.push(Action::ReloadTable(idx));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcraft_core::{Column as Col, ResultSet};
    use dbcraft_engine::TablePage;

    fn tab(pk: &[&str]) -> TableTab {
        let rs = ResultSet {
            columns: vec![Col { name: "id".into(), type_name: "INT".into() }, Col { name: "name".into(), type_name: "TEXT".into() }],
            rows: vec![vec![Value::Int(1), Value::Text("a".into())], vec![Value::Int(2), Value::Null]],
            ..ResultSet::default()
        };
        TableTab {
            connection: "c".into(),
            table: "t".into(),
            page: Some(TablePage { result: rs, total: 2, primary_key: pk.iter().map(|s| s.to_string()).collect(), ..TablePage::default() }),
            ..TableTab::default()
        }
    }

    #[test]
    fn builds_changes_keyed_by_primary_key() {
        let mut t = tab(&["id"]);
        t.edits.insert((0, 1), Some("b".into()));
        t.edits.insert((1, 1), None);
        t.deleted.insert(1); // deleted wins over edits on the same row
        t.inserted.push([(1, Some("new".into()))].into_iter().collect());
        let p = edit_params(&t).unwrap();
        let ch = p["changes"].as_array().unwrap();
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[0], json!({"op":"update","key":{"id":{"t":"Int","v":1}},"set":{"name":"b"}}));
        assert_eq!(ch[1], json!({"op":"delete","key":{"id":{"t":"Int","v":2}}}));
        assert_eq!(ch[2], json!({"op":"insert","values":{"name":"new"}}));
    }

    #[test]
    fn without_primary_key_every_column_is_the_key() {
        let mut t = tab(&[]);
        t.edits.insert((1, 0), Some("5".into()));
        let p = edit_params(&t).unwrap();
        assert_eq!(p["changes"][0]["key"], json!({"id":{"t":"Int","v":2},"name":{"t":"Null"}}));
    }

    #[test]
    fn only_enabled_complete_filters_are_sent() {
        let mut t = tab(&["id"]);
        t.applied_filters = vec![
            FilterRow { enabled: true, column: "name".into(), op: "contains".into(), value: "x".into() },
            FilterRow { enabled: false, column: "id".into(), op: "=".into(), value: "1".into() },
            FilterRow { enabled: true, column: "name".into(), op: "is_null".into(), value: "ignored".into() },
        ];
        let p = data_params(&t);
        assert_eq!(p["filters"], json!([{"column":"name","op":"contains","value":"x"},{"column":"name","op":"is_null"}]));
    }
}
