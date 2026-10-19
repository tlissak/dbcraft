//! View state. Serializable so a control channel can read and drive it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use dbcraft_core::{ResultSet, UserInfo};
use dbcraft_engine::TablePage;
use dbcraft_model::BatchJob;

use crate::grid::GridSel;
use crate::jobs::Job;
use crate::tasks::Task;
use dbcraft_model::ObjectKind;
use serde::{Deserialize, Serialize};

/// What's selected in the navigation pane; the Objects tab lists its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub connection: String,
    pub database: Option<String>,
    /// Object group shown in the Objects tab (Tables, Views, …).
    pub group: ObjectKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryTab {
    pub title: String,
    pub connection: Option<String>,
    pub database: Option<String>,
    pub sql: String,
    /// Saved query this tab edits (`None`: never saved, "Untitled Query").
    pub saved_id: Option<String>,
    /// SQL as last saved, to tell whether there are unsaved changes.
    pub saved_sql: String,
    /// Index of the shown result tab; `results.len()` = the Message tab.
    pub result_idx: usize,
    pub results: Vec<ResultSet>,
    pub messages: Vec<String>,
    pub error: Option<String>,
    pub grid: GridSel,
    /// "Save As" name prompt (Some while open).
    pub save_prompt: Option<String>,
    /// Close the tab once the pending save succeeds.
    pub close_after_save: bool,
    /// The running query (Stop cancels it).
    #[serde(skip)]
    pub task: Option<Task>,
}

impl QueryTab {
    /// Unsaved changes: edited since the last save, or new and non-empty.
    pub fn dirty(&self) -> bool {
        self.sql != self.saved_sql && !(self.saved_id.is_none() && self.sql.trim().is_empty())
    }
}

/// Navicat's Console: a command-line session on one connection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsoleTab {
    pub connection: String,
    pub database: Option<String>,
    pub scrollback: String,
    pub input: String,
    /// Statement being continued over several lines (until `;`).
    pub pending: String,
    pub history: Vec<String>,
    /// Position while browsing history with Up/Down (`history.len()` = new line).
    pub history_pos: usize,
    /// The running statement (Stop cancels it).
    #[serde(skip)]
    pub task: Option<Task>,
}

/// Tools › Data Transfer form.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransferForm {
    pub source: Option<String>,
    pub source_db: Option<String>,
    /// `true`: write a SQL file instead of a connection.
    pub to_file: bool,
    pub target: Option<String>,
    pub target_db: Option<String>,
    pub file: String,
    pub dialect: String,
    /// (schema, table) → checked.
    pub tables: Vec<(Option<String>, String, bool)>,
    pub create_tables: bool,
    pub drop_existing: bool,
    pub create_records: bool,
    pub continue_on_error: bool,
    pub batch_size: String,
    #[serde(skip)]
    pub job: Job,
    pub status: Option<String>,
}

/// Tools › Automation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutomationView {
    /// Job being edited (a copy; saved with Save).
    pub editing: Option<BatchJob>,
    pub frequency: String,
    pub time: String,
    pub day: String,
    /// New step pickers.
    pub step_kind: String,
    pub step_connection: Option<String>,
    pub step_database: Option<String>,
    pub step_sql: String,
    #[serde(skip)]
    pub job: Job,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UserSubTab {
    #[default]
    General,
    MemberOf,
    ServerPrivileges,
    Privileges,
    SqlPreview,
}

/// User / Role designer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserTab {
    pub connection: String,
    /// The account as loaded (`None`: new).
    pub original: Option<UserInfo>,
    pub user: UserInfo,
    pub password: String,
    pub confirm: String,
    pub sub: UserSubTab,
    pub new_object: String,
    pub preview: Vec<String>,
    pub error: Option<String>,
    pub saved: bool,
}

impl UserTab {
    pub fn title(&self) -> String {
        let kind = if self.user.is_role { "Role" } else { "User" };
        if self.original.is_none() { format!("New {kind}") } else { format!("{} ({kind})", self.user.display_name()) }
    }
}

/// What the Objects tab lists (switched by the toolbar: Table/View, Query, Backup, User).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ObjectsMode {
    #[default]
    Objects,
    Queries,
    Backups,
    Users,
}

/// One filter row (Navicat's filter wizard).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FilterRow {
    pub enabled: bool,
    pub column: String,
    pub op: String,
    pub value: String,
}

