//! The egui shell. Thin: it draws the session and turns clicks into engine commands. Only pure
//! view state (panels, tabs, dialogs) lives here, in [`state::UiState`].

pub(crate) mod app;
mod automation_tab;
mod conn_dialog;
mod console_tab;
pub mod control;
pub mod display;
pub mod grid;
pub mod icons;
pub mod jobs;
pub mod menu_catalog;
mod objects_modes;
mod objects_tab;
pub mod options_dialog;
mod query_tab;
pub mod state;
mod table_tab;
pub mod tasks;
pub mod theme;
mod transfer_tab;
mod user_tab;

pub use app::{DbCraftApp, StartupAction};

/// Shell-only commands (view/window state, no engine involvement).
pub const UI_COMMANDS: &[&str] =
    &["ui.newQuery", "ui.console", "ui.exit", "ui.toggleNavigation", "ui.toggleInfo", "ui.toggleMessages", "ui.closeAllTabs", "ui.about", "ui.options"];

/// Ids of every registered engine command.
pub fn engine_command_ids() -> Vec<&'static str> {
    dbcraft_engine::commands().iter().map(|c| c.id).collect()
}

/// Menu items that do something today, out of the whole catalog.
pub fn parity() -> (usize, usize) {
    let engine = engine_command_ids();
    let live = menu_catalog::CATALOG.iter().filter(|m| engine.contains(&m.id) || UI_COMMANDS.contains(&m.id)).count();
    (live, menu_catalog::CATALOG.len())
}

/// Raise when parity rises; never lower it.
pub const PARITY_FLOOR: usize = 20;

#[cfg(test)]
mod tests {
    #[test]
    fn parity_does_not_regress() {
        let (live, total) = super::parity();
        assert!(live >= super::PARITY_FLOOR, "menu parity fell to {live}/{total}");
    }
}
