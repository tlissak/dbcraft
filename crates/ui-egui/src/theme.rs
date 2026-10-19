//! Look and feel. A light theme in the spirit of Navicat Premium on Windows (white panes, a pale
//! grey toolbar, Windows-blue selection, Segoe UI) and a dark theme. Colours live here, never in
//! drawing code.
//! Clean-room: colours were picked by eye; no Navicat assets are used. Icons are Phosphor (MIT).

use std::cell::Cell;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// One theme's colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub text: Color32,
    pub text_weak: Color32,
    pub bg: Color32,
    pub toolbar_bg: Color32,
    pub panel_bg: Color32,
    pub border: Color32,
    pub selection: Color32,
    pub selection_stroke: Color32,
    pub hover: Color32,
    pub accent: Color32,
    pub stripe: Color32,
    pub grid_header: Color32,
    pub tab_inactive: Color32,
    /// Edited cell, inserted row, deleted row (data grid).
    pub edited: Color32,
    pub inserted: Color32,
    pub deleted: Color32,
    pub error: Color32,
    pub success: Color32,
    pub closed: Color32,
    pub dark: bool,
}

pub const LIGHT: Palette = Palette {
    text: Color32::from_rgb(0x1f, 0x1f, 0x1f),
    text_weak: Color32::from_rgb(0x70, 0x70, 0x70),
    bg: Color32::WHITE,
    toolbar_bg: Color32::from_rgb(0xf5, 0xf6, 0xf7),
    panel_bg: Color32::from_rgb(0xfa, 0xfa, 0xfb),
    border: Color32::from_rgb(0xdc, 0xdd, 0xdf),
    selection: Color32::from_rgb(0xcc, 0xe4, 0xfb),
    selection_stroke: Color32::from_rgb(0x7a, 0xb8, 0xf0),
    hover: Color32::from_rgb(0xe8, 0xf2, 0xfd),
    accent: Color32::from_rgb(0x1e, 0x6f, 0xd9),
    stripe: Color32::from_rgb(0xf6, 0xf8, 0xfa),
    grid_header: Color32::from_rgb(0xef, 0xf1, 0xf3),
    tab_inactive: Color32::from_rgb(0xec, 0xed, 0xef),
    edited: Color32::from_rgb(0xff, 0xf4, 0xc2),
    inserted: Color32::from_rgb(0xdc, 0xf5, 0xdc),
    deleted: Color32::from_rgb(0xfb, 0xdc, 0xdc),
    error: Color32::from_rgb(0xc6, 0x28, 0x28),
    success: Color32::from_rgb(0x2e, 0x7d, 0x32),
    closed: Color32::from_rgb(0xa0, 0xa4, 0xa8),
    dark: false,
};

/// Dark theme: charcoal panes, the same blue accent lifted for contrast.
pub const DARK: Palette = Palette {
    text: Color32::from_rgb(0xe3, 0xe4, 0xe6),
    text_weak: Color32::from_rgb(0x9a, 0x9d, 0xa1),
    bg: Color32::from_rgb(0x1f, 0x20, 0x23),
    toolbar_bg: Color32::from_rgb(0x2b, 0x2d, 0x30),
    panel_bg: Color32::from_rgb(0x25, 0x27, 0x2a),
    border: Color32::from_rgb(0x3c, 0x3f, 0x43),
    selection: Color32::from_rgb(0x23, 0x4a, 0x74),
    selection_stroke: Color32::from_rgb(0x3d, 0x7a, 0xc0),
    hover: Color32::from_rgb(0x2f, 0x3a, 0x47),
    accent: Color32::from_rgb(0x4c, 0x9b, 0xf0),
    stripe: Color32::from_rgb(0x23, 0x25, 0x28),
    grid_header: Color32::from_rgb(0x2d, 0x2f, 0x33),
    tab_inactive: Color32::from_rgb(0x2a, 0x2c, 0x2f),
    edited: Color32::from_rgb(0x5a, 0x4a, 0x12),
    inserted: Color32::from_rgb(0x1f, 0x4a, 0x26),
    deleted: Color32::from_rgb(0x5a, 0x25, 0x25),
    error: Color32::from_rgb(0xef, 0x6b, 0x6b),
    success: Color32::from_rgb(0x5c, 0xc4, 0x6a),
    closed: Color32::from_rgb(0x6f, 0x73, 0x78),
    dark: true,
};

thread_local! {
    // egui draws on one thread; per-thread keeps parallel UI tests independent.
    static CURRENT: Cell<Palette> = const { Cell::new(LIGHT) };
}

/// The colours drawing code uses, from the current theme.
pub struct Tokens;

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        impl Tokens {
            $(
                pub fn $name() -> Color32 {
                    CURRENT.with(|c| c.get().$name)
                }
            )*
        }
    };
}

tokens!(
    text,
    text_weak,
    bg,
    toolbar_bg,
    panel_bg,
    border,
    selection,
    selection_stroke,
    hover,
    accent,
    stripe,
    grid_header,
    tab_inactive,
    edited,
    inserted,
    deleted,
    error,
    success,
    closed
);

pub fn palette() -> Palette {
    CURRENT.with(Cell::get)
}

/// Switch between the light and dark theme (Tools › Options › Appearance).
pub fn set_dark(ctx: &egui::Context, dark: bool) {
    CURRENT.with(|c| c.set(if dark { DARK } else { LIGHT }));
    ctx.set_visuals(visuals());
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
    let p = palette();
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(p.text);
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.stripe;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    let r = CornerRadius::same(3);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = p.bg;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.selection_stroke);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.weak_bg_fill = p.selection;
    v.widgets.active.bg_fill = p.selection;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.corner_radius = r;
    v.widgets.open.weak_bg_fill = p.selection;
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
