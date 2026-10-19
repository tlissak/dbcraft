use std::collections::HashSet;

use dbcraft_core::{DbKind, ResultSet, UserInfo};
use dbcraft_engine::{Session, TablePage};
use dbcraft_model::ObjectKind;
use egui::collapsing_header::CollapsingState;
use egui::{RichText, Ui};
use serde_json::{Value as Json, json};

use crate::automation_tab::AutoAct;
use crate::conn_dialog::{self, Outcome};
use crate::icons;
use crate::jobs::Job;
use crate::menu_catalog::{CATALOG, MENUS};
use crate::state::{AutomationView, ConnectionForm, ConsoleTab, ObjectsMode, QueryTab, Selection, Tab, TableTab, TransferForm, UiState, UserTab};
use crate::tasks::{self, Task};
use crate::theme::Tokens;

/// Something the user asked for this frame; applied after drawing so drawing can borrow the
/// session immutably.
pub enum Action {
    Command(&'static str, Json),
    Ui(&'static str),
    NewQuery {
        connection: Option<String>,
        database: Option<String>,
    },
    Select(Selection),
    OpenTable {
        connection: String,
        database: String,
        schema: Option<String>,
        table: String,
    },
    ReloadTable(usize),
    ApplyTable(usize),
    LoadDdl {
        connection: String,
        database: String,
        schema: Option<String>,
        table: String,
    },
    EditConnection(String),
    /// Load a database's object list and table details (once; retried via Refresh).
    LoadDb(String, String),
    RunQuery(usize),
    /// Save (opens Save As for a new query).
    SaveQuery(usize),
    SaveQueryAs(usize, String),
    CancelSavePrompt(usize),
    OpenSavedQuery(String),
    /// Close a tab (asks first if a query has unsaved changes).
    CloseTab(usize),
    OpenConsole(String, Option<String>),
    ConsoleEnter(usize),
    OpenTransfer,
    TransferLoadTables(usize),
    TransferStart(usize),
    TransferSaveJob(usize),
    OpenAutomation,
    Automation(usize, AutoAct),
    SetObjectsMode(ObjectsMode),
    LoadBackups,
    BackupCreate,
    BackupRestore(String),
    BackupDelete(String),
    BackupOpenFolder,
    LoadUsers,
    NewUser {
        connection: String,
        role: bool,
    },
    OpenUser {
        connection: String,
        name: String,
        host: Option<String>,
    },
    DeleteUser {
        connection: String,
        name: String,
        host: Option<String>,
    },
    UserPreview(usize),
    UserSave(usize),
    /// Replace the SQL of the active query tab.
    SetQueryText(String),
    /// Stop the running task of tab `i` (Stop button, Ctrl+T).
    StopTask(usize),
    /// Give up opening this connection.
    StopOpening(String),
}

/// Navigation groups under a database, in Navicat's order.
const GROUPS: &[(ObjectKind, &str)] = &[
    (ObjectKind::Table, "Tables"),
    (ObjectKind::View, "Views"),
    (ObjectKind::Function, "Functions"),
    (ObjectKind::Procedure, "Procedures"),
    (ObjectKind::Trigger, "Triggers"),
    (ObjectKind::Event, "Events"),
    (ObjectKind::Sequence, "Sequences"),
    (ObjectKind::Index, "Indexes"),
];

pub struct DbCraftApp {
    pub session: Session,
    pub state: UiState,
    /// Tree nodes we already tried to load, so a failing server isn't retried every frame.
    pub(crate) tried: HashSet<String>,
    themed: bool,
    /// Applied on the first frame (command-line `--select`, `--open-table`, …).
    startup: Vec<Action>,
    /// For starting background jobs from code without a `ctx` at hand.
    pub(crate) ctx_for_jobs: egui::Context,
    /// Set after the first tree frame. eframe persists which tree nodes were expanded, and an
    /// expanded connection node opens its connection; at launch we collapse them instead, so
    /// connecting is always a manual action.
    tree_started: bool,
    /// Opening connections and loading object lists in the background (ROADMAP 1.1).
    bg: Vec<(Task, Then)>,
    /// Actions waiting for a connection that is still opening.
    after_open: Vec<(String, Action)>,
    /// Tests: wait for each background task right after starting it, so a frame later its result
    /// is on screen (kittest doesn't run real time).
    wait_for_tasks: bool,
}

/// What to do when a background task in [`DbCraftApp::bg`] finishes.
enum Then {
    Opened(String),
    LoadedDb,
    Ddl { schema: Option<String>, table: String },
}

/// Things to do at launch, from the command line (handy for agents and screenshots).
pub enum StartupAction {
    /// Open the connection and select it, or one of its databases.
    Select {
        connection: String,
        database: Option<String>,
    },
    OpenTable {
        connection: String,
        database: String,
        schema: Option<String>,
        table: String,
    },
    NewConnection,
    /// Open a Console tab on a connection.
    Console {
        connection: String,
        database: Option<String>,
    },
    /// `transfer` or `automation`.
    Tool(String),
    /// Objects tab mode: `queries`, `backups` or `users`.
    Mode(String),
    NewQuery {
        connection: String,
        database: Option<String>,
        sql: String,
    },
}

impl DbCraftApp {
    pub fn new(session: Session, startup_error: Option<String>) -> Self {
        let state = UiState { status: startup_error, ..UiState::default() };
        DbCraftApp {
            session,
            state,
            tried: HashSet::new(),
            themed: false,
            startup: Vec::new(),
            ctx_for_jobs: egui::Context::default(),
            tree_started: false,
            bg: Vec::new(),
            after_open: Vec::new(),
            wait_for_tasks: false,
        }
    }

    /// Tests: finish each background task before the next frame (see `wait_for_tasks`).
    pub fn waiting_for_tasks(mut self) -> Self {
        self.wait_for_tasks = true;
        self
    }

    pub fn startup(&mut self, a: StartupAction) {
        match a {
            StartupAction::Select { connection, database } => {
                self.startup.push(Action::Command("connection.open", json!({ "id": connection })));
                self.startup.push(Action::Select(Selection { connection, database, group: ObjectKind::Table }));
            }
            StartupAction::OpenTable { connection, database, schema, table } => {
                if !self.startup.iter().any(|a| matches!(a, Action::Command("connection.open", p) if p["id"] == connection)) {
                    self.startup.push(Action::Command("connection.open", json!({ "id": connection })));
                }
                self.startup.push(Action::OpenTable { connection, database, schema, table });
            }
            StartupAction::NewConnection => self.startup.push(Action::Ui("dialog.newConnection")),
            StartupAction::Console { connection, database } => self.startup.push(Action::OpenConsole(connection, database)),
            StartupAction::NewQuery { connection, database, sql } => {
                self.startup.push(Action::Command("connection.open", json!({ "id": connection })));
                self.startup.push(Action::NewQuery { connection: Some(connection), database });
                self.startup.push(Action::SetQueryText(sql));
            }
            StartupAction::Tool(t) => self.startup.push(if t == "automation" { Action::OpenAutomation } else { Action::OpenTransfer }),
            StartupAction::Mode(m) => self.startup.push(Action::SetObjectsMode(match m.as_str() {
                "queries" => ObjectsMode::Queries,
                "backups" => ObjectsMode::Backups,
                "users" => ObjectsMode::Users,
                _ => ObjectsMode::Objects,
            })),
        }
    }

    fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Command("connection.open", params) => {
                    if let Some(id) = params["id"].as_str() {
                        self.open_async(id);
                    }
                }
                Action::Command(id @ ("database.tables" | "database.objects"), params) => self.spawn_bg(vec![(id, params)], Then::LoadedDb),
                Action::Command(id, params) => {
                    self.run(id, &params);
                }
                Action::NewQuery { connection, database } => self.new_query(connection, database),
                Action::Ui(id) => self.ui_command(ctx, id),
                Action::Select(sel) => {
                    // Reveal the selection in the tree.
                    let mut ids = vec![egui::Id::new(("conn", &sel.connection))];
                    if let Some(db) = &sel.database {
                        ids.push(egui::Id::new(("db", sel.connection.as_str(), db.as_str())));
                    }
                    for id in ids {
                        let mut st = CollapsingState::load_with_default_open(ctx, id, false);
                        st.set_open(true);
                        st.store(ctx);
                    }
                    self.select(sel);
                }
                Action::OpenTable { connection, database, schema, table } => self.open_table(connection, database, schema, table),
                Action::ReloadTable(i) => self.reload_table(i),
                Action::ApplyTable(i) => self.apply_table(i),
                Action::LoadDdl { connection, database, schema, table } => {
                    self.state.objects.ddl = Some("-- Loading…".into());
                    let params = json!({ "connection": connection, "database": database, "schema": schema, "table": table });
                    self.spawn_bg(vec![("table.ddl", params)], Then::Ddl { schema, table });
                }
                Action::LoadDb(conn, db) => {
                    let objects = self.tried.insert(format!("d/{conn}/{db}"));
                    let tables = self.tried.insert(format!("t/{conn}/{db}"));
                    self.load_db(&conn, &db, tables, objects);
                }
                Action::EditConnection(id) => {
                    if let Ok(cfg) = self.session.workspace.connection(&id) {
                        self.state.connection_form = Some(ConnectionForm::edit(cfg));
                    }
                }
                Action::RunQuery(i) => self.run_query(i),
                Action::SaveQuery(i) => self.save_query(i, None),
                Action::SaveQueryAs(i, name) => self.save_query(i, Some(name)),
                Action::CancelSavePrompt(i) => {
                    if let Some(Tab::Query(q)) = self.state.tabs.get_mut(i) {
                        q.save_prompt = None;
                        q.close_after_save = false;
                    }
                }
                Action::OpenSavedQuery(id) => self.open_saved_query(&id),
                Action::CloseTab(i) => self.close_tab(i, false),
                Action::OpenConsole(conn, db) => {
                    let mut t = ConsoleTab { connection: conn.clone(), database: db, ..ConsoleTab::default() };
                    let name = self.session.workspace.connection(&conn).map(|c| c.name.clone()).unwrap_or_default();
                    t.scrollback = format!("DbCraft console: {name}\nEnd statements with ;   use <db> switches database, clear clears the screen.\n\n");
                    self.state.tabs.push(Tab::Console(Box::new(t)));
                    self.state.active_tab = self.state.tabs.len() - 1;
                }
                Action::ConsoleEnter(i) => self.console_enter(i),
                Action::OpenTransfer => {
                    let mut f = TransferForm::new();
                    if let Some(sel) = &self.state.selection {
                        f.source = Some(sel.connection.clone());
                        f.source_db = sel.database.clone();
                    }
                    self.state.tabs.push(Tab::Transfer(Box::new(f)));
                    self.state.active_tab = self.state.tabs.len() - 1;
                    self.transfer_load_tables(self.state.active_tab);
                }
                Action::TransferLoadTables(i) => self.transfer_load_tables(i),
                Action::TransferStart(i) => {
                    if let Some(Tab::Transfer(f)) = self.state.tabs.get_mut(i)
                        && let Ok(params) = f.params()
                    {
                        f.status = None;
                        f.job = Job::spawn(ctx, &self.session, "tools.dataTransfer", params);
                    }
                }
                Action::TransferSaveJob(i) => {
                    if let Some(Tab::Transfer(f)) = self.state.tabs.get_mut(i)
                        && let Ok(params) = f.params()
                    {
                        let label = format!("Data Transfer from {}", f.source.clone().unwrap_or_default());
                        let r = self.session.dispatch(
                            "automation.save",
                            &json!({"name": label, "steps": [{"command": "tools.dataTransfer", "params": params, "label": label}]}),
                        );
                        f.status = Some(match r {
                            Ok(v) => format!("Saved as batch job {} (Tools › Automation)", v["id"].as_str().unwrap_or("")),
                            Err(e) => e.to_string(),
                        });
                    }
                }
                Action::OpenAutomation => {
                    if let Some(i) = self.state.tabs.iter().position(|t| matches!(t, Tab::Automation(_))) {
                        self.state.active_tab = i;
                    } else {
                        self.state.tabs.push(Tab::Automation(Box::new(AutomationView::new())));
                        self.state.active_tab = self.state.tabs.len() - 1;
                    }
                }
                Action::Automation(i, act) => self.automation(i, act),
                Action::SetObjectsMode(m) => {
                    self.state.objects.mode = m;
                    self.state.objects.backups = None;
                    self.state.objects.users = None;
                    self.state.active_tab = 0;
                }
                Action::LoadBackups => self.load_backups(),
                Action::BackupCreate => {
                    if let Some(sel) = &self.state.selection
                        && let Some(db) = &sel.database
                    {
                        let params = json!({"connection": sel.connection, "database": db, "comment": self.state.objects.backup_comment});
                        self.state.objects.job = Job::spawn(ctx, &self.session, "backup.create", params);
                        self.state.objects.job_seen = false;
                        self.state.objects.backup_comment.clear();
                    }
                }
                Action::BackupRestore(path) => {
                    if let Some(sel) = &self.state.selection
                        && let Some(db) = &sel.database
                    {
                        let params = json!({"connection": sel.connection, "database": db, "path": path});
                        self.state.objects.job = Job::spawn(ctx, &self.session, "backup.restore", params);
                        self.state.objects.job_seen = false;
                    }
                }
                Action::BackupDelete(path) => {
                    self.run("backup.delete", &json!({ "path": path }));
                    self.state.objects.selected_backup = None;
                    self.load_backups();
                }
                Action::BackupOpenFolder => {
                    if let Some(sel) = &self.state.selection
                        && let (Some(db), Some(dir)) = (&sel.database, &self.session.data_dir)
                    {
                        let clean = |x: &str| x.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect::<String>();
                        let path = dir.join("backups").join(clean(&sel.connection)).join(clean(db));
                        let _ = std::fs::create_dir_all(&path);
                        let _ = std::process::Command::new("explorer").arg(&path).spawn();
                    }
                }
                Action::LoadUsers => {
                    if let Some(sel) = &self.state.selection {
                        let r = self.session.dispatch("user.list", &json!({ "connection": sel.connection }));
                        self.state.objects.users =
                            Some(r.map_err(|e| e.to_string()).and_then(|v| serde_json::from_value::<Vec<UserInfo>>(v).map_err(|e| e.to_string())));
                    }
                }
                Action::NewUser { connection, role } => {
                    let kind = self.session.workspace.connection(&connection).map(|c| c.kind).ok();
                    let host = matches!(kind, Some(DbKind::Mysql | DbKind::Mariadb)).then(|| "%".to_string());
                    let user = UserInfo { name: if role { "new_role".into() } else { "new_user".into() }, host, is_role: role, ..UserInfo::default() };
                    self.state.tabs.push(Tab::User(Box::new(UserTab { connection, user, ..UserTab::default() })));
                    self.state.active_tab = self.state.tabs.len() - 1;
                }
                Action::OpenUser { connection, name, host } => {
                    match self
                        .session
                        .dispatch("user.get", &json!({ "connection": connection, "name": name, "host": host }))
                        .map(serde_json::from_value::<UserInfo>)
                    {
                        Ok(Ok(u)) => {
                            if !matches!(self.state.objects.users, Some(Ok(_))) {
                                let r = self.session.dispatch("user.list", &json!({ "connection": connection }));
                                self.state.objects.users =
                                    Some(r.map_err(|e| e.to_string()).and_then(|v| serde_json::from_value::<Vec<UserInfo>>(v).map_err(|e| e.to_string())));
                            }
                            self.state.tabs.push(Tab::User(Box::new(UserTab { connection, original: Some(u.clone()), user: u, ..UserTab::default() })));
                            self.state.active_tab = self.state.tabs.len() - 1;
                        }
                        Ok(Err(e)) => self.state.status = Some(e.to_string()),
                        Err(e) => self.state.status = Some(e.to_string()),
                    }
                }
                Action::DeleteUser { connection, name, host } => {
                    self.run("user.delete", &json!({ "connection": connection, "name": name, "host": host }));
                    self.state.objects.selected_user = None;
                    self.state.objects.users = None;
                }
                Action::SetQueryText(sql) => {
                    if let Some(Tab::Query(q)) = self.state.tabs.get_mut(self.state.active_tab) {
                        q.sql = sql;
                    }
                }
                Action::StopTask(i) => {
                    if let Some(t) = self.state.tabs.get(i).and_then(tab_task) {
                        t.cancel();
                    }
                }
                Action::StopOpening(id) => {
                    for (t, then) in &self.bg {
                        if matches!(then, Then::Opened(c) if *c == id) {
                            t.cancel();
                        }
                    }
                }
                Action::UserPreview(i) => self.user_save(i, true),
                Action::UserSave(i) => self.user_save(i, false),
            }
        }
    }

    fn run_query(&mut self, i: usize) {
        let Some(Tab::Query(q)) = self.state.tabs.get(i) else { return };
        let Some(conn) = q.connection.clone() else { return };
        if q.task.is_some() {
            return; // already running; Stop first
        }
        let mut steps = Vec::new();
        if !self.session.is_open(&conn) {
            steps.push(("connection.open", json!({ "id": conn })));
        }
        steps.push(("query.run", json!({ "connection": conn, "database": q.database, "sql": q.sql })));
        let task = self.spawn_task(steps);
        if let Some(Tab::Query(q)) = self.state.tabs.get_mut(i) {
            q.task = Some(task);
        }
    }

    fn query_done(q: &mut QueryTab, result: Result<Json, String>, messages: Vec<String>) {
        q.messages = messages;
        q.grid = Default::default();
        match result {
            Ok(v) => {
                q.results = serde_json::from_value(v["results"].clone()).unwrap_or_default();
                q.error = None;
                q.result_idx = 0;
            }
            Err(e) => {
                q.error = Some(e);
                q.results.clear();
                q.result_idx = 0; // == results.len(): the Message tab
            }
        }
    }

    fn save_query(&mut self, i: usize, name: Option<String>) {
        let Some(Tab::Query(q)) = self.state.tabs.get_mut(i) else { return };
        let Some(conn) = q.connection.clone() else {
            q.error = Some("Choose a connection before saving the query.".into());
            q.result_idx = q.results.len();
            return;
        };
        let (id, title) = match (&name, &q.saved_id) {
            (Some(n), _) => (None, n.clone()),
            (None, Some(id)) => (Some(id.clone()), q.title.clone()),
            (None, None) => {
                q.save_prompt = Some(String::new());
                return;
            }
        };
        let r = self.session.dispatch("query.save", &json!({ "id": id, "name": title, "connection": conn, "database": q.database, "sql": q.sql }));
        match r {
            Ok(v) => {
                q.saved_id = v["id"].as_str().map(str::to_string);
                q.saved_sql = q.sql.clone();
                q.title = title;
                q.save_prompt = None;
                q.error = None;
                if q.close_after_save {
                    self.close_tab(i, true);
                }
            }
            Err(e) => {
                q.error = Some(e.to_string());
                q.result_idx = q.results.len();
            }
        }
    }

    fn open_saved_query(&mut self, id: &str) {
        if let Some(i) = self.state.tabs.iter().position(|t| matches!(t, Tab::Query(q) if q.saved_id.as_deref() == Some(id))) {
            self.state.active_tab = i;
            return;
        }
        let Some(sq) = self.session.workspace.queries.iter().find(|q| q.id == id).cloned() else { return };
        self.open_async(&sq.connection);
        let tab = QueryTab {
            title: sq.name,
            connection: Some(sq.connection),
            database: sq.database,
            sql: sq.sql.clone(),
            saved_id: Some(sq.id),
            saved_sql: sq.sql,
            ..QueryTab::default()
        };
        self.state.tabs.push(Tab::Query(Box::new(tab)));
        self.state.active_tab = self.state.tabs.len() - 1;
    }

    /// Close tab `i`; `force` skips the unsaved-changes prompt.
    pub fn close_tab(&mut self, i: usize, force: bool) {
        if i == 0 || i >= self.state.tabs.len() {
            return;
        }
        if !force && matches!(self.state.tabs.get(i), Some(Tab::Query(q)) if q.dirty()) {
            self.state.confirm_close = Some(i);
            self.state.active_tab = i;
            return;
        }
        let removed = self.state.tabs.remove(i);
        if let Some(t) = tab_task(&removed) {
            t.cancel();
        }
        if self.state.active_tab >= i {
            self.state.active_tab = self.state.active_tab.saturating_sub(1);
        }
    }

    fn console_enter(&mut self, i: usize) {
        let Some(Tab::Console(t)) = self.state.tabs.get_mut(i) else { return };
        let kind = self.session.workspace.connection(&t.connection).map(|c| c.kind).ok();
        let line = std::mem::take(&mut t.input);
        let prompt = crate::console_tab::prompt(kind, t);
        t.scrollback.push_str(&format!("{prompt}{line}\n"));
        if !line.trim().is_empty() {
            t.history.push(line.clone());
        }
        t.history_pos = t.history.len();
        if !crate::console_tab::complete(&t.pending, &line) {
            if !line.trim().is_empty() {
                t.pending.push_str(&line);
                t.pending.push('\n');
            }
            return;
        }
        let stmt = format!("{}{line}", std::mem::take(&mut t.pending)).trim().to_string();
        let lower = stmt.trim_end_matches(';').trim().to_ascii_lowercase();
        if lower == "clear" {
            t.scrollback.clear();
            return;
        }
        if matches!(lower.as_str(), "exit" | "quit") {
            t.scrollback.push_str("Bye (close the tab to end the console).\n");
            return;
        }
        if lower == "help" {
            t.scrollback.push_str("End SQL statements with ;  ·  use <db> / \\c <db> switch database  ·  clear  ·  Up/Down: history\n\n");
            return;
        }
        if let Some(db) = lower.strip_prefix("use ").or_else(|| lower.strip_prefix("\\c ")) {
            let original = stmt.trim_end_matches(';').trim();
            let name = original.get(original.len() - db.len()..).unwrap_or(db).trim().trim_matches(['`', '"']).to_string();
            t.database = Some(name.clone());
            t.scrollback.push_str(&format!("Database changed to {name}\n\n"));
            return;
        }
        let (conn, db) = (t.connection.clone(), t.database.clone());
        let mut steps = Vec::new();
        if !self.session.is_open(&conn) {
            steps.push(("connection.open", json!({ "id": conn })));
        }
        steps.push(("query.run", json!({ "connection": conn, "database": db, "sql": stmt })));
        let task = self.spawn_task(steps);
        if let Some(Tab::Console(t)) = self.state.tabs.get_mut(i) {
            t.task = Some(task);
        }
    }

    fn console_done(t: &mut ConsoleTab, result: Result<Json, String>) {
        match result {
            Ok(v) => {
                let results: Vec<ResultSet> = serde_json::from_value(v["results"].clone()).unwrap_or_default();
                for rs in results {
                    t.scrollback.push_str(&rs.to_text_table());
                    t.scrollback.push_str("\n\n");
                }
            }
            Err(e) => t.scrollback.push_str(&format!("ERROR: {e}\n\n")),
        }
    }

    fn transfer_load_tables(&mut self, i: usize) {
        let Some(Tab::Transfer(f)) = self.state.tabs.get(i) else { return };
        let (Some(conn), Some(db)) = (f.source.clone(), f.source_db.clone()) else {
            // Opening the source lists its databases for the picker.
            if let Some(conn) = f.source.clone() {
                self.open_async(&conn);
            }
            return;
        };
        let r = self.session.dispatch("database.tables", &json!({ "connection": conn, "database": db }));
        if let Some(Tab::Transfer(f)) = self.state.tabs.get_mut(i) {
            match r {
                Ok(v) => {
                    let infos: Vec<dbcraft_core::TableInfo> = serde_json::from_value(v).unwrap_or_default();
                    f.tables = infos.into_iter().filter(|t| t.kind == "table").map(|t| (t.schema, t.name, true)).collect();
                }
                Err(e) => f.status = Some(e.to_string()),
            }
        }
    }

    fn automation(&mut self, i: usize, act: AutoAct) {
        let ctx_cli = std::env::current_exe().ok().map(|p| p.with_file_name(if cfg!(windows) { "dbcraft-cli.exe" } else { "dbcraft-cli" }));
        let Some(Tab::Automation(v)) = self.state.tabs.get_mut(i) else { return };
        v.status = None;
        match act {
            AutoAct::New => v.editing = Some(crate::automation_tab::blank_job(&self.session)),
            AutoAct::Edit(id) => {
                v.editing = self.session.workspace.automations.iter().find(|j| j.id == id).cloned();
                if let Some(s) = v.editing.as_ref().and_then(|j| j.schedule.clone()) {
                    v.frequency = s.frequency;
                    v.time = s.time;
                    v.day = s.day.unwrap_or_else(|| "MON".into());
                }
            }
            AutoAct::Save => {
                if let Some(j) = &v.editing {
                    let r = self
                        .session
                        .dispatch("automation.save", &json!({ "id": j.id, "name": j.name, "steps": j.steps, "continue_on_error": j.continue_on_error }));
                    v.status = Some(match r {
                        Ok(_) => "Saved".into(),
                        Err(e) => e.to_string(),
                    });
                    if let Some(saved) = self.session.workspace.automations.iter().find(|x| x.id == j.id) {
                        v.editing = Some(saved.clone());
                    }
                }
            }
            AutoAct::Delete(id) => {
                let r = self.session.dispatch("automation.delete", &json!({ "id": id }));
                if v.editing.as_ref().is_some_and(|j| j.id == id) {
                    v.editing = None;
                }
                if let Err(e) = r {
                    v.status = Some(e.to_string());
                }
            }
            AutoAct::Run => {
                if let Some(j) = &v.editing {
                    // Run the version on screen: save it first so the worker sees the same steps.
                    let saved = self
                        .session
                        .dispatch("automation.save", &json!({ "id": j.id, "name": j.name, "steps": j.steps, "continue_on_error": j.continue_on_error }));
                    match saved {
                        Ok(_) => v.job = Job::spawn(&self.ctx_for_jobs, &self.session, "automation.run", json!({ "id": j.id })),
                        Err(e) => v.status = Some(e.to_string()),
                    }
                }
            }
            AutoAct::Schedule => {
                if let (Some(j), Some(cli)) = (&v.editing, ctx_cli) {
                    let r = self.session.dispatch(
                        "automation.schedule",
                        &json!({ "id": j.id, "frequency": v.frequency, "time": v.time, "day": v.day, "cli_path": cli.to_string_lossy() }),
                    );
                    v.status = Some(match r {
                        Ok(r) => format!("Scheduled as task {}", r["task"].as_str().unwrap_or("")),
                        Err(e) => e.to_string(),
                    });
                }
            }
            AutoAct::Unschedule => {
                if let Some(j) = &v.editing {
                    let r = self.session.dispatch("automation.unschedule", &json!({ "id": j.id }));
                    v.status = Some(match r {
                        Ok(_) => "Schedule deleted".into(),
                        Err(e) => e.to_string(),
                    });
                }
            }
        }
    }

    fn load_backups(&mut self) {
        let Some(sel) = &self.state.selection else { return };
        let Some(db) = &sel.database else { return };
        let r = self.session.dispatch("backup.list", &json!({ "connection": sel.connection, "database": db }));
        self.state.objects.backups = Some(r.ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default());
    }

    fn user_save(&mut self, i: usize, dry_run: bool) {
        let Some(Tab::User(t)) = self.state.tabs.get_mut(i) else { return };
        if !t.password.is_empty() && t.password != t.confirm {
            t.error = Some("Passwords do not match".into());
            return;
        }
        let r = self
            .session
            .dispatch("user.save", &json!({ "connection": t.connection, "user": t.user, "original": t.original, "password": t.password, "dry_run": dry_run }));
        match r {
            Ok(v) => {
                t.preview = serde_json::from_value(v["sql"].clone()).unwrap_or_default();
                t.error = None;
                if !dry_run {
                    // Reload what the server has now.
                    let got = self.session.dispatch("user.get", &json!({ "connection": t.connection, "name": t.user.name, "host": t.user.host }));
                    if let Ok(u) = got.and_then(|v| serde_json::from_value::<UserInfo>(v).map_err(|e| dbcraft_core::Error::InvalidParam(e.to_string()))) {
                        t.original = Some(u.clone());
                        t.user = u;
                    }
                    t.password.clear();
                    t.confirm.clear();
                    t.saved = true;
                    self.state.objects.users = None;
                }
            }
            Err(e) => {
                t.error = Some(e.to_string());
                t.saved = false;
            }
        }
    }

    /// Finished background jobs: refresh what they changed.
    fn poll_jobs(&mut self) {
        if let Some(Ok(v)) = self.state.objects.job.result()
            && self.state.objects.job.started()
            && !self.state.objects.job_seen
        {
            self.state.objects.job_seen = true;
            let _ = v;
            self.load_backups();
            if let Some(sel) = &self.state.selection
                && let Some(db) = &sel.database
            {
                self.session.tables.remove(&(sel.connection.clone(), db.clone()));
                self.session.objects.remove(&(sel.connection.clone(), db.clone()));
                self.tried.retain(|k| !k.ends_with(&format!("/{}/{db}", sel.connection)));
            }
        }
        for t in &mut self.state.tabs {
            match t {
                Tab::Transfer(f) if f.job.started() && f.status.is_none() => {
                    if let Some(r) = f.job.result() {
                        f.status = Some(match r {
                            Ok(v) => format!("{} tables, {} rows copied, {} errors", v["tables"], v["rows"], v["errors"].as_array().map_or(0, Vec::len)),
                            Err(e) => e,
                        });
                        // Target trees may have new tables.
                        self.session.tables.clear();
                        self.session.objects.clear();
                        self.tried.retain(|k| k.starts_with("c/"));
                    }
                }
                Tab::Automation(v) if v.job.started() && v.status.is_none() => {
                    if let Some(r) = v.job.result() {
                        let id = v.editing.as_ref().map(|j| j.id.clone());
                        v.status = Some(match &r {
                            Ok(res) => res["summary"].as_str().unwrap_or("").to_string(),
                            Err(e) => e.clone(),
                        });
                        // The worker had no workspace file; record the run here.
                        if let (Some(id), Ok(res)) = (id, &r)
                            && let Some(job) = self.session.workspace.automations.iter_mut().find(|j| j.id == id)
                        {
                            job.last_run = res["summary"].as_str().map(str::to_string);
                            // Keep the copy on screen in step, or it would look "unsaved".
                            if let Some(e) = v.editing.as_mut().filter(|e| e.id == id) {
                                e.last_run = job.last_run.clone();
                            }
                            let _ = self.session.save_workspace();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn run(&mut self, id: &str, params: &Json) -> Option<Json> {
        match self.session.dispatch(id, params) {
            Ok(v) => {
                self.state.status = None;
                Some(v)
            }
            Err(e) => {
                self.state.status = Some(e.to_string());
                None
            }
        }
    }

    fn select(&mut self, sel: Selection) {
        if let Some(db) = &sel.database {
            let key = (sel.connection.clone(), db.clone());
            let tables = !self.session.tables.contains_key(&key) && self.tried.insert(format!("t/{}/{db}", sel.connection));
            let objects = !self.session.objects.contains_key(&key) && self.tried.insert(format!("d/{}/{db}", sel.connection));
            self.load_db(&sel.connection, db, tables, objects);
        }
        if self.state.selection.as_ref().map(|s| (&s.connection, &s.database)) != Some((&sel.connection, &sel.database)) {
            self.state.objects.selected = None;
            self.state.objects.ddl = None;
        }
        self.state.selection = Some(sel);
        self.state.active_tab = 0;
    }

    fn new_query(&mut self, connection: Option<String>, database: Option<String>) {
        let n = self.state.tabs.iter().filter(|t| matches!(t, Tab::Query(_))).count() + 1;
        let sel = self.state.selection.clone();
        let connection = connection.or_else(|| sel.as_ref().map(|s| s.connection.clone()).filter(|c| self.session.is_open(c)));
        let database = database.or_else(|| sel.and_then(|s| s.database));
        self.state.tabs.push(Tab::Query(Box::new(QueryTab { title: format!("Untitled Query {n}"), connection, database, ..QueryTab::default() })));
        self.state.active_tab = self.state.tabs.len() - 1;
    }

    fn open_table(&mut self, connection: String, database: String, schema: Option<String>, table: String) {
        let existing = self.state.tabs.iter().position(|t| {
            matches!(t, Tab::Table(tt) if tt.connection == connection && tt.database.as_deref() == Some(&database) && tt.schema == schema && tt.table == table)
        });
        if let Some(i) = existing {
            self.state.active_tab = i;
            return;
        }
        let tab = TableTab { connection, database: Some(database), schema, table, limit: dbcraft_engine::DEFAULT_PAGE, ..TableTab::default() };
        self.state.tabs.push(Tab::Table(Box::new(tab)));
        self.state.active_tab = self.state.tabs.len() - 1;
        self.reload_table(self.state.active_tab);
    }

    fn reload_table(&mut self, i: usize) {
        let Some(Tab::Table(tab)) = self.state.tabs.get(i) else { return };
        if let Some(t) = &tab.task {
            t.cancel(); // a newer page (sort, filter, paging) replaces the one loading
        }
        let conn = tab.connection.clone();
        if !self.session.is_open(&conn) {
            // Loaded once the connection is open (see `Then::Opened`).
            self.open_async(&conn);
            return;
        }
        let params = crate::table_tab::data_params(tab);
        let task = self.spawn_task(vec![("table.data", params)]);
        if let Some(Tab::Table(tab)) = self.state.tabs.get_mut(i) {
            tab.task = Some(task);
        }
    }

    fn table_done(tab: &mut TableTab, result: Result<Json, String>) {
        match result.and_then(|v| serde_json::from_value::<TablePage>(v).map_err(|e| e.to_string())) {
            Ok(page) => {
                let rows = page.result.rows.len();
                let cols = page.result.columns.len();
                tab.page = Some(page);
                tab.discard();
                tab.error = None;
                tab.selected = tab.selected.filter(|(r, c)| *r < rows && *c < cols);
            }
            Err(e) => tab.error = Some(e),
        }
    }

    /// Start a task for a tab (query, table page, console statement).
    fn spawn_task(&self, steps: Vec<(&'static str, Json)>) -> Task {
        let task = Task::spawn(&self.ctx_for_jobs, &self.session, steps);
        if self.wait_for_tasks {
            tasks::wait(&task, std::time::Duration::from_secs(60));
        }
        task
    }

    fn spawn_bg(&mut self, steps: Vec<(&'static str, Json)>, then: Then) {
        let task = self.spawn_task(steps);
        self.bg.push((task, then));
    }

    /// Something runs in the background (opening, loading, a tab's query or page).
    pub fn is_busy(&self) -> bool {
        !self.bg.is_empty() || self.state.tabs.iter().any(|t| tab_task(t).is_some())
    }

    /// A connection is being opened in the background.
    pub(crate) fn opening(&self, id: &str) -> bool {
        self.bg.iter().any(|(_, t)| matches!(t, Then::Opened(c) if c == id))
    }

    /// Open a saved connection without blocking the window (no-op if open or opening).
    fn open_async(&mut self, id: &str) {
        if self.session.is_open(id) || self.opening(id) || self.session.workspace.connection(id).is_err() {
            return;
        }
        self.spawn_bg(vec![("connection.open", json!({ "id": id }))], Then::Opened(id.to_string()));
    }

    /// Load a database's table details and/or object list in the background.
    fn load_db(&mut self, conn: &str, db: &str, tables: bool, objects: bool) {
        if !tables && !objects {
            return;
        }
        if !self.session.is_open(conn) {
            // Try again once the connection is open.
            if tables {
                self.tried.remove(&format!("t/{conn}/{db}"));
            }
            if objects {
                self.tried.remove(&format!("d/{conn}/{db}"));
            }
            self.after_open.push((conn.to_string(), Action::LoadDb(conn.to_string(), db.to_string())));
            self.open_async(conn);
            return;
        }
        let params = json!({ "connection": conn, "database": db });
        let mut steps = Vec::new();
        if objects {
            steps.push(("database.objects", params.clone()));
        }
        if tables {
            steps.push(("database.tables", params));
        }
        self.spawn_bg(steps, Then::LoadedDb);
    }

    /// Hand finished tasks' results to their tabs and caches. Called at the start of each frame.
    fn poll_tasks(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        for i in 0..self.state.tabs.len() {
            let Some(task) = self.state.tabs.get(i).and_then(tab_task).cloned() else { continue };
            let Some(done) = task.take() else { continue };
            changed = true;
            let tasks::Done { result, mut session, .. } = done;
            let messages = std::mem::take(&mut session.messages);
            self.session.absorb(session);
            self.session.messages.extend(messages.iter().cloned());
            match self.state.tabs.get_mut(i) {
                Some(Tab::Query(q)) => {
                    q.task = None;
                    Self::query_done(q, result, messages);
                }
                Some(Tab::Table(t)) => {
                    t.task = None;
                    Self::table_done(t, result);
                }
                Some(Tab::Console(c)) => {
                    c.task = None;
                    Self::console_done(c, result);
                }
                _ => {}
            }
        }
        let mut k = 0;
        while k < self.bg.len() {
            let Some(done) = self.bg.get(k).and_then(|(t, _)| t.take()) else {
                k += 1;
                continue;
            };
            changed = true;
            let (_, then) = self.bg.remove(k);
            let tasks::Done { result, session, cancelled } = done;
            self.session.absorb(session);
            match then {
                Then::Opened(id) => self.opened(ctx, &id, result, cancelled),
                Then::LoadedDb => {
                    if let Err(e) = result
                        && !cancelled
                    {
                        self.state.status = Some(e);
                    }
                }
                Then::Ddl { schema, table } => {
                    if self.state.objects.selected == Some((schema, table)) {
                        self.state.objects.ddl = Some(match result {
                            Ok(v) => v["ddl"].as_str().unwrap_or("").to_string(),
                            Err(e) => format!("-- {e}"),
                        });
                    }
                }
            }
        }
        if changed {
            ctx.request_repaint();
        }
    }

    /// A background `connection.open` finished: run what waited for it, or report why not.
    fn opened(&mut self, ctx: &egui::Context, id: &str, result: Result<Json, String>, cancelled: bool) {
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.after_open).into_iter().partition(|(c, _)| c == id);
        self.after_open = rest;
        match result {
            Ok(_) => {
                self.state.status = None;
                // Table tabs opened before the connection was ready.
                let pending: Vec<usize> = (0..self.state.tabs.len())
                    .filter(|i| matches!(self.state.tabs.get(*i), Some(Tab::Table(t)) if t.connection == id && t.page.is_none() && t.task.is_none()))
                    .collect();
                for i in pending {
                    self.reload_table(i);
                }
                self.apply(ctx, mine.into_iter().map(|(_, a)| a).collect());
            }
            Err(e) => {
                if !cancelled {
                    self.state.status = Some(e.clone());
                }
                for t in &mut self.state.tabs {
                    if let Tab::Table(t) = t
                        && t.connection == id
                        && t.page.is_none()
                    {
                        t.error = Some(e.clone());
                    }
                }
            }
        }
    }

    /// Stop (Ctrl+T): the active tab's task, or on the Objects tab everything opening/loading.
    fn stop(&mut self) {
        match self.state.tabs.get(self.state.active_tab).and_then(tab_task) {
            Some(t) => t.cancel(),
            None => {
                for (t, _) in &self.bg {
                    t.cancel();
                }
            }
        }
    }

    fn apply_table(&mut self, i: usize) {
        let Some(Tab::Table(tab)) = self.state.tabs.get_mut(i) else { return };
        let Some(params) = crate::table_tab::edit_params(tab) else { return };
        match self.session.dispatch("table.applyEdits", &params) {
            Ok(_) => {
                tab.discard();
                self.reload_table(i);
                // Row counts in the Objects tab may have changed.
                if let Some(Tab::Table(tab)) = self.state.tabs.get(i)
                    && let Some(db) = tab.database.clone()
                {
                    self.session.tables.remove(&(tab.connection.clone(), db.clone()));
                    self.tried.remove(&format!("t/{}/{db}", tab.connection));
                }
            }
            Err(e) => tab.error = Some(e.to_string()),
        }
    }

    fn ui_command(&mut self, ctx: &egui::Context, id: &str) {
        match id {
            "ui.newQuery" => self.new_query(None, None),
            "ui.exit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "ui.toggleNavigation" => self.state.show_navigation ^= true,
            "ui.toggleInfo" => self.state.show_info ^= true,
            "ui.closeAllTabs" => {
                self.state.tabs.truncate(1);
                self.state.active_tab = 0;
            }
            "ui.about" => self.state.show_about = true,
            "ui.toggleMessages" => self.new_query(None, None),
            "dialog.newConnection" => self.state.connection_form = Some(ConnectionForm::new(DbKind::Mysql)),
            "ui.objects.queries" => self.apply(ctx, vec![Action::SetObjectsMode(ObjectsMode::Queries)]),
            "ui.objects.backups" => self.apply(ctx, vec![Action::SetObjectsMode(ObjectsMode::Backups)]),
            "ui.objects.users" => self.apply(ctx, vec![Action::SetObjectsMode(ObjectsMode::Users)]),
            "ui.automation" => self.apply(ctx, vec![Action::OpenAutomation]),
            "ui.transfer" => self.apply(ctx, vec![Action::OpenTransfer]),
            "ui.console" => {
                if let Some(sel) = self.state.selection.clone() {
                    self.apply(ctx, vec![Action::OpenConsole(sel.connection, sel.database)]);
                } else {
                    self.state.status = Some("Select a connection first, then Tools › Console.".into());
                }
            }
            "ui.objects.table" | "ui.objects.view" => {
                self.state.objects.mode = ObjectsMode::Objects;
                if let Some(sel) = self.state.selection.clone()
                    && sel.database.is_some()
                {
                    let group = if id == "ui.objects.view" { ObjectKind::View } else { ObjectKind::Table };
                    self.select(Selection { group, ..sel });
                }
            }
            _ => {}
        }
    }

    /// Menu items that need a dialog or the current selection rather than direct dispatch.
    fn menu_item(&self, id: &'static str) -> Option<Action> {
        let selected = self.state.selection.as_ref().map(|s| s.connection.clone());
        match id {
            "connection.save" => Some(Action::Ui("dialog.newConnection")),
            "connection.open" | "connection.close" | "connection.delete" => selected.map(|c| Action::Command(id, json!({ "id": c }))),
            "database.objects" => self
                .state
                .selection
                .as_ref()
                .and_then(|s| s.database.as_ref().map(|d| Action::Command("database.tables", json!({ "connection": s.connection, "database": d })))),
            "tools.dataTransfer" => Some(Action::Ui("ui.transfer")),
            "backup.create" => Some(Action::Ui("ui.objects.backups")),
            "automation.save" => Some(Action::Ui("ui.automation")),
            "user.list" => Some(Action::Ui("ui.objects.users")),
            "query.save" => matches!(self.state.tabs.get(self.state.active_tab), Some(Tab::Query(_))).then_some(Action::SaveQuery(self.state.active_tab)),
            "export.resultCsv" => rfd::FileDialog::new()
                .add_filter("CSV", &["csv"])
                .set_file_name("result.csv")
                .save_file()
                .map(|p| Action::Command(id, json!({ "path": p.to_string_lossy() }))),
            id if crate::UI_COMMANDS.contains(&id) => Some(Action::Ui(id)),
            _ => None,
        }
    }

    fn menu_bar(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let engine = crate::engine_command_ids();
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in MENUS {
                ui.menu_button(*menu, |ui| {
                    for item in CATALOG.iter().filter(|i| i.path.split('/').next() == Some(menu)) {
                        let label = item.path.rsplit('/').next().unwrap_or(item.path);
                        let live = engine.contains(&item.id) || crate::UI_COMMANDS.contains(&item.id);
                        if ui.add_enabled(live, egui::Button::new(label)).clicked() {
                            if let Some(a) = self.menu_item(item.id) {
                                actions.push(a);
                            }
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    fn toolbar(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, (glyph, label, color, action)) in icons::TOOLBAR.iter().enumerate() {
                if i == 2 || i == 8 {
                    ui.separator();
                }
                let enabled = !action.is_empty();
                let r = toolbar_button(ui, glyph, label, *color, enabled);
                let r = if enabled { r } else { r.on_hover_text("Coming soon (ROADMAP.md)") };
                if r.clicked() {
                    actions.push(Action::Ui(action));
                }
            }
        });
    }

    fn navigation(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let sel = self.state.selection.clone();
        // Tree rows: hover/press show a fill only (no border), like the Windows tree view.
        let w = &mut ui.visuals_mut().widgets;
        w.hovered.bg_stroke = egui::Stroke::NONE;
        w.active.bg_stroke = egui::Stroke::NONE;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let conns = self.session.workspace.connections.clone();
            for cfg in &conns {
                let open = self.session.is_open(&cfg.id);
                let id = egui::Id::new(("conn", &cfg.id));
                let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
                if !open && !self.tree_started {
                    // Restored expanded from the last run: don't auto-connect at launch.
                    state.set_open(false);
                }
                let opening = self.opening(&cfg.id);
                if !open && !opening && state.is_open() && self.tried.contains(&format!("c/{}", cfg.id)) {
                    // Opening failed earlier; keep it collapsed until asked again.
                    state.set_open(false);
                }
                let header = state.show_header(ui, |ui| {
                    let is_sel = sel.as_ref().is_some_and(|s| s.connection == cfg.id && s.database.is_none());
                    let (glyph, color) = icons::connection_glyph(cfg.kind, open);
                    let mut text = icon_job(glyph, color, 15.0, &cfg.name);
                    if cfg.ssh.is_some() {
                        text.append(&format!("  {}", icons::LOCK_KEY), 0.0, egui::TextFormat { color: Tokens::TEXT_WEAK, ..Default::default() });
                    }
                    ui.selectable_label(is_sel, text)
                });
                let (_, header_resp, _) = header.body(|ui| {
                    if !open {
                        if self.tried.insert(format!("c/{}", cfg.id)) {
                            actions.push(Action::Command("connection.open", json!({ "id": cfg.id })));
                        }
                        if opening {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.weak("Connecting…");
                                if ui.small_button("Stop").clicked() {
                                    actions.push(Action::StopOpening(cfg.id.clone()));
                                }
                            });
                            return;
                        }
                        if ui.button("Open Connection").clicked() {
                            actions.push(Action::Command("connection.open", json!({ "id": cfg.id })));
                        }
                        return;
                    }
                    for db in self.session.databases.get(&cfg.id).into_iter().flatten() {
                        self.database_node(ui, cfg.id.as_str(), cfg.kind, db, sel.as_ref(), actions);
                    }
                });
                let r = header_resp.inner;
                if r.clicked() {
                    actions.push(Action::Select(Selection { connection: cfg.id.clone(), database: None, group: ObjectKind::Table }));
                }
                if r.double_clicked() && !open {
                    self.tried.remove(&format!("c/{}", cfg.id));
                    actions.push(Action::Command("connection.open", json!({ "id": cfg.id })));
                    if let Some(mut s) = CollapsingState::load(ui.ctx(), id) {
                        s.set_open(true);
                        s.store(ui.ctx());
                    }
                }
                r.context_menu(|ui| {
                    if open {
                        if ui.button("Close Connection").clicked() {
                            actions.push(Action::Command("connection.close", json!({ "id": cfg.id })));
                            ui.close();
                        }
                        if ui.button(format!("{} New Query", icons::FILE_SQL)).clicked() {
                            actions.push(Action::NewQuery { connection: Some(cfg.id.clone()), database: None });
                            ui.close();
                        }
                        if ui.button(format!("{} Console", egui_phosphor::regular::TERMINAL_WINDOW)).clicked() {
                            actions.push(Action::OpenConsole(cfg.id.clone(), None));
                            ui.close();
                        }
                        if cfg.kind != DbKind::Sqlite && ui.button(format!("{} Users", egui_phosphor::regular::USERS)).clicked() {
                            actions.push(Action::Select(Selection { connection: cfg.id.clone(), database: None, group: ObjectKind::Table }));
                            actions.push(Action::SetObjectsMode(ObjectsMode::Users));
                            ui.close();
                        }
                    } else if ui.button("Open Connection").clicked() {
                        self.tried.remove(&format!("c/{}", cfg.id));
                        actions.push(Action::Command("connection.open", json!({ "id": cfg.id })));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(format!("{} Edit Connection…", icons::PENCIL_SIMPLE)).clicked() {
                        actions.push(Action::EditConnection(cfg.id.clone()));
                        ui.close();
                    }
                    if ui.button(format!("{} Delete Connection", icons::TRASH)).clicked() {
                        actions.push(Action::Command("connection.delete", json!({ "id": cfg.id })));
                        ui.close();
                    }
                });
            }
            self.tree_started = true;
            if self.session.workspace.connections.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.weak("No connections yet.");
                    if ui.button(format!("{} New Connection", icons::PLUG)).clicked() {
                        actions.push(Action::Ui("dialog.newConnection"));
                    }
                });
            }
        });
    }

    fn database_node(&self, ui: &mut Ui, conn: &str, kind: DbKind, db: &str, sel: Option<&Selection>, actions: &mut Vec<Action>) {
        let key = (conn.to_string(), db.to_string());
        let loaded = self.session.objects.contains_key(&key);
        let id = egui::Id::new(("db", conn, db));
        let state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
        let header = state.show_header(ui, |ui| {
            let is_sel = sel.is_some_and(|s| s.connection == conn && s.database.as_deref() == Some(db) && s.group == ObjectKind::Table);
            let (glyph, color) = icons::database_glyph(loaded);
            let job = icon_job(glyph, color, 14.0, db);
            ui.selectable_label(is_sel, job)
        });
        let (_, resp, _) = header.body(|ui| {
            let Some(objs) = self.session.objects.get(&key) else {
                if self.tried.contains(&format!("d/{conn}/{db}")) {
                    ui.weak("(could not load objects)");
                } else {
                    actions.push(Action::LoadDb(conn.to_string(), db.to_string()));
                    ui.spinner();
                }
                return;
            };
            for (group, title) in GROUPS {
                let items: Vec<_> = objs.iter().filter(|o| o.kind == *group).collect();
                if items.is_empty() && !matches!(group, ObjectKind::Table | ObjectKind::View) {
                    continue;
                }
                let gid = egui::Id::new(("grp", conn, db, *title));
                let gstate = CollapsingState::load_with_default_open(ui.ctx(), gid, false);
                let gh = gstate.show_header(ui, |ui| {
                    let is_sel = sel.is_some_and(|s| s.connection == conn && s.database.as_deref() == Some(db) && s.group == *group);
                    let (glyph, color) = icons::object_glyph(*group);
                    let job = icon_job(glyph, color, 14.0, title);
                    ui.selectable_label(is_sel, job)
                });
                let (_, gresp, _) = gh.body(|ui| {
                    for o in items {
                        let label = match &o.schema {
                            Some(s) if kind == DbKind::Postgres && s != "public" => format!("{s}.{}", o.name),
                            _ => o.name.clone(),
                        };
                        let (glyph, color) = icons::object_glyph(o.kind);
                        let job = icon_job(glyph, color, 13.0, &label);
                        let r = ui.selectable_label(false, job);
                        let schema = if kind == DbKind::Postgres { o.schema.clone() } else { None };
                        if r.double_clicked() && matches!(o.kind, ObjectKind::Table | ObjectKind::View) {
                            actions.push(Action::OpenTable { connection: conn.to_string(), database: db.to_string(), schema, table: o.name.clone() });
                        } else if r.clicked() {
                            actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: o.kind }));
                        }
                    }
                });
                if gresp.inner.clicked() {
                    actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: *group }));
                    actions.push(Action::SetObjectsMode(ObjectsMode::Objects));
                }
            }
            // Saved queries and backups (Navicat shows both under the database).
            let queries: Vec<_> = self.session.workspace.queries.iter().filter(|q| q.connection == conn && q.database.as_deref() == Some(db)).collect();
            let qid = egui::Id::new(("grp", conn, db, "Queries"));
            let qh = CollapsingState::load_with_default_open(ui.ctx(), qid, false).show_header(ui, |ui| {
                let is_sel = self.state.objects.mode == ObjectsMode::Queries && sel.is_some_and(|s| s.connection == conn && s.database.as_deref() == Some(db));
                ui.selectable_label(is_sel, icon_job(icons::FILE_SQL, Tokens::ACCENT, 14.0, &format!("Queries ({})", queries.len())))
            });
            let (_, qresp, _) = qh.body(|ui| {
                for q in queries {
                    let r = ui.selectable_label(false, icon_job(icons::FILE_SQL, Tokens::ACCENT, 13.0, &q.name));
                    if r.double_clicked() {
                        actions.push(Action::OpenSavedQuery(q.id.clone()));
                    }
                }
            });
            if qresp.inner.clicked() {
                actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: ObjectKind::Table }));
                actions.push(Action::SetObjectsMode(ObjectsMode::Queries));
            }
            let is_sel = self.state.objects.mode == ObjectsMode::Backups && sel.is_some_and(|s| s.connection == conn && s.database.as_deref() == Some(db));
            let br = ui.selectable_label(is_sel, icon_job(egui_phosphor::regular::ARCHIVE, egui::Color32::from_rgb(0x27, 0xae, 0x60), 14.0, "Backups"));
            if br.clicked() {
                actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: ObjectKind::Table }));
                actions.push(Action::SetObjectsMode(ObjectsMode::Backups));
            }
        });
        let r = resp.inner;
        if r.clicked() {
            actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: ObjectKind::Table }));
            actions.push(Action::SetObjectsMode(ObjectsMode::Objects));
        }
        r.context_menu(|ui| {
            if ui.button(format!("{} New Query", icons::FILE_SQL)).clicked() {
                actions.push(Action::NewQuery { connection: Some(conn.to_string()), database: Some(db.to_string()) });
                ui.close();
            }
            if ui.button(format!("{} Console", egui_phosphor::regular::TERMINAL_WINDOW)).clicked() {
                actions.push(Action::OpenConsole(conn.to_string(), Some(db.to_string())));
                ui.close();
            }
            if ui.button(format!("{} Backups", egui_phosphor::regular::ARCHIVE)).clicked() {
                actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: ObjectKind::Table }));
                actions.push(Action::SetObjectsMode(ObjectsMode::Backups));
                ui.close();
            }
            if ui.button(format!("{} Data Transfer…", egui_phosphor::regular::ARROWS_LEFT_RIGHT)).clicked() {
                actions.push(Action::Select(Selection { connection: conn.to_string(), database: Some(db.to_string()), group: ObjectKind::Table }));
                actions.push(Action::OpenTransfer);
                ui.close();
            }
            ui.separator();
            if ui.button(format!("{} Refresh", icons::ARROWS_CLOCKWISE)).clicked() {
                actions.push(Action::Command("database.objects", json!({ "connection": conn, "database": db })));
                actions.push(Action::Command("database.tables", json!({ "connection": conn, "database": db })));
                ui.close();
            }
        });
    }

    fn tab_strip(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let mut close = None;
        egui::Frame::new().fill(Tokens::TOOLBAR_BG).inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 0 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for (i, t) in self.state.tabs.iter().enumerate() {
                    let (icon, title, dirty) = match t {
                        Tab::Objects => (RichText::new(egui_phosphor::regular::SQUARES_FOUR).color(Tokens::ACCENT), "Objects".to_string(), false),
                        Tab::Table(tt) => (icons::object(ObjectKind::Table), tt.title(), tt.dirty()),
                        Tab::Query(q) => (RichText::new(icons::FILE_SQL).color(Tokens::ACCENT), q.title.clone(), q.dirty()),
                        Tab::Console(c) => {
                            let name = self.session.workspace.connection(&c.connection).map(|x| x.name.clone()).unwrap_or_default();
                            (RichText::new(egui_phosphor::regular::TERMINAL_WINDOW).color(Tokens::TEXT), format!("Console - {name}"), false)
                        }
                        Tab::Transfer(_) => (RichText::new(egui_phosphor::regular::ARROWS_LEFT_RIGHT).color(Tokens::ACCENT), "Data Transfer".into(), false),
                        Tab::Automation(_) => {
                            (RichText::new(egui_phosphor::regular::ROBOT).color(egui::Color32::from_rgb(0xd4, 0x8a, 0x17)), "Automation".into(), false)
                        }
                        Tab::User(u) => (RichText::new(egui_phosphor::regular::USER).color(Tokens::TEXT), u.title(), false),
                    };
                    let active = i == self.state.active_tab;
                    let fill = if active { Tokens::BG } else { Tokens::TAB_INACTIVE };
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, Tokens::BORDER))
                        .corner_radius(egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 })
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(icon);
                                let label = if dirty { format!("{title} *") } else { title };
                                if ui.add(egui::Label::new(label).sense(egui::Sense::click())).clicked() {
                                    self.state.active_tab = i;
                                }
                                if i > 0 && ui.add(egui::Button::new(RichText::new(icons::X).small()).frame(false)).clicked() {
                                    close = Some(i);
                                }
                            });
                        });
                }
            });
        });
        if let Some(i) = close {
            actions.push(Action::CloseTab(i));
        }
    }

    fn central(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        self.tab_strip(ui, actions);
        let idx = self.state.active_tab;
        let open_conns: Vec<(String, String)> =
            self.session.workspace.connections.iter().filter(|c| self.session.is_open(&c.id)).map(|c| (c.id.clone(), c.name.clone())).collect();
        let Some(tab) = self.state.tabs.get_mut(idx) else { return };
        match tab {
            Tab::Objects => crate::objects_tab::show(ui, &self.session, self.state.selection.as_ref(), &mut self.state.objects, self.state.show_info, actions),
            Tab::Table(tt) => {
                let dialect = crate::query_tab::dialect_of(&self.session, Some(&tt.connection));
                crate::table_tab::show(ui, idx, tt, dialect, actions)
            }
            Tab::Query(q) => crate::query_tab::show(ui, idx, q, &open_conns, &self.session, actions),
            Tab::Console(c) => crate::console_tab::show(ui, idx, c, &self.session, actions),
            Tab::Transfer(f) => crate::transfer_tab::show(ui, idx, f, &self.session, actions),
            Tab::Automation(v) => crate::automation_tab::show(ui, idx, v, &self.session, actions),
            Tab::User(u) => {
                let users = match &self.state.objects.users {
                    Some(Ok(l)) => l.clone(),
                    _ => Vec::new(),
                };
                crate::user_tab::show(ui, idx, u, &self.session, &users, actions)
            }
        }
    }
}

