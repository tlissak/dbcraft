use dbcraft_core::Result;
use serde_json::Value as Json;

use crate::Session;

/// One user-visible action. The id is the contract shared by the menus
/// (`ui-egui/src/menu_catalog.rs`), the CLI, the control channel and MCP.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    /// Dotted id, e.g. `connection.open`.
    pub id: &'static str,
    pub label: &'static str,
    /// Navicat menu path, `/`-separated (empty when the command has no menu item).
    pub menu: &'static str,
    pub shortcut: Option<&'static str>,
    /// JSON params, one line, for `--help`, MCP tool descriptions and agents.
    pub params_doc: &'static str,
    pub enabled: fn(&Session) -> bool,
    pub run: fn(&mut Session, &Json) -> Result<Json>,
}

pub fn always(_: &Session) -> bool {
    true
}

pub fn has_open(s: &Session) -> bool {
    s.any_open()
}

/// The full registry. Each area keeps its commands in its own `*_cmds.rs` module.
pub fn commands() -> Vec<CommandSpec> {
    let mut v = Vec::new();
    v.extend(crate::connection_cmds::specs());
    v.extend(crate::query_cmds::specs());
    v.extend(crate::data_cmds::specs());
    v.extend(crate::export_cmds::specs());
    v.extend(crate::saved_query_cmds::specs());
    v.extend(crate::transfer_cmds::specs());
    v.extend(crate::backup_cmds::specs());
    v.extend(crate::automation_cmds::specs());
    v.extend(crate::options_cmds::specs());
    v.extend(crate::user_cmds::specs());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_documented() {
        let cmds = commands();
        let mut ids: Vec<_> = cmds.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate command id");
        for c in &cmds {
            assert!(c.id.contains('.'), "{} should be area.action", c.id);
            assert!(!c.label.is_empty() && !c.params_doc.is_empty(), "{} needs a label and params_doc", c.id);
        }
    }
}
