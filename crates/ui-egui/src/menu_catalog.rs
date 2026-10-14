//! Navicat Premium's menu bar, item by item, with the engine command id each item maps to.
//! Written from observing the installed app (clean-room: names and structure only).
//!
//! An item is **live** when its id is registered in `dbcraft_engine::commands()` (or is a shell
//! command in `app.rs`); otherwise it shows disabled. To ship a menu item, register a command with
//! the exact id below. `cargo xtask parity` counts live / total.

pub struct MenuItem {
    pub path: &'static str,
    pub id: &'static str,
}

const fn m(path: &'static str, id: &'static str) -> MenuItem {
    MenuItem { path, id }
}

pub const MENUS: &[&str] = &["File", "Edit", "View", "Favorites", "Tools", "Window", "Help"];

pub const CATALOG: &[MenuItem] = &[
    // File
    m("File/New Connection", "connection.save"),
    m("File/New Query", "ui.newQuery"),
    m("File/Save", "query.save"),
    m("File/Open Connection", "connection.open"),
    m("File/Close Connection", "connection.close"),
    m("File/Delete Connection", "connection.delete"),
    m("File/Import Connections", "connection.import"),
    m("File/Export Connections", "connection.export"),
    m("File/Open External File", "file.openExternal"),
    m("File/Recent Queries", "file.recentQueries"),
    m("File/Exit", "ui.exit"),
    // Edit
    m("Edit/Undo", "edit.undo"),
    m("Edit/Redo", "edit.redo"),
    m("Edit/Cut", "edit.cut"),
    m("Edit/Copy", "edit.copy"),
    m("Edit/Paste", "edit.paste"),
    m("Edit/Select All", "edit.selectAll"),
    m("Edit/Find", "edit.find"),
    // View
    m("View/Navigation Pane", "ui.toggleNavigation"),
    m("View/Information Pane", "ui.toggleInfo"),
    m("View/Refresh", "database.objects"),
    m("View/Show Hidden Items", "view.showHidden"),
    // Favorites
    m("Favorites/Add to Favorites", "favorites.add"),
    m("Favorites/Manage Favorites", "favorites.manage"),
    // Tools
    m("Tools/Data Transfer", "tools.dataTransfer"),
    m("Tools/Data Synchronization", "tools.dataSync"),
    m("Tools/Structure Synchronization", "tools.structureSync"),
    m("Tools/Data Generation", "tools.dataGeneration"),
    m("Tools/Import Wizard", "tools.importWizard"),
    m("Tools/Export Wizard", "export.resultCsv"),
    m("Tools/Backup", "backup.create"),
    m("Tools/Automation", "automation.save"),
    m("Tools/Console", "ui.console"),
    m("Tools/User Manager", "user.list"),
    m("Tools/Model", "tools.model"),
    m("Tools/Charts", "tools.charts"),
    m("Tools/Server Monitor", "tools.serverMonitor"),
    m("Tools/History Log", "ui.toggleMessages"),
    m("Tools/Options", "tools.options"),
    // Window
    m("Window/Close All Tabs", "ui.closeAllTabs"),
    // Help
    m("Help/About DbCraft", "ui.about"),
];