/// The running task of a query, table or console tab.
fn tab_task(t: &Tab) -> Option<&Task> {
    match t {
        Tab::Query(q) => q.task.as_ref(),
        Tab::Table(t) => t.task.as_ref(),
        Tab::Console(c) => c.task.as_ref(),
        _ => None,
    }
}

/// A Navicat-style toolbar button: large tinted icon above a small label, sized to its label.
fn toolbar_button(ui: &mut Ui, glyph: &str, label: &str, color: egui::Color32, enabled: bool) -> egui::Response {
    let icon_font = egui::FontId::proportional(24.0);
    let label_font = egui::FontId::proportional(12.0);
    let (icon_color, text_color) = if enabled { (color, Tokens::TEXT) } else { (Tokens::CLOSED, Tokens::TEXT_WEAK) };
    let icon = ui.painter().layout_no_wrap(glyph.to_string(), icon_font, icon_color);
    let text = ui.painter().layout_no_wrap(label.to_string(), label_font, text_color);
    let size = egui::vec2(text.size().x.max(icon.size().x).max(44.0) + 14.0, 54.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect(rect, 4.0, Tokens::HOVER, egui::Stroke::new(1.0, Tokens::SELECTION_STROKE), egui::StrokeKind::Inside);
    }
    let icon_pos = egui::pos2(rect.center().x - icon.size().x / 2.0, rect.top() + 5.0);
    let text_pos = egui::pos2(rect.center().x - text.size().x / 2.0, rect.bottom() - text.size().y - 4.0);
    ui.painter().galley(icon_pos, icon, icon_color);
    ui.painter().galley(text_pos, text, text_color);
    resp
}

