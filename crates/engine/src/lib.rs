//! The engine: a [`Session`] (saved workspace + live connections + last result) and the command
//! registry. The UI, CLI, control channel and MCP server all act through [`Session::dispatch`], so
//! anything a user can click, an agent can do.

pub mod alter;
mod automation_cmds;
mod backup_cmds;
pub mod backup_format;
mod commands;
mod connection_cmds;
mod data_cmds;
mod ddl;
mod export_cmds;
mod options_cmds;
mod params;
mod query_cmds;
mod saved_query_cmds;
pub mod secrets;
mod structure_sync_cmds;
mod table_cmds;
pub mod transfer;
mod transfer_cmds;
mod user_cmds;

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

pub use automation_cmds::task_xml;
pub use backup_cmds::BackupInfo;
pub use commands::{CommandSpec, commands};
pub use data_cmds::{DEFAULT_PAGE, FILTER_OPS, TablePage};
use dbcraft_core::{Error, Result, ResultSet, TableInfo};
use dbcraft_drivers::Connection;
pub use dbcraft_drivers::users::{PrivilegeCatalog, catalog as privilege_catalog};
pub use dbcraft_drivers::{Cancel, current_cancel, supported_kinds, with_cancel};
use dbcraft_model::{DbObject, PasswordStorage, Workspace};
pub use options_cmds::{check_format, page_size};
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
    open: HashMap<String, Arc<dyn Connection>>,
    /// Connections that were open when this session was forked with [`Session::fork_shared`].
    forked_open: HashSet<String>,
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
    /// The OS keychain (or a test store), when this workspace keeps passwords there.
    secrets: Option<Arc<dyn secrets::SecretStore>>,
    /// What the keychain holds per account, so saves only write changed passwords.
    stored: std::sync::Mutex<HashMap<String, Option<String>>>,
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

    /// Keep this workspace's passwords in `store` (the OS keychain). With `migrate`, a workspace
    /// that keeps them in its file moves them to the store now; otherwise only a workspace already
    /// in keychain mode reads its passwords from it. Returns entries that could not be read.
    pub fn attach_secrets(&mut self, store: Arc<dyn secrets::SecretStore>, migrate: bool) -> Result<Vec<String>> {
        self.secrets = Some(store.clone());
        match self.workspace.password_storage {
            PasswordStorage::Keychain => {
                let failed = secrets::load(&mut self.workspace, store.as_ref());
                // What was just read is what the keychain holds.
                let mut stored = self.stored.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                *stored = secrets::snapshot(&self.workspace);
                Ok(failed)
            }
            PasswordStorage::File if migrate => {
                if self.workspace.id.is_empty() {
                    self.workspace.id = secrets::new_workspace_id();
                }
                self.workspace.password_storage = PasswordStorage::Keychain;
                if let Err(e) = self.save_workspace() {
                    // Keep the passwords where they were rather than lose them.
                    self.workspace.password_storage = PasswordStorage::File;
                    return Err(e);
                }
                Ok(Vec::new())
            }
            PasswordStorage::File => Ok(Vec::new()),
        }
    }

    /// Desktop app and CLI: use the OS keychain for passwords. `migrate` moves the passwords of a
    /// file-mode workspace into it (the user's own workspace); without it, only a workspace already
    /// in keychain mode reads from it (dev/test workspaces keep their file). Returns a warning for
    /// the status bar, if any.
    pub fn use_os_keychain(&mut self, migrate: bool) -> Option<String> {
        let Some(store) = secrets::OsKeychain::available() else {
            return (self.workspace.password_storage == PasswordStorage::Keychain)
                .then(|| "this workspace keeps its passwords in the OS keychain, which is not available here".to_string());
        };
        match self.attach_secrets(store, migrate) {
            Ok(failed) if failed.is_empty() => None,
            Ok(failed) => Some(format!("could not read {} saved password(s) from the OS keychain", failed.len())),
            Err(e) => Some(format!("passwords stay in the workspace file: {e}")),
        }
    }

    pub fn save_workspace(&self) -> Result<()> {
        let Some(path) = &self.workspace_path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = {
            let mut stored = self.stored.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            secrets::for_file(&self.workspace, self.secrets.as_deref(), &mut stored)?
        };
        // Write then rename so a crash mid-write never leaves a truncated file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, file.to_json()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Remove a deleted connection's passwords from the keychain.
    pub(crate) fn forget_secrets(&self, conn: &str) -> Result<()> {
        let mut stored = self.stored.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        secrets::forget(&self.workspace, self.secrets.as_deref(), &mut stored, conn)
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.open.contains_key(id)
    }

    pub fn any_open(&self) -> bool {
        !self.open.is_empty()
    }

    pub(crate) fn conn(&self, id: &str) -> Result<Arc<dyn Connection>> {
        self.open.get(id).cloned().ok_or_else(|| Error::InvalidParam(format!("connection `{id}` is not open (run connection.open first)")))
    }

    /// Like `conn`, but opens a saved connection on demand (tools, automation, background jobs).
    pub(crate) fn conn_auto(&mut self, id: &str) -> Result<Arc<dyn Connection>> {
        if !self.open.contains_key(id) {
            let cfg = self.workspace.connection(id)?.clone();
            let conn = dbcraft_drivers::connect(&cfg)?;
            let dbs = conn.list_databases()?;
            self.databases.insert(id.to_string(), dbs);
            self.open.insert(id.to_string(), Arc::from(conn));
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

    /// A session for a background task that shares this session's live connections (queries,
    /// opening, loading objects). Run commands on it on a worker thread, then hand it back with
    /// [`Session::absorb`].
    pub fn fork_shared(&self) -> Session {
        Session {
            workspace: self.workspace.clone(),
            data_dir: self.data_dir.clone(),
            open: self.open.clone(),
            forked_open: self.open.keys().cloned().collect(),
            databases: self.databases.clone(),
            ..Session::default()
        }
    }

    /// Take back what a [`fork_shared`](Session::fork_shared) worker learned: connections it
    /// opened, databases, object lists, its last result and messages. A connection the user
    /// closed meanwhile stays closed.
    pub fn absorb(&mut self, worker: Session) {
        let Session { open, forked_open, databases, objects, tables, last_result, messages, .. } = worker;
        for (id, conn) in open {
            if !forked_open.contains(&id) && !self.open.contains_key(&id) && self.workspace.connection(&id).is_ok() {
                self.open.insert(id, conn);
            }
        }
        let live = |id: &String| self.open.contains_key(id);
        let databases: Vec<_> = databases.into_iter().filter(|(id, _)| live(id)).collect();
        let objects: Vec<_> = objects.into_iter().filter(|((id, _), _)| live(id)).collect();
        let tables: Vec<_> = tables.into_iter().filter(|((id, _), _)| live(id)).collect();
        self.databases.extend(databases);
        self.objects.extend(objects);
        self.tables.extend(tables);
        if last_result.is_some() {
            self.last_result = last_result;
        }
        self.messages.extend(messages);
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
