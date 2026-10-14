//! Icons: Phosphor glyphs (MIT) tinted per object kind and engine. Closed connections and
//! databases are grey; open ones take their colour (Navicat's open/closed cue).

use dbcraft_core::DbKind;
use dbcraft_model::ObjectKind;
use egui::{Color32, RichText};
use egui_phosphor::regular as ph;

use crate::theme::Tokens;

pub fn engine_color(kind: DbKind) -> Color32 {
    match kind {
        DbKind::Mysql => Color32::from_rgb(0x00, 0x75, 0x8f),
        DbKind::Mariadb => Color32::from_rgb(0xa0, 0x5a, 0x2c),
        DbKind::Postgres => Color32::from_rgb(0x33, 0x67, 0x91),
        DbKind::Sqlite => Color32::from_rgb(0x0f, 0x80, 0xcc),
        DbKind::Sqlserver => Color32::from_rgb(0xcc, 0x29, 0x27),
        DbKind::Oracle => Color32::from_rgb(0xe0, 0x1f, 0x1f),
        DbKind::Mongodb => Color32::from_rgb(0x13, 0xaa, 0x52),
        DbKind::Redis => Color32::from_rgb(0xdc, 0x38, 0x2d),
    }
}

/// Connection node icon: a database glyph in the engine colour when open, grey when closed.
pub fn connection_glyph(kind: DbKind, open: bool) -> (&'static str, Color32) {
    if open { (ph::PLUGS_CONNECTED, engine_color(kind)) } else { (ph::DATABASE, Tokens::CLOSED) }
}

pub fn connection(kind: DbKind, open: bool) -> RichText {
    let (g, c) = connection_glyph(kind, open);
    RichText::new(g).size(15.0).color(c)
}

/// Database node: green once its objects are loaded, grey before.
pub fn database_glyph(open: bool) -> (&'static str, Color32) {
    (ph::DATABASE, if open { Color32::from_rgb(0x2e, 0x9e, 0x5b) } else { Tokens::CLOSED })
}

pub fn object(kind: ObjectKind) -> RichText {
    let (g, c) = object_glyph(kind);
    RichText::new(g).color(c)
}

pub fn object_glyph(kind: ObjectKind) -> (&'static str, Color32) {
    match kind {
        ObjectKind::Table => (ph::TABLE, Color32::from_rgb(0x1e, 0x6f, 0xd9)),
        ObjectKind::View => (ph::EYE, Color32::from_rgb(0x8e, 0x44, 0xd6)),
        ObjectKind::Function => (ph::FUNCTION, Color32::from_rgb(0xe6, 0x7e, 0x22)),
        ObjectKind::Procedure => (ph::CODE, Color32::from_rgb(0xe6, 0x7e, 0x22)),
        ObjectKind::Trigger => (ph::LIGHTNING, Color32::from_rgb(0xd6, 0x45, 0x45)),
        ObjectKind::Event => (ph::CALENDAR_BLANK, Color32::from_rgb(0x27, 0xae, 0x60)),
        ObjectKind::Index => (ph::LIST_NUMBERS, Color32::from_rgb(0x29, 0x9e, 0xc4)),
        ObjectKind::Sequence => (ph::HASH, Color32::from_rgb(0x7f, 0x8c, 0x8d)),
        ObjectKind::Database | ObjectKind::Schema => (ph::FOLDER, Color32::from_rgb(0xd4, 0xa0, 0x17)),
    }
}

/// Toolbar buttons (Navicat's main toolbar), each with its own tint.
pub const TOOLBAR: &[(&str, &str, Color32, &str)] = &[
    (ph::PLUG, "Connection", Color32::from_rgb(0x1e, 0x88, 0x3c), "dialog.newConnection"),
    (ph::FILE_SQL, "New Query", Color32::from_rgb(0x1e, 0x6f, 0xd9), "ui.newQuery"),
    (ph::TABLE, "Table", Color32::from_rgb(0x1e, 0x6f, 0xd9), "ui.objects.table"),
    (ph::EYE, "View", Color32::from_rgb(0x8e, 0x44, 0xd6), "ui.objects.view"),
    (ph::FUNCTION, "Function", Color32::from_rgb(0xe6, 0x7e, 0x22), ""),
    (ph::USER, "User", Color32::from_rgb(0x55, 0x6b, 0x82), "ui.objects.users"),
    (ph::DOTS_THREE_CIRCLE, "Others", Color32::from_rgb(0x55, 0x6b, 0x82), ""),
    (ph::MAGNIFYING_GLASS, "Query", Color32::from_rgb(0x1e, 0x6f, 0xd9), "ui.objects.queries"),
    (ph::ARCHIVE, "Backup", Color32::from_rgb(0x27, 0xae, 0x60), "ui.objects.backups"),
    (ph::ROBOT, "Automation", Color32::from_rgb(0xd4, 0x8a, 0x17), "ui.automation"),
    (ph::TREE_STRUCTURE, "Model", Color32::from_rgb(0x16, 0xa0, 0x85), ""),
    (ph::CHART_BAR, "Charts", Color32::from_rgb(0xc0, 0x39, 0x2b), ""),
];

pub use ph::{
    ARROWS_CLOCKWISE, CARET_DOUBLE_LEFT, CARET_DOUBLE_RIGHT, CARET_LEFT, CARET_RIGHT, CHECK, CODE, DOWNLOAD_SIMPLE, FILE_SQL, FUNNEL, INFO, KEY, LOCK_KEY,
    MAGNIFYING_GLASS, MINUS, PENCIL_SIMPLE, PLAY, PLUG, PLUS, SHIELD_CHECK, SORT_ASCENDING, SORT_DESCENDING, TABLE, TRASH, UPLOAD_SIMPLE, WRENCH, X,
};