/// Coloured icon glyph followed by a label, for tree rows.
fn icon_job(glyph: &str, color: egui::Color32, size: f32, label: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(glyph, 0.0, egui::TextFormat { color, font_id: egui::FontId::proportional(size), valign: egui::Align::Center, ..Default::default() });
    job.append(
        &format!("  {label}"),
        0.0,
        egui::TextFormat { color: Tokens::TEXT, font_id: egui::FontId::proportional(13.0), valign: egui::Align::Center, ..Default::default() },
    );
    job
}

impl eframe::App for DbCraftApp {
    fn ui(&mut self, root: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        if !self.themed {
            crate::theme::apply(ctx);
            self.themed = true;
        }
        self.ctx_for_jobs = ctx.clone();
        self.poll_jobs();
        self.poll_tasks(ctx);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::T)) {
            self.stop();
        }
        let mut actions = std::mem::take(&mut self.startup);
        egui::Panel::top("menu").frame(egui::Frame::new().fill(Tokens::TOOLBAR_BG).inner_margin(egui::Margin::symmetric(6, 2))).show(root, |ui| {
            self.menu_bar(ui, &mut actions);
            ui.add_space(2.0);
            self.toolbar(ui, &mut actions);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(Tokens::TOOLBAR_BG).inner_margin(egui::Margin::symmetric(8, 3))).show(root, |ui| {
            ui.horizontal(|ui| match &self.state.status {
                Some(e) => {
                    ui.colored_label(Tokens::ERROR, format!("{} {e}", icons::X));
                }
                None => {
                    let open = self.session.workspace.connections.iter().filter(|c| self.session.is_open(&c.id)).count();
                    let sel = self.state.selection.as_ref().and_then(|s| {
                        let c = self.session.workspace.connection(&s.connection).ok()?;
                        Some(match &s.database {
                            Some(d) => format!("{}  ›  {d}", c.name),
                            None => c.name.clone(),
                        })
                    });
                    ui.weak(sel.unwrap_or_default());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(format!("{} connections, {open} open", self.session.workspace.connections.len()));
                    });
                }
            });
        });
        if self.state.show_navigation {
            egui::Panel::left("navigation")
                .resizable(true)
                .default_size(270.0)
                .frame(egui::Frame::new().fill(Tokens::PANEL_BG).inner_margin(egui::Margin::same(4)))
                .show(root, |ui| self.navigation(ui, &mut actions));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Tokens::BG)).show(root, |ui| self.central(ui, &mut actions));

        if let Some(form) = &mut self.state.connection_form {
            match conn_dialog::show(ctx, form) {
                Outcome::Open => {}
                Outcome::Cancel => self.state.connection_form = None,
                Outcome::Save(params) => {
                    if let Some(v) = self.run("connection.save", &params) {
                        let id = v["id"].as_str().unwrap_or("").to_string();
                        // Re-open with the new settings if it was open.
                        if self.session.is_open(&id) {
                            self.run("connection.close", &json!({ "id": id }));
                        }
                        self.tried.retain(|k| !k.contains(&id));
                        self.state.connection_form = None;
                    }
                }
            }
        }
        if let Some(i) = self.state.confirm_close {
            let title = match self.state.tabs.get(i) {
                Some(Tab::Query(q)) => q.title.clone(),
                _ => String::new(),
            };
            let mut open = true;
            egui::Window::new("Save Changes")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    ui.label(format!("Do you want to save changes to `{title}`?"));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Save").strong()).clicked() {
                            if let Some(Tab::Query(q)) = self.state.tabs.get_mut(i) {
                                q.close_after_save = true;
                            }
                            actions.push(Action::SaveQuery(i));
                            self.state.confirm_close = None;
                        }
                        if ui.button("Don't Save").clicked() {
                            self.state.confirm_close = None;
                            self.close_tab(i, true);
                        }
                        if ui.button("Cancel").clicked() {
                            self.state.confirm_close = None;
                        }
                    });
                });
            if !open {
                self.state.confirm_close = None;
            }
        }
        if self.state.show_about {
            egui::Window::new("About DbCraft").open(&mut self.state.show_about).collapsible(false).show(ctx, |ui| {
                ui.heading("DbCraft");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A native database manager in pure Rust.");
                ui.weak("Icons: Phosphor (MIT).");
            });
        }

        // Selection actions from the tree can repeat within a frame; keep the last.
        let mut seen_select = false;
        let mut deduped = Vec::with_capacity(actions.len());
        for a in actions.into_iter().rev() {
            if matches!(a, Action::Select(_)) {
                if seen_select {
                    continue;
                }
                seen_select = true;
            }
            deduped.push(a);
        }
        deduped.reverse();
        if !deduped.is_empty() {
            // State changed after this frame was drawn: draw again so the screen never lags behind.
            ctx.request_repaint();
        }
        self.apply(ctx, deduped);
    }
}
