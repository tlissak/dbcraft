//! Look and feel. A light theme in the spirit of Navicat Premium on Windows: white panes, a pale
//! grey toolbar, Windows-blue selection, Segoe UI. Colours live here, never in drawing code.
//! Clean-room: colours were picked by eye; no Navicat assets are used. Icons are Phosphor (MIT).

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub struct Tokens;

impl Tokens {
    pub const TEXT: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
    pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x70, 0x70, 0x70);
    pub const BG: Color32 = Color32::WHITE;
    pub const TOOLBAR_BG: Color32 = Color32::from_rgb(0xf5, 0xf6, 0xf7);
    pub const PANEL_BG: Color32 = Color32::from_rgb(0xfa, 0xfa, 0xfb);
    pub const BORDER: Color32 = Color32::from_rgb(0xdc, 0xdd, 0xdf);
    pub const SELECTION: Color32 = Color32::from_rgb(0xcc, 0xe4, 0xfb);
    pub const SELECTION_STROKE: Color32 = Color32::from_rgb(0x7a, 0xb8, 0xf0);
    pub const HOVER: Color32 = Color32::from_rgb(0xe8, 0xf2, 0xfd);
    pub const ACCENT: Color32 = Color32::from_rgb(0x1e, 0x6f, 0xd9);
    pub const STRIPE: Color32 = Color32::from_rgb(0xf6, 0xf8, 0xfa);
    pub const GRID_HEADER: Color32 = Color32::from_rgb(0xef, 0xf1, 0xf3);
    pub const TAB_INACTIVE: Color32 = Color32::from_rgb(0xec, 0xed, 0xef);
    /// Edited cell, inserted row, deleted row (data grid).
    pub const EDITED: Color32 = Color32::from_rgb(0xff, 0xf4, 0xc2);
    pub const INSERTED: Color32 = Color32::from_rgb(0xdc, 0xf5, 0xdc);
    pub const DELETED: Color32 = Color32::from_rgb(0xfb, 0xdc, 0xdc);
    pub const ERROR: Color32 = Color32::from_rgb(0xc6, 0x28, 0x28);
    pub const SUCCESS: Color32 = Color32::from_rgb(0x2e, 0x7d, 0x32);
    pub const CLOSED: Color32 = Color32::from_rgb(0xa0, 0xa4, 0xa8);
}

/// Load Segoe UI / Consolas from the system when present (never committed to the repo), Phosphor
/// icons as fallback so icons mix into any label.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, path, family) in
        [("segoe", "C:\\Windows\\Fonts\\segoeui.ttf", FontFamily::Proportional), ("consolas", "C:\\Windows\\Fonts\\consola.ttf", FontFamily::Monospace)]
    {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), FontData::from_owned(bytes).into());
            if let Some(list) = fonts.families.get_mut(&family) {
                list.insert(0, name.into());
            }
        }
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_visuals(visuals());
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::proportional(17.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        // Labels are not text-selectable: no I-beam over grids and lists, and clicks reach the
        // row underneath (cell values are copied with Ctrl+C / the context menu instead).
        style.interaction.selectable_labels = false;
    });
}

fn visuals() -> Visuals {
    let mut v = Visuals::light();
    v.override_text_color = Some(Tokens::TEXT);
    v.panel_fill = Tokens::BG;
    v.window_fill = Tokens::BG;
    v.extreme_bg_color = Tokens::BG;
    v.faint_bg_color = Tokens::STRIPE;
    v.window_stroke = Stroke::new(1.0, Tokens::BORDER);
    v.selection.bg_fill = Tokens::SELECTION;
    v.selection.stroke = Stroke::new(1.0, Tokens::ACCENT);
    v.hyperlink_color = Tokens::ACCENT;
    v.error_fg_color = Tokens::ERROR;
    let r = CornerRadius::same(3);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Tokens::BORDER);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = Tokens::BG;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Tokens::BORDER);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.weak_bg_fill = Tokens::HOVER;
    v.widgets.hovered.bg_fill = Tokens::HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Tokens::SELECTION_STROKE);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.weak_bg_fill = Tokens::SELECTION;
    v.widgets.active.bg_fill = Tokens::SELECTION;
    v.widgets.active.bg_stroke = Stroke::new(1.0, Tokens::ACCENT);
    v.widgets.active.corner_radius = r;
    v.widgets.open.weak_bg_fill = Tokens::SELECTION;
    v.widgets.open.corner_radius = r;
    // No growth on hover/press: egui's default expands widgets by 1px, which made tree rows
    // look like they jump. Strokes are drawn inside the item instead.
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.expansion = 0.0;
    }
    v
}

/// Human-readable byte size, like Navicat's Data Length column.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.2} {}", UNITS.get(u).copied().unwrap_or("B")) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_sizes() {
        assert_eq!(super::bytes(0), "0 B");
        assert_eq!(super::bytes(16384), "16.00 KB");
        assert_eq!(super::bytes(u64::MAX), "16777216.00 TB");
    }
}
