//! Backup / Restore (Navicat's built-in backup, stored as SQL scripts named YYYYMMDDhhmmss).
//! Files live in `<data dir>/backups/<connection>/<database>/`.

use std::io::Write;
use std::path::{Path, PathBuf};

use dbcraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::Session;
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
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "backup.create",
            label: "New Backup",
            menu: "Tools/Backup",
            shortcut: None,
            params_doc: r#"{"connection":str,"database":str,"tables"?:[{"name","schema"?}] (default all),"comment"?:str,"path"?:file} -> BackupInfo"#,
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
            params_doc: r#"{"connection":str,"database":str,"path":file,"continue_on_error"?:bool} -> {"statements":int,"errors":[str]} (replaces the backed-up tables)"#,
            enabled: always,
            run: restore,
        },
        CommandSpec { id: "backup.delete", label: "Delete Backup", menu: "", shortcut: None, params_doc: r#"{"path":file}"#, enabled: always, run: delete },
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
        ..BackupInfo::default()
    };
    if let Ok(f) = std::fs::File::open(path) {
        use std::io::BufRead;
        for line in std::io::BufReader::new(f).lines().take(5).map_while(std::result::Result::ok) {
            if let Some(js) = line.strip_prefix(HEADER)
                && let Ok(h) = serde_json::from_str::<BackupInfo>(js)
            {
                info = BackupInfo { file: info.file, path: info.path, size: info.size, ..h };
            }
        }
    }
    info
}

fn create(s: &mut Session, p: &Json) -> Result<Json> {
    let conn = str_req(p, "connection")?.to_string();
    let db = str_req(p, "database")?.to_string();
    let comment = str_opt(p, "comment")?.unwrap_or("").to_string();
    let tables = tables_param(s, p, &conn, Some(&db))?;
    let dialect = s.conn_auto(&conn)?.dialect();
    let now = chrono::Local::now();
    let path = match str_opt(p, "path")? {
        Some(p) => PathBuf::from(p),
        None => {
            // Navicat-style name; never overwrite an existing backup made in the same second.
            let dir = backup_dir(s, &conn, &db)?;
            let stamp = now.format("%Y%m%d%H%M%S").to_string();
            let mut path = dir.join(format!("{stamp}.sql"));
            let mut n = 2;
            while path.exists() {
                path = dir.join(format!("{stamp}_{n}.sql"));
                n += 1;
            }
            path
        }
    };
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
        Ok(entries) => {
            entries.filter_map(std::result::Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "sql")).map(|p| read_info(&p)).collect()
        }
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
    if !file.starts_with(&root) || file.extension().is_none_or(|x| x != "sql") {
        return Err(Error::InvalidParam("only backup files in the backups folder can be deleted".into()));
    }
    std::fs::remove_file(file)?;
    Ok(json!({}))
}