/// A pending cell value: `None` means NULL.
pub type CellEdit = Option<String>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TableTab {
    pub connection: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: String,
    pub page: Option<TablePage>,
    pub offset: u64,
    pub limit: u64,
    pub show_filter: bool,
    pub filters: Vec<FilterRow>,
    pub match_any: bool,
    /// Filters that produced `page` (the filter bar can be edited without reloading).
    pub applied_filters: Vec<FilterRow>,
    pub sort: Option<(String, bool)>,
    /// (row, col) → new value, for loaded rows.
    pub edits: BTreeMap<(usize, usize), CellEdit>,
    pub deleted: BTreeSet<usize>,
    /// New rows: col → value (unset columns take the server default).
    pub inserted: Vec<BTreeMap<usize, CellEdit>>,
    /// Cell being edited: (row, col, text). Rows ≥ loaded count are inserted rows.
    pub editing: Option<(usize, usize, String)>,
    pub selected: Option<(usize, usize)>,
    /// Row/column/block selection (copy, delete records).
    pub grid: GridSel,
    pub error: Option<String>,
    /// The page being loaded (Stop cancels it); the last page stays on screen meanwhile.
    #[serde(skip)]
    pub task: Option<Task>,
}

impl TableTab {
    pub fn dirty(&self) -> bool {
        !self.edits.is_empty() || !self.deleted.is_empty() || !self.inserted.is_empty()
    }

    pub fn discard(&mut self) {
        self.edits.clear();
        self.deleted.clear();
        self.inserted.clear();
        self.editing = None;
    }

    pub fn title(&self) -> String {
        match &self.database {
            Some(db) => format!("{} @{db}", self.table),
            None => self.table.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Tab {
    Objects,
    Table(Box<TableTab>),
    Query(Box<QueryTab>),
    Console(Box<ConsoleTab>),
    Transfer(Box<TransferForm>),
    Automation(Box<AutomationView>),
    User(Box<UserTab>),
    /// Table Designer (Design Table / New Table).
    Designer(Box<crate::designer_tab::DesignerTab>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ConnTab {
    #[default]
    General,
    Advanced,
    Databases,
    Ssl,
    Ssh,
}

/// Result of a background "Test Connection".
pub type TestSlot = Arc<Mutex<Option<Result<String, String>>>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionForm {
    pub id: Option<String>,
    pub tab: ConnTab,
    pub kind: String,
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub save_password: bool,
    pub database: String,
    pub ssh_enabled: bool,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    /// `password` or `publickey`.
    pub ssh_auth: String,
    pub ssh_password: String,
    pub ssh_key: String,
    #[serde(skip)]
    pub test: Option<TestSlot>,
    /// Last finished test, shown until the form changes.
    pub test_result: Option<Result<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObjectsView {
    pub mode: ObjectsMode,
    pub search: String,
    /// Backup list / user list for the current selection (loaded on demand).
    pub backups: Option<Vec<dbcraft_engine::BackupInfo>>,
    pub users: Option<Result<Vec<UserInfo>, String>>,
    pub backup_comment: String,
    pub selected_backup: Option<String>,
    pub selected_user: Option<(String, Option<String>)>,
    /// Restore / delete confirmation for this backup path.
    pub confirm_restore: Option<String>,
    pub confirm_delete_user: Option<(String, Option<String>)>,
    pub confirm_delete_backup: Option<String>,
    #[serde(skip)]
    pub job: Job,
    /// The finished job's result has been handled.
    pub job_seen: bool,
    /// Selected row in the Objects list: (schema, name).
    pub selected: Option<(Option<String>, String)>,
    pub info_ddl: bool,
    /// DDL of the selected object, loaded on demand.
    pub ddl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    pub show_navigation: bool,
    pub show_info: bool,
    pub selection: Option<Selection>,
    pub objects: ObjectsView,
    /// `tabs[0]` is always the Objects tab.
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub connection_form: Option<ConnectionForm>,
    /// "Save changes to query?" for this tab index.
    pub confirm_close: Option<usize>,
    pub show_about: bool,
    /// Last error, shown in the status bar until the next action.
    pub status: Option<String>,
    /// Tools › Options dialog (Some while open).
    pub options_form: Option<crate::options_dialog::OptionsForm>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            show_navigation: true,
            show_info: true,
            selection: None,
            objects: ObjectsView::default(),
            tabs: vec![Tab::Objects],
            active_tab: 0,
            connection_form: None,
            confirm_close: None,
            show_about: false,
            status: None,
            options_form: None,
        }
    }
}
