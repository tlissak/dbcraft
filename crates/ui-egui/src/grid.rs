//! Grid selection and the read-only result grid (Navicat's Result tabs).
//!
//! Selection works like Navicat's grid: click a cell; Shift+click extends a block; click a row
//! number to select whole rows (Ctrl toggles, Shift extends); click a column header to select
//! columns; click the corner (or Ctrl+A) to select all. Ctrl+C copies the selection as
//! tab-separated text; the context menu also copies with headers or as INSERT statements.

use std::collections::BTreeSet;

use dbcraft_core::{ResultSet, Value};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::theme::Tokens;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridSel {
    #[default]
    None,
    /// A block from `anchor` to `focus` (inclusive).
    Cells {
        anchor: (usize, usize),
        focus: (usize, usize),
    },
    Rows {
        anchor: usize,
        rows: BTreeSet<usize>,
    },
    Cols {
        anchor: usize,
        cols: BTreeSet<usize>,
    },
    All,
}

impl GridSel {
    pub fn contains(&self, r: usize, c: usize) -> bool {
        match self {
            GridSel::None => false,
            GridSel::Cells { anchor, focus } => {
                (anchor.0.min(focus.0)..=anchor.0.max(focus.0)).contains(&r) && (anchor.1.min(focus.1)..=anchor.1.max(focus.1)).contains(&c)
            }
            GridSel::Rows { rows, .. } => rows.contains(&r),
            GridSel::Cols { cols, .. } => cols.contains(&c),
            GridSel::All => true,
        }
    }

    pub fn row_fully_selected(&self, r: usize) -> bool {
        matches!(self, GridSel::All) || matches!(self, GridSel::Rows { rows, .. } if rows.contains(&r))
    }

    pub fn col_fully_selected(&self, c: usize) -> bool {
        matches!(self, GridSel::All) || matches!(self, GridSel::Cols { cols, .. } if cols.contains(&c))
    }

    /// The cell with the keyboard/edit focus.
    pub fn focus(&self) -> Option<(usize, usize)> {
        match self {
            GridSel::Cells { focus, .. } => Some(*focus),
            GridSel::Rows { anchor, .. } => Some((*anchor, 0)),
            GridSel::Cols { anchor, .. } => Some((0, *anchor)),
            _ => None,
        }
    }

    pub fn click_cell(&mut self, r: usize, c: usize, m: egui::Modifiers) {
        *self = match (&*self, m.shift) {
            (GridSel::Cells { anchor, .. }, true) => GridSel::Cells { anchor: *anchor, focus: (r, c) },
            _ => GridSel::Cells { anchor: (r, c), focus: (r, c) },
        };
    }

    pub fn click_row(&mut self, r: usize, m: egui::Modifiers) {
        match self {
            GridSel::Rows { anchor, rows } if m.shift => {
                let (a, b) = ((*anchor).min(r), (*anchor).max(r));
                rows.extend(a..=b);
            }
            GridSel::Rows { anchor, rows } if m.command => {
                if !rows.remove(&r) {
                    rows.insert(r);
                }
                *anchor = r;
            }
            _ => *self = GridSel::Rows { anchor: r, rows: [r].into() },
        }
    }

    pub fn click_col(&mut self, c: usize, m: egui::Modifiers) {
        match self {
            GridSel::Cols { anchor, cols } if m.shift => {
                let (a, b) = ((*anchor).min(c), (*anchor).max(c));
                cols.extend(a..=b);
            }
            GridSel::Cols { anchor, cols } if m.command => {
                if !cols.remove(&c) {
                    cols.insert(c);
                }
                *anchor = c;
            }
            _ => *self = GridSel::Cols { anchor: c, cols: [c].into() },
        }
    }

    /// Selected rows and columns, in order (the bounding block for cell selections).
    pub fn span(&self, nrows: usize, ncols: usize) -> (Vec<usize>, Vec<usize>) {
        let all_r = || (0..nrows).collect::<Vec<_>>();
        let all_c = || (0..ncols).collect::<Vec<_>>();
        match self {
            GridSel::None => (Vec::new(), Vec::new()),
            GridSel::Cells { anchor, focus } => {
                let rows = (anchor.0.min(focus.0)..=anchor.0.max(focus.0)).filter(|r| *r < nrows).collect();
                let cols = (anchor.1.min(focus.1)..=anchor.1.max(focus.1)).filter(|c| *c < ncols).collect();
                (rows, cols)
            }
            GridSel::Rows { rows, .. } => (rows.iter().copied().filter(|r| *r < nrows).collect(), all_c()),
            GridSel::Cols { cols, .. } => (all_r(), cols.iter().copied().filter(|c| *c < ncols).collect()),
            GridSel::All => (all_r(), all_c()),
        }
    }
}

fn tsv_field(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        other => other.display().replace(['\t', '\n', '\r'], " "),
    }
}

