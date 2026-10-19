//! The workspace model: what Navicat shows in its Connection pane and tab strip, as pure data.
//! Serializable so the control channel and MCP can read it, and so it persists between runs.

use dbcraft_core::{ConnectionConfig, Error, Result};
use serde::{Deserialize, Serialize};

/// Kinds of objects in the navigation tree (Navicat's Objects toolbar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Database,
    Schema,
    Table,
    View,
    Function,
    Procedure,
    Trigger,
    Event,
    Index,
    Sequence,
}

/// A database object as listed by a driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbObject {
    pub kind: ObjectKind,
    pub name: String,
    /// Owning schema (PostgreSQL/SQL Server) or database (MySQL), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// A saved query (Navicat: Queries under a database).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub connection: String,
    #[serde(default)]
    pub database: Option<String>,
    pub sql: String,
    /// Unix seconds of the last save.
    #[serde(default)]
    pub modified: u64,
}

/// One step of a batch job: an engine command and its params.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStep {
    pub command: String,
    #[serde(default)]
    pub params: serde_json::Value,
    /// Shown in the job list ("Backup sakila_lite on PostgreSQL").
    #[serde(default)]
    pub label: String,
}

/// When a batch job runs by itself (registered with the OS scheduler).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// `once`, `hourly`, `daily` or `weekly`.
    pub frequency: String,
    /// `HH:MM`, local time.
    pub time: String,
    /// Weekly: `MON`…`SUN`.
    #[serde(default)]
    pub day: Option<String>,
}

/// Navicat Automation batch job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub steps: Vec<JobStep>,
    #[serde(default = "yes")]
    pub continue_on_error: bool,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// Summary of the last run ("2026-10-14 02:00 ok, 2 steps").
    #[serde(default)]
    pub last_run: Option<String>,
}

fn yes() -> bool {
    true
}

/// Where a workspace keeps saved passwords.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PasswordStorage {
    /// In `workspace.json` (dev/test workspaces, or no keychain on this machine).
    #[default]
    File,
    /// In the OS keychain; the file holds no passwords.
    Keychain,
}

impl PasswordStorage {
    fn is_file(&self) -> bool {
        *self == PasswordStorage::File
    }
}

/// Tools › Options. Every field has a default, so older workspace files load unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// Appearance: `light` or `dark`.
    pub theme: String,
    /// Toolbar: large icons (Navicat's "Use Big Icons") and labels under them ("Show Caption").
    pub big_icons: bool,
    pub show_caption: bool,
    /// Editor: font size in points, and soft-wrap long lines.
    pub editor_font_size: f32,
    pub word_wrap: bool,
    /// Records: rows per page in the table viewer; off loads up to the engine's maximum.
    pub limit_records: bool,
    pub page_size: u64,
    /// Tabs: ask before closing a query with unsaved changes.
    pub confirm_close: bool,
    /// Display formats: how NULL reads, and strftime formats for dates and times (empty: as the
    /// server returns them).
    pub null_text: String,
    pub date_format: String,
    pub time_format: String,
    pub datetime_format: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            theme: "light".into(),
            big_icons: true,
            show_caption: true,
            editor_font_size: 13.0,
            word_wrap: false,
            limit_records: true,
            page_size: 1000,
            confirm_close: true,
            null_text: "(NULL)".into(),
            date_format: String::new(),
            time_format: String::new(),
            datetime_format: String::new(),
        }
    }
}

impl Options {
    fn is_default(&self) -> bool {
        *self == Options::default()
    }
}

/// Saved state: connections, queries, batch jobs. Persisted as `workspace.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    /// Names this workspace's keychain entries (set when passwords move to the keychain).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "PasswordStorage::is_file")]
    pub password_storage: PasswordStorage,
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
    #[serde(default)]
    pub queries: Vec<SavedQuery>,
    #[serde(default)]
    pub automations: Vec<BatchJob>,
    #[serde(default, skip_serializing_if = "Options::is_default")]
    pub options: Options,
}

/// A fresh id like `{prefix}-3` not in `taken`.
pub fn next_id<'a>(prefix: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    (1..).map(|n| format!("{prefix}-{n}")).find(|id| taken.clone().all(|t| t != id)).unwrap_or_default()
}

impl Workspace {
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| Error::InvalidParam(format!("workspace file: {e}")))
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| Error::InvalidParam(e.to_string()))
    }

    pub fn connection(&self, id: &str) -> Result<&ConnectionConfig> {
        self.connections.iter().find(|c| c.id == id).ok_or_else(|| Error::NotFound(format!("connection `{id}`")))
    }

    /// Add or replace (by id).
    pub fn upsert(&mut self, cfg: ConnectionConfig) {
        match self.connections.iter_mut().find(|c| c.id == cfg.id) {
            Some(slot) => *slot = cfg,
            None => self.connections.push(cfg),
        }
    }

    pub fn remove(&mut self, id: &str) -> Result<ConnectionConfig> {
        let pos = self.connections.iter().position(|c| c.id == id).ok_or_else(|| Error::NotFound(format!("connection `{id}`")))?;
        Ok(self.connections.remove(pos))
    }

    /// A fresh id like `conn-3` that isn't taken.
    pub fn next_id(&self) -> String {
        (1..).map(|n| format!("conn-{n}")).find(|id| self.connections.iter().all(|c| &c.id != id)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_by_id_and_remove_errors_when_missing() {
        let mut ws = Workspace::default();
        ws.upsert(ConnectionConfig::sqlite("a", "A", "a.db"));
        ws.upsert(ConnectionConfig::sqlite("a", "A2", "a.db"));
        assert_eq!(ws.connections.len(), 1);
        assert_eq!(ws.connection("a").unwrap().name, "A2");
        assert!(ws.remove("zzz").is_err());
        assert_eq!(ws.next_id(), "conn-1");
    }

    #[test]
    fn corrupt_workspace_file_is_an_error_not_a_panic() {
        assert!(Workspace::from_json("{not json").is_err());
        assert_eq!(Workspace::from_json("{}").unwrap(), Workspace::default());
    }
}
