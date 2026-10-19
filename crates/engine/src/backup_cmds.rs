//! Backup / Restore (Navicat's built-in backup). New backups are `.dbcb` archives (see
//! [`crate::backup_format`]): parallel, compressed, complete. Older `.sql` script backups still
//! list and restore. Files are named YYYYMMDDhhmmss and live in
//! `<data dir>/backups/<connection>/<database>/`.

use std::io::Write;
use std::path::{Path, PathBuf};

use dbcraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::backup_format::{self, EXTENSION};
use crate::commands::{CommandSpec, always};
use crate::params::{str_opt, str_req};
use crate::transfer::{self, Options, Sink};
use crate::transfer_cmds::tables_param;

const HEADER: &str = "-- dbcraft-backup: ";

/// What the backup list shows; read from the file's header line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupInfo {
    pub file: String,
    pub path: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub tables: usize,
    #[serde(default)]
    pub rows: u64,
    #[serde(default)]
    pub dialect: String,
    /// `dbcb` (archive) or `sql` (script, older backups).
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub views: usize,
    /// False for a backup that was interrupted while written.
    #[serde(default)]
    pub complete: bool,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "backup.create",
            label: "New Backup",
            menu: "Tools/Backup",
            shortcut: None,
            params_doc: r#"{"connection":str,"database":str,"tables"?:[{"name","schema"?}] (default all),"views"?:bool|[str] (default all),"comment"?:str,"file_name"?:str (Use specified file name),"path"?:file,"single_transaction"?:bool,"lock_tables"?:bool,"format"?:"dbcb"|"sql"} -> BackupInfo"#,
            enabled: always,
            run: create,
        },
        CommandSpec {
            id: "backup.list",
            label: "Backups",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database":str} -> [BackupInfo] (newest first)"#,
            enabled: always,
            run: list,
        },
        CommandSpec {
            id: "backup.restore",
            label: "Restore Backup",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database":str,"path":file,"tables"?:[str] (default all),"continue_on_error"?:bool} -> {"statements":int,"errors":[str],"warnings":[str]} (replaces the backed-up tables and views)"#,
            enabled: always,
            run: restore,
        },
        CommandSpec { id: "backup.delete", label: "Delete Backup", menu: "", shortcut: None, params_doc: r#"{"path":file}"#, enabled: always, run: delete },
        CommandSpec {
            id: "backup.extractSql",
            label: "Extract SQL",
            menu: "",
            shortcut: None,
            params_doc: r#"{"path":file (.dbcb),"out":file (.sql)} -> BackupInfo"#,
            enabled: always,
            run: extract_sql,
        },
    ]
}

fn backup_dir(s: &Session, conn: &str, db: &str) -> Result<PathBuf> {
    let base = s.data_dir.clone().ok_or_else(|| Error::InvalidParam("no data folder for backups: pass `path`".into()))?;
    let clean = |x: &str| x.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect::<String>();
    Ok(base.join("backups").join(clean(conn)).join(clean(db)))
}

fn read_info(path: &Path) -> BackupInfo {
    let mut info = BackupInfo {
        file: path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        path: path.to_string_lossy().into_owned(),
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        format: "sql".into(),
        complete: true,
        ..BackupInfo::default()
    };
    if path.extension().is_some_and(|x| x == EXTENSION) {
        info.format = EXTENSION.into();
        info.complete = false;
        if let Ok(m) = backup_format::read_manifest(path) {
            info.created = m.created;
            info.comment = m.comment;
            info.tables = m.tables.len();
            info.views = m.views.len();
            info.rows = m.rows;
            info.dialect = m.dialect;
            info.complete = m.complete;
        }
        return info;
    }
    if let Ok(f) = std::fs::File::open(path) {
        use std::io::BufRead;
        for line in std::io::BufReader::new(f).lines().take(5).map_while(std::result::Result::ok) {
            if let Some(js) = line.strip_prefix(HEADER)
                && let Ok(h) = serde_json::from_str::<BackupInfo>(js)
            {
                info = BackupInfo { file: info.file, path: info.path, size: info.size, format: "sql".into(), complete: true, ..h };
            }
        }
    }
    info
}

fn bool_param(p: &Json, key: &str) -> bool {
    p.get(key).and_then(Json::as_bool).unwrap_or(false)
}

