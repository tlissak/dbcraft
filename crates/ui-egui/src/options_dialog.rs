//! Tools › Options (Navicat: a category list on the left, its settings on the right). OK saves
//! through the `tools.options` command; nothing changes until then.

use dbcraft_model::Options;
use egui::RichText;
use serde::{Deserialize, Serialize};

use crate::theme::Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OptionsPage {
    #[default]
    Appearance,
    Tabs,
    Editor,
    Records,
    Display,
}

const PAGES: [(OptionsPage, &str); 5] = [
    (OptionsPage::Appearance, "Appearance"),
    (OptionsPage::Tabs, "Tabs"),
    (OptionsPage::Editor, "Editor"),
    (OptionsPage::Records, "Records"),
    (OptionsPage::Display, "Display Formats"),
];

/// The dialog's working copy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptionsForm {
    pub options: Options,
    pub page: OptionsPage,
    pub error: Option<String>,
}

impl OptionsForm {
    pub fn new(options: Options) -> Self {
        OptionsForm { options, page: OptionsPage::default(), error: None }
    }
}

pub enum Outcome {
    Open,
    Cancel,
    Save(Options),
}

pub fn show(ctx: &egui::Context, form: &mut OptionsForm) -> Outcome {
    let mut out = Outcome::Open;
    let mut open = true;
    egui::Window::new("Options")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        // Fixed, like Navicat's: an auto-sized window with a vertical separator grows each frame.
        .fixed_size([600.0, 340.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(140.0);
                    for (page, label) in PAGES {
                        if ui.add(egui::Button::selectable(form.page == page, label).min_size(egui::vec2(140.0, 24.0))).clicked() {
                            form.page = page;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_min_size(egui::vec2(420.0, 250.0));
                    page(ui, form);
                });
            });
            ui.separator();
            if let Some(e) = &form.error {
                ui.colored_label(Tokens::error(), e);
            }
            ui.horizontal(|ui| {
                if ui.button("Reset to Defaults").clicked() {
                    form.options = Options::default();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("Cancel").min_size(egui::vec2(70.0, 0.0))).clicked() {
                        out = Outcome::Cancel;
                    }
                    if ui.add(egui::Button::new(RichText::new("OK").strong()).min_size(egui::vec2(70.0, 0.0))).clicked() {
                        out = Outcome::Save(form.options.clone());
                    }
                });
            });
        });
    if !open {
        out = Outcome::Cancel;
    }
    out
}

fn page(ui: &mut egui::Ui, form: &mut OptionsForm) {
    let o = &mut form.options;
    match form.page {
        OptionsPage::Appearance => {
            ui.strong("Theme");
            ui.horizontal(|ui| {
                ui.radio_value(&mut o.theme, "light".to_string(), "Light");
                ui.radio_value(&mut o.theme, "dark".to_string(), "Dark");
            });
            ui.add_space(8.0);
            ui.strong("Toolbar");
            ui.checkbox(&mut o.big_icons, "Use Big Icons");
            ui.checkbox(&mut o.show_caption, "Show Caption");
        }
        OptionsPage::Tabs => {
            ui.checkbox(&mut o.confirm_close, "Ask to save changes when closing a query");
        }
        OptionsPage::Editor => {
            ui.horizontal(|ui| {
                ui.label("Font size");
                ui.add(egui::DragValue::new(&mut o.editor_font_size).range(8.0..=40.0).speed(0.25).suffix(" pt"));
            });
            ui.checkbox(&mut o.word_wrap, "Word Wrap");
        }
        OptionsPage::Records => {
            ui.horizontal(|ui| {
                ui.checkbox(&mut o.limit_records, "Limit records");
                ui.add_enabled(o.limit_records, egui::DragValue::new(&mut o.page_size).range(1..=100_000));
                ui.label("records per page");
            });
            ui.weak("Applies to tables opened from now on. Without a limit, up to 100,000 rows load at once.");
        }
        OptionsPage::Display => {
            egui::Grid::new("display_formats").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("NULL shows as");
                ui.text_edit_singleline(&mut o.null_text);
                ui.end_row();
                for (label, value, hint) in [
                    ("Date", &mut o.date_format, "%Y-%m-%d"),
                    ("Time", &mut o.time_format, "%H:%M:%S"),
                    ("Date Time", &mut o.datetime_format, "%Y-%m-%d %H:%M:%S"),
                ] {
                    ui.label(label);
                    ui.add(egui::TextEdit::singleline(value).hint_text(hint));
                    ui.end_row();
                }
            });
            ui.weak("strftime codes (%Y year, %m month, %d day, %H hour, %M minute, %S second). Empty: as the server returns it.");
        }
    }
}
