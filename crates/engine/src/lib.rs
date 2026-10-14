//! The engine: a [`Session`] (saved workspace + live connections + last result) and the command
//! registry. The UI, CLI, control channel and MCP server all act through [`Session::dispatch`], so
//! anything a user can click, an agent can do.

mod automation_cmds;
mod backup_cmds;
mod commands;
mod connection_cmds;
mod data_cmds;
mod export_cmds;
mod params;
mod query_cmds;
mod saved_query_cmds;
pub mod transfer;
mod transfer_cmds;
mod user_cmds;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

pub use automation_cmds::task_xml;
pub use backup_cmds::BackupInfo;
pub use commands::{CommandSpec, commands};
pub use data_cmds::{DEFAULT_PAGE, FILTER_OPS, TablePage};
use dbcraft_core::{Error, Result, ResultSet, TableInfo};
use dbcraft_drivers::Connection;
pub use dbcraft_drivers::supported_kinds;
pub use dbcraft_drivers::users::{PrivilegeCatalog, catalog as privilege_catalog};
use dbcraft_model::{DbObject, Workspace};
use serde_json::Value as Json;

/// What [`Session::dispatch`] returns.
pub type CommandResult = Result<Json>;

/// Receives progress lines from long commands (transfer, backup, automation).
pub type ProgressSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Everything the app knows at runtime.
#[derive(Default)]
pub struct Session {
    pub workspace: Workspace,
    /// Where the workspace is saved after each change (`None`: in-memory, e.g. tests).
    pub workspace_path: Option<PathBuf>,
    open: HashMap<String, Box<dyn Connection>>,
    /// Databases per open connection, filled on open.
    pub databases: HashMap<String, Vec<String>>,
    /// Objects per (connection id, database), filled when a database is expanded.
    pub objects: HashMap<(String, String), Vec<DbObject>>,
    /// Objects-tab table details per (connection id, database).
    pub tables: HashMap<(String, String), Vec<TableInfo>>,
    /// Result of the last statement that ran (the grid shows it).
    pub last_result: Option<ResultSet>,
    /// Messages tab: one line per statement or error.
    pub messages: Vec<String>,
    /// Where backups and other files live (next to the workspace file).
    pub data_dir: Option<PathBuf>,
    /// Progress for long commands; `None` = silent.
    pub progress: Option<ProgressSink>,
}

impl Session {
    pub fn new() -> Self {
        Session::default()
    }

    /// Load (or start) the workspace at `path`. A corrupt file is reported, not fatal: the session
    /// starts empty and the bad file is left untouched until the next save.
    pub fn with_workspace_file(path: PathBuf) -> (Self, Option<Error>) {
        let data_dir = path.parent().map(|p| p.join("data"));
        let mut s = Session { workspace_path: Some(path.clone()), data_dir, ..Session::default() };
        let err = match std::fs::read_to_string(&path) {
            Ok(text) => match Workspace::from_json(&text) {
                Ok(ws) => {
                    s.workspace = ws;
                    None
                }
                Err(e) => Some(e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => Some(e.into()),
        };
        (s, err)
    }

    pub fn save_workspace(&self) -> Result<()> {
        let Some(path) = &self.workspace_path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write then rename so a crash mid-write never leaves a truncated file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.workspace.to_json()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.open.contains_key(id)
    }

    pub fn any_open(&self) -> bool {
        !self.open.is_empty()
    }

    pub(crate) fn conn(&mut self, id: &str) -> Result<&mut Box<dyn Connection>> {
        self.open.get_mut(id).ok_or_else(|| Error::InvalidParam(format!("connection `{id}` is not open (run connection.open first)")))
    }

    /// Like `conn`, but opens a saved connection on demand (tools, automation, background jobs).
    pub(crate) fn conn_auto(&mut self, id: &str) -> Result<&mut Box<dyn Connection>> {
        if !self.open.contains_key(id) {
            let cfg = self.workspace.connection(id)?.clone();
            let mut conn = dbcraft_drivers::connect(&cfg)?;
            let dbs = conn.list_databases()?;
            self.databases.insert(id.to_string(), dbs);
            self.open.insert(id.to_string(), conn);
            self.report(&format!("Connected to {}", cfg.name));
        }
        self.conn(id)
    }

    /// Send a progress line to the sink (if any) and the Messages log.
    pub fn report(&mut self, msg: &str) {
        if let Some(p) = &self.progress {
            p(msg);
        }
        self.messages.push(msg.to_string());
    }

    /// A fresh session over the same saved state, for a background worker: no live connections,
    /// no workspace file (the caller keeps ownership of saving).
    pub fn fork(&self, progress: Option<ProgressSink>) -> Session {
        Session { workspace: self.workspace.clone(), data_dir: self.data_dir.clone(), progress, ..Session::default() }
    }

    /// Run a command by id. Never panics: an escaped panic is caught and returned as an error so
    /// the session survives.
    pub fn dispatch(&mut self, id: &str, params: &Json) -> Result<Json> {
        let spec = commands().into_iter().find(|c| c.id == id).ok_or_else(|| Error::NotFound(format!("command `{id}`")))?;
        if !(spec.enabled)(self) {
            return Err(Error::InvalidParam(format!("command `{id}` is not available right now")));
        }
        log::debug!("dispatch {id} {params}");
        match catch_unwind(AssertUnwindSafe(|| (spec.run)(self, params))) {
            Ok(r) => {
                if let Err(e) = &r {
                    self.messages.push(format!("[{id}] {e}"));
                }
                r
            }
            Err(_) => {
                let e = Error::Database(format!("internal error in `{id}` (please report it); the session was kept"));
                self.messages.push(e.to_string());
                Err(e)
            }
        }
    }
}