/// Where a new backup goes: `path`, `file_name` in the backups folder (Navicat's "Use specify
/// file name"), or the next free YYYYMMDDhhmmss name.
fn new_backup_path(s: &Session, p: &Json, conn: &str, db: &str, ext: &str) -> Result<PathBuf> {
    if let Some(path) = str_opt(p, "path")? {
        return Ok(PathBuf::from(path));
    }
    let dir = backup_dir(s, conn, db)?;
    if let Some(name) = str_opt(p, "file_name")?.map(str::trim).filter(|n| !n.is_empty()) {
        // A plain file name: no folders, nothing that escapes the backups folder.
        if name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Err(Error::InvalidParam("the file name must not contain a folder".into()));
        }
        let file = if name.to_ascii_lowercase().ends_with(&format!(".{ext}")) { name.to_string() } else { format!("{name}.{ext}") };
        return Ok(dir.join(file));
    }
    // Never overwrite a backup made in the same second.
    let stamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let mut path = dir.join(format!("{stamp}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stamp}_{n}.{ext}"));
        n += 1;
    }
    Ok(path)
}

fn create(s: &mut Session, p: &Json) -> Result<Json> {
    if str_opt(p, "format")? == Some("sql") {
        return create_sql(s, p);
    }
    let conn = str_req(p, "connection")?.to_string();
    let db = str_req(p, "database")?.to_string();
    let tables = tables_param(s, p, &conn, Some(&db))?;
    let views = match p.get("views") {
        None | Some(Json::Null) | Some(Json::Bool(true)) => None,
        Some(Json::Bool(false)) => Some(Vec::new()),
        Some(v) => Some(serde_json::from_value::<Vec<String>>(v.clone()).map_err(|e| Error::InvalidParam(format!("`views`: {e}")))?),
    };
    let path = new_backup_path(s, p, &conn, &db, EXTENSION)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    s.report(&format!("Backup {db} -> {}", path.display()));
    let spec = backup_format::BackupSpec {
        conn: s.conn_auto(&conn)?,
        connection: &conn,
        database: &db,
        tables: &tables,
        views,
        comment: str_opt(p, "comment")?.unwrap_or(""),
        single_transaction: bool_param(p, "single_transaction"),
        lock_tables: bool_param(p, "lock_tables"),
        progress: s.progress.clone(),
    };
    let m = backup_format::create(&spec, &path)?;
    s.report(&format!("Backup finished: {} tables, {} views, {} rows, {} chunks", m.tables.len(), m.views.len(), m.rows, m.chunks));
    serde_json::to_value(read_info(&path)).map_err(|e| Error::Io(e.to_string()))
}