/// The selection as tab-separated text (what Ctrl+C puts on the clipboard).
pub fn selection_tsv(rs: &ResultSet, sel: &GridSel, with_headers: bool) -> String {
    let (rows, cols) = sel.span(rs.rows.len(), rs.columns.len());
    let mut lines = Vec::new();
    if with_headers {
        lines.push(cols.iter().filter_map(|c| rs.columns.get(*c).map(|x| x.name.clone())).collect::<Vec<_>>().join("\t"));
    }
    for r in rows {
        let Some(row) = rs.rows.get(r) else { continue };
        lines.push(cols.iter().map(|c| row.get(*c).map(tsv_field).unwrap_or_default()).collect::<Vec<_>>().join("\t"));
    }
    lines.join("\r\n")
}

/// The selected rows as INSERT statements (Navicat: Copy As › Insert Statement).
pub fn selection_inserts(rs: &ResultSet, sel: &GridSel, dialect: Dialect, table: &str) -> String {
    let (rows, cols) = sel.span(rs.rows.len(), rs.columns.len());
    let names = cols.iter().filter_map(|c| rs.columns.get(*c)).map(|c| quote_ident(dialect, &c.name)).collect::<Vec<_>>().join(", ");
    rows.iter()
        .filter_map(|r| rs.rows.get(*r))
        .map(|row| {
            let vals = cols
                .iter()
                .map(|c| match row.get(*c) {
                    None | Some(Value::Null) => "NULL".to_string(),
                    Some(Value::Int(i)) => i.to_string(),
                    Some(Value::UInt(u)) => u.to_string(),
                    Some(Value::Float(f)) if f.is_finite() => f.to_string(),
                    Some(Value::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_string(),
                    Some(Value::Bytes(b)) => dbcraft_sql::bytes_literal(dialect, b),
                    Some(other) => quote_literal(dialect, &other.display()),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("INSERT INTO {} ({names}) VALUES ({vals});", quote_ident(dialect, table))
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Ctrl+A / Ctrl+C for a grid, when no text field has the keyboard. Returns true if handled.
pub fn keyboard(ui: &Ui, rs: &ResultSet, sel: &mut GridSel) -> bool {
    if ui.ctx().egui_wants_keyboard_input() {
        return false;
    }
    let (select_all, copy) = ui.input_mut(|i| {
        let a = i.consume_key(egui::Modifiers::COMMAND, egui::Key::A);
        let c = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
        (a, c)
    });
    if select_all {
        *sel = GridSel::All;
    }
    if copy && *sel != GridSel::None {
        ui.ctx().copy_text(selection_tsv(rs, sel, false));
        return true;
    }
    select_all
}

/// Paint the selection background of a cell (call first in the cell closure).
pub fn paint_cell(ui: &Ui, sel: &GridSel, r: usize, c: usize, base: Option<egui::Color32>) {
    let rect = ui.max_rect();
    if sel.contains(r, c) {
        ui.painter().rect_filled(rect, 0.0, Tokens::SELECTION);
    } else if let Some(bg) = base {
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    if sel.focus() == Some((r, c)) && matches!(sel, GridSel::Cells { .. }) {
        ui.painter().rect_stroke(rect.shrink(0.5), 0.0, egui::Stroke::new(1.5, Tokens::ACCENT), egui::StrokeKind::Inside);
    }
}

/// Row-number gutter cell: click selects the row (Ctrl/Shift extend).
pub fn row_header(ui: &mut Ui, label: &str, r: usize, sel: &mut GridSel) -> egui::Response {
    let rect = ui.max_rect();
    let full = sel.row_fully_selected(r);
    ui.painter().rect_filled(rect, 0.0, if full { Tokens::SELECTION_STROKE } else { Tokens::GRID_HEADER });
    let resp = ui.allocate_rect(rect, egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Row {label}")));
    ui.painter().text(rect.right_center() - egui::vec2(4.0, 0.0), egui::Align2::RIGHT_CENTER, label, egui::FontId::proportional(11.0), Tokens::TEXT_WEAK);
    if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        sel.click_row(r, m);
    }
    resp
}

/// Read-only result grid with Navicat-style selection and copy.
pub fn result_grid(ui: &mut Ui, id: egui::Id, rs: &ResultSet, sel: &mut GridSel, dialect: Dialect) {
    if rs.columns.is_empty() {
        ui.label(format!("Affected rows: {}   Time: {}ms", rs.rows_affected, rs.elapsed_ms));
        return;
    }
    keyboard(ui, rs, sel);
    let mut menu_action: Option<&'static str> = None;
    egui::ScrollArea::horizontal().id_salt(id.with("h")).show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt(id)
            .striped(true)
            .resizable(true)
            .min_scrolled_height(0.0)
            .column(Column::exact(46.0))
            .columns(Column::initial(120.0).at_least(40.0).clip(true), rs.columns.len())
            .header(22.0, |mut h| {
                h.col(|ui| {
                    // Corner: select all.
                    let rect = ui.max_rect();
                    ui.painter().rect_filled(rect, 0.0, Tokens::GRID_HEADER);
                    if ui.allocate_rect(rect, egui::Sense::click()).on_hover_text("Select All (Ctrl+A)").clicked() {
                        *sel = GridSel::All;
                    }
                });
                for (c, col) in rs.columns.iter().enumerate() {
                    h.col(|ui| {
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(rect, 0.0, if sel.col_fully_selected(c) { Tokens::SELECTION_STROKE } else { Tokens::GRID_HEADER });
                        let r = ui.add(egui::Label::new(RichText::new(&col.name).strong()).truncate().sense(egui::Sense::click()));
                        let r = r.on_hover_text(&col.type_name);
                        if r.clicked() {
                            let m = ui.input(|i| i.modifiers);
                            sel.click_col(c, m);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(20.0, rs.rows.len(), |mut row| {
                    let r = row.index();
                    let Some(cells) = rs.rows.get(r) else { return };
                    row.col(|ui| {
                        row_header(ui, &(r + 1).to_string(), r, sel);
                    });
                    for (c, v) in cells.iter().enumerate() {
                        row.col(|ui| {
                            paint_cell(ui, sel, r, c, None);
                            let text =
                                if matches!(v, Value::Null) { RichText::new("(NULL)").color(Tokens::TEXT_WEAK).italics() } else { RichText::new(v.display()) };
                            let resp = ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click()));
                            if resp.clicked() {
                                let m = ui.input(|i| i.modifiers);
                                sel.click_cell(r, c, m);
                            }
                            if resp.secondary_clicked() && !sel.contains(r, c) {
                                sel.click_cell(r, c, egui::Modifiers::NONE);
                            }
                            resp.context_menu(|ui| {
                                for (label, act) in [("Copy", "copy"), ("Copy with Headers", "copy_headers"), ("Copy as INSERT Statements", "copy_insert")] {
                                    if ui.button(label).clicked() {
                                        menu_action = Some(act);
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui.button("Select All").clicked() {
                                    menu_action = Some("select_all");
                                    ui.close();
                                }
                            });
                        });
                    }
                });
            });
    });
    match menu_action {
        Some("copy") => ui.ctx().copy_text(selection_tsv(rs, sel, false)),
        Some("copy_headers") => ui.ctx().copy_text(selection_tsv(rs, sel, true)),
        Some("copy_insert") => ui.ctx().copy_text(selection_inserts(rs, sel, dialect, "result")),
        Some("select_all") => *sel = GridSel::All,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcraft_core::Column as Col;

    fn rs() -> ResultSet {
        ResultSet {
            columns: ["a", "b", "c"].iter().map(|n| Col { name: (*n).into(), type_name: "T".into() }).collect(),
            rows: (0..4).map(|r| (0..3).map(|c| if r == 1 && c == 1 { Value::Null } else { Value::Int((r * 10 + c) as i64) }).collect()).collect(),
            ..ResultSet::default()
        }
    }

    #[test]
    fn selections_and_copy() {
        let rs = rs();
        let mut sel = GridSel::None;
        sel.click_cell(0, 1, egui::Modifiers::NONE);
        sel.click_cell(1, 2, egui::Modifiers::SHIFT);
        assert!(sel.contains(1, 1) && !sel.contains(2, 1));
        assert_eq!(selection_tsv(&rs, &sel, true), "b\tc\r\n1\t2\r\n\t12");

        sel.click_row(2, egui::Modifiers::NONE);
        sel.click_row(3, egui::Modifiers::SHIFT);
        assert_eq!(selection_tsv(&rs, &sel, false), "20\t21\t22\r\n30\t31\t32");
        sel.click_row(2, egui::Modifiers::COMMAND);
        assert_eq!(sel.span(4, 3).0, [3]);

        sel.click_col(2, egui::Modifiers::NONE);
        assert_eq!(selection_tsv(&rs, &sel, false), "2\r\n12\r\n22\r\n32");
        assert!(sel.col_fully_selected(2));

        let all = GridSel::All;
        assert_eq!(
            selection_inserts(&rs, &GridSel::Rows { anchor: 1, rows: [1].into() }, Dialect::Mysql, "t"),
            "INSERT INTO `t` (`a`, `b`, `c`) VALUES (10, NULL, 12);"
        );
        assert_eq!(all.span(4, 3), ((0..4).collect(), (0..3).collect()));
        // Out-of-range selections are clipped, never indexed.
        assert_eq!(GridSel::Cells { anchor: (9, 9), focus: (7, 7) }.span(4, 3), (vec![], vec![]));
    }
}