/// The older SQL-script format (`"format": "sql"`).
fn create_sql(s: &mut Session, p: &Json) -> Result<Json> {
    let conn = str_req(p, "connection")?.to_string();
    let db = str_req(p, "database")?.to_string();
    let comment = str_opt(p, "comment")?.unwrap_or("").to_string();
    let tables = tables_param(s, p, &conn, Some(&db))?;
    let dialect = s.conn_auto(&conn)?.dialect();
    let now = chrono::Local::now();
    let path = new_backup_path(s, p, &conn, &db, "sql")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    s.report(&format!("Backup {db} -> {}", path.display()));
    let tmp = path.with_extension("sql.part");
    // Views come with the new backup format (restoring a view defined by another user needs privileges).
    let opts = Options { drop_existing: true, ..Options::default() };
    let mut file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    let mut info =
        BackupInfo { created: now.format("%Y-%m-%d %H:%M:%S").to_string(), comment, dialect: format!("{dialect:?}").to_lowercase(), ..BackupInfo::default() };
    // Header first with placeholders; the summary is appended as a trailer and re-read below.
    writeln!(file, "-- DbCraft backup of `{db}` on `{conn}`")?;
    let result = transfer::run(s, &conn, Some(&db), &tables, &mut Sink::File { out: &mut file, dialect, keep_schema: true }, &opts);
    let summary = match result {
        Ok(sum) => sum,
        Err(e) => {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    info.tables = summary.tables;
    info.rows = summary.rows;
    writeln!(file, "{HEADER}{}", serde_json::to_string(&info).map_err(|e| Error::Io(e.to_string()))?)?;
    file.flush()?;
    drop(file);
    // Put the header line at the top so listing reads only the first lines.
    let body = std::fs::read_to_string(&tmp)?;
    let mut lines: Vec<&str> = body.lines().collect();
    if let Some(pos) = lines.iter().rposition(|l| l.starts_with(HEADER)) {
        let h = lines.remove(pos);
        lines.insert(1, h);
    }
    std::fs::write(&tmp, lines.join("\n") + "\n")?;
    std::fs::rename(&tmp, &path)?;
    let out = read_info(&path);
    s.report(&format!("Backup finished: {} tables, {} rows", out.tables, out.rows));
    serde_json::to_value(out).map_err(|e| Error::Io(e.to_string()))
}

fn list(s: &mut Session, p: &Json) -> Result<Json> {
    let dir = backup_dir(s, str_req(p, "connection")?, str_req(p, "database")?)?;
    let mut v: Vec<BackupInfo> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sql" || x == EXTENSION))
            .map(|p| read_info(&p))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    v.sort_by(|a, b| b.file.cmp(&a.file));
    serde_json::to_value(v).map_err(|e| Error::Io(e.to_string()))
}

fn restore(s: &mut Session, p: &Json) -> Result<Json> {
    let conn = str_req(p, "connection")?.to_string();
    let db = str_req(p, "database")?.to_string();
    let path = str_req(p, "path")?;
    let continue_on_error = p.get("continue_on_error").and_then(Json::as_bool).unwrap_or(false);
    if Path::new(path).extension().is_some_and(|x| x == EXTENSION) {
        let tables = match p.get("tables") {
            None | Some(Json::Null) => None,
            Some(v) => Some(serde_json::from_value::<Vec<String>>(v.clone()).map_err(|e| Error::InvalidParam(format!("`tables`: {e}")))?),
        };
        s.report(&format!("Restore {} into {db}", Path::new(path).display()));
        let spec = backup_format::RestoreSpec { conn: s.conn_auto(&conn)?, database: &db, tables, continue_on_error, progress: s.progress.clone() };
        let sum = backup_format::restore(&spec, Path::new(path))?;
        s.tables.retain(|(c, d), _| !(c == &conn && d == &db));
        s.objects.retain(|(c, d), _| !(c == &conn && d == &db));
        for w in &sum.warnings {
            s.report(&format!("  warning: {w}"));
        }
        return Ok(json!({ "statements": sum.statements, "tables": sum.tables, "errors": sum.errors, "warnings": sum.warnings }));
    }
    let text = std::fs::read_to_string(path)?;
    let dialect = s.conn_auto(&conn)?.dialect();
    let stmts = dbcraft_sql::split_statements(dialect, &text);
    s.report(&format!("Restore {} into {db}: {} statements", Path::new(path).display(), stmts.len()));
    let mut errors = Vec::new();
    for (i, stmt) in stmts.iter().enumerate() {
        if let Err(e) = s.conn_auto(&conn)?.execute_atomic(Some(&db), &[(stmt.clone(), None)]) {
            let msg = format!("statement {}: {e}", i + 1);
            s.report(&format!("  error: {msg}"));
            if !continue_on_error {
                return Err(Error::Database(msg));
            }
            errors.push(msg);
        }
        if (i + 1) % 50 == 0 {
            s.report(&format!("  {} / {}", i + 1, stmts.len()));
        }
    }
    s.tables.retain(|(c, d), _| !(c == &conn && d == &db));
    s.objects.retain(|(c, d), _| !(c == &conn && d == &db));
    s.report(&format!("Restore finished: {} statements, {} errors", stmts.len(), errors.len()));
    Ok(json!({ "statements": stmts.len(), "errors": errors }))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let path = PathBuf::from(str_req(p, "path")?);
    // Only files inside the backups folder: this command must never become "delete any file".
    let root = s.data_dir.clone().ok_or_else(|| Error::InvalidParam("no data folder".into()))?.join("backups");
    let (Ok(root), Ok(file)) = (root.canonicalize(), path.canonicalize()) else {
        return Err(Error::NotFound(format!("backup `{}`", path.display())));
    };
    if !file.starts_with(&root) || file.extension().is_none_or(|x| x != "sql" && x != EXTENSION) {
        return Err(Error::InvalidParam("only backup files in the backups folder can be deleted".into()));
    }
    std::fs::remove_file(file)?;
    Ok(json!({}))
}

fn extract_sql(_: &mut Session, p: &Json) -> Result<Json> {
    let path = PathBuf::from(str_req(p, "path")?);
    let out = PathBuf::from(str_req(p, "out")?);
    if out == path {
        return Err(Error::InvalidParam("`out` must be another file".into()));
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(&out)?);
    backup_format::extract_sql(&path, &mut w)?;
    w.flush()?;
    serde_json::to_value(read_info(&path)).map_err(|e| Error::Io(e.to_string()))
}
