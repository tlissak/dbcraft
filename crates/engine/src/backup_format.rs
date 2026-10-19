//! DbCraft backups (`.dbcb`): fast to write, fast to restore, complete.
//!
//! A tar archive:
//! - `manifest.json`: what's inside (written first with room to spare, completed at the end, so
//!   listing backups reads only the first few KB);
//! - `schema/NNNN.json`: each table's [`TableSchema`], parents before children;
//! - `views.json`;
//! - `data/NNNN/CCCCC.sql.gz`: the rows as gzipped multi-row `INSERT`s, a few MB per chunk, each
//!   statement stored as `<length>\n<sql>\n` (no SQL parsing on restore).
//!
//! Tables are read by several workers at once (keyset paging, compression on the workers) while
//! one thread appends chunks to the archive. Restore creates the tables, loads chunks in parallel
//! (one transaction each), then indexes, foreign keys, sequences, triggers and views. "Use single
//! transaction" and "Lock all tables" read through one dedicated connection for a consistent
//! snapshot.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex, PoisonError};

use dbcraft_core::{Error, ForeignKeyInfo, Result, TableSchema, ViewInfo};
use dbcraft_drivers::Connection;
use dbcraft_sql::{Dialect, quote_ident};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

use crate::transfer::{RowReader, TableRef, insert_statements, qualify};
use crate::{ProgressSink, ddl};

pub const EXTENSION: &str = "dbcb";
const FORMAT: &str = "dbcraft-backup";
/// Room reserved for the manifest (rewritten in place at the end).
const MANIFEST_SIZE: usize = 64 * 1024;
/// Uncompressed SQL per chunk.
const CHUNK: usize = 4 << 20;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub created: String,
    pub comment: String,
    pub connection: String,
    pub database: String,
    /// `mysql`, `postgres` or `sqlite`.
    pub dialect: String,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub rows: u64,
    pub chunks: u64,
    /// False while writing; a backup without it was interrupted.
    pub complete: bool,
    pub warnings: Vec<String>,
}

/// What a backup run takes.
pub struct BackupSpec<'a> {
    pub conn: Arc<dyn Connection>,
    pub connection: &'a str,
    pub database: &'a str,
    pub tables: &'a [TableRef],
    /// View names to include (`None`: all).
    pub views: Option<Vec<String>>,
    pub comment: &'a str,
    pub single_transaction: bool,
    pub lock_tables: bool,
    pub progress: Option<ProgressSink>,
}

fn dialect_name(d: Dialect) -> &'static str {
    match d {
        Dialect::Mysql => "mysql",
        Dialect::Postgres => "postgres",
        Dialect::Sqlite => "sqlite",
        Dialect::Mssql => "mssql",
    }
}

fn report(p: &Option<ProgressSink>, msg: &str) {
    if let Some(p) = p {
        p(msg);
    }
}

fn tar_append(b: &mut tar::Builder<impl Write>, path: &str, data: &[u8]) -> Result<()> {
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_cksum();
    b.append_data(&mut h, path, data).map_err(Error::from)
}

fn padded_manifest(m: &Manifest) -> Result<Vec<u8>> {
    let mut m = m.clone();
    let mut json = serde_json::to_vec(&m).map_err(|e| Error::Io(e.to_string()))?;
    // Keep it inside the reserved room: drop warnings, then shorten the comment.
    while json.len() > MANIFEST_SIZE {
        if m.warnings.pop().is_none() {
            let keep = m.comment.chars().count() / 2;
            m.comment = m.comment.chars().take(keep).collect();
            m.tables.truncate(m.tables.len() / 2);
        }
        json = serde_json::to_vec(&m).map_err(|e| Error::Io(e.to_string()))?;
    }
    json.resize(MANIFEST_SIZE, b' ');
    Ok(json)
}

/// Gzip a list of statements in the chunk encoding.
fn chunk_bytes(stmts: &[String]) -> Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    for s in stmts {
        write!(enc, "{}\n{s}\n", s.len())?;
    }
    enc.finish().map_err(Error::from)
}

/// Statements of a chunk.
fn chunk_statements(data: &[u8]) -> Result<Vec<String>> {
    let mut r = BufReader::new(GzDecoder::new(data));
    let mut out = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(out);
        }
        let len: usize = line.trim().parse().map_err(|_| Error::Io("damaged backup chunk (bad length)".into()))?;
        if len > 256 << 20 {
            return Err(Error::Io("damaged backup chunk (statement too large)".into()));
        }
        let mut buf = vec![0u8; len + 1];
        r.read_exact(&mut buf)?;
        buf.pop();
        out.push(String::from_utf8(buf).map_err(|_| Error::Io("damaged backup chunk (not UTF-8)".into()))?);
    }
}

/// Table names in the backup: schema-qualified on PostgreSQL, bare elsewhere (restore picks the
/// target database).
fn target_name(d: Dialect, t: &TableSchema) -> String {
    qualify(d, None, if d == Dialect::Postgres { t.schema.as_deref() } else { None }, &t.name)
}

fn ref_name(d: Dialect, f: &ForeignKeyInfo, own_schema: Option<&str>) -> String {
    qualify(d, None, if d == Dialect::Postgres { f.ref_schema.as_deref().or(own_schema) } else { None }, &f.ref_table)
}

/// Write a backup to `path` (atomically: `path.part` until complete).
pub fn create(spec: &BackupSpec, path: &Path) -> Result<Manifest> {
    let d = spec.conn.dialect();
    // A consistent snapshot needs one session for every read.
    let snapshot = spec.single_transaction || spec.lock_tables;
    let reader_conn: Arc<dyn Connection> = if snapshot { Arc::from(spec.conn.dedicated()?) } else { spec.conn.clone() };
    let db = Some(spec.database);
    if snapshot {
        begin_snapshot(reader_conn.as_ref(), d, spec)?;
    }
    let result = write_archive(spec, reader_conn.clone(), d, path, snapshot);
    if snapshot {
        let end = if spec.lock_tables && d == Dialect::Mysql { "UNLOCK TABLES" } else { "COMMIT" };
        let _ = reader_conn.query(db, end);
    }
    result
}

fn begin_snapshot(c: &dyn Connection, d: Dialect, spec: &BackupSpec) -> Result<()> {
    let db = Some(spec.database);
    let names: Vec<String> = spec.tables.iter().map(|t| qualify(d, None, t.schema.as_deref(), &t.name)).collect();
    match d {
        Dialect::Mysql if spec.lock_tables => {
            let list = names.iter().map(|n| format!("{n} READ")).collect::<Vec<_>>().join(", ");
            c.query(db, &format!("LOCK TABLES {list}"))?;
        }
        Dialect::Mysql => {
            c.query(db, "SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ")?;
            c.query(db, "START TRANSACTION WITH CONSISTENT SNAPSHOT")?;
        }
        Dialect::Postgres => {
            c.query(db, "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")?;
            if spec.lock_tables && !names.is_empty() {
                c.query(db, &format!("LOCK TABLE {} IN SHARE MODE", names.join(", ")))?;
            }
        }
        _ => {
            c.query(db, if spec.lock_tables { "BEGIN IMMEDIATE" } else { "BEGIN" })?;
        }
    }
    Ok(())
}

fn write_archive(spec: &BackupSpec, reader: Arc<dyn Connection>, d: Dialect, path: &Path, snapshot: bool) -> Result<Manifest> {
    let db = Some(spec.database);
    let mut schemas = Vec::new();
    for t in spec.tables {
        schemas.push(reader.table_schema(db, t.schema.as_deref(), &t.name)?);
    }
    let order = ddl::dependency_order(&schemas);
    let schemas: Vec<TableSchema> = order.into_iter().filter_map(|i| schemas.get(i).cloned()).collect();
    let views: Vec<ViewInfo> = reader.views(spec.database)?.into_iter().filter(|v| spec.views.as_ref().is_none_or(|names| names.contains(&v.name))).collect();
    let mut m = Manifest {
        format: FORMAT.into(),
        version: 1,
        created: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        comment: spec.comment.to_string(),
        connection: spec.connection.to_string(),
        database: spec.database.to_string(),
        dialect: dialect_name(d).into(),
        tables: schemas.iter().map(|t| t.name.clone()).collect(),
        views: views.iter().map(|v| v.name.clone()).collect(),
        ..Manifest::default()
    };
    let part = path.with_extension(format!("{EXTENSION}.part"));
    let file = std::fs::File::create(&part)?;
    let mut tar = tar::Builder::new(std::io::BufWriter::new(file));
    let outcome = (|| -> Result<()> {
        tar_append(&mut tar, "manifest.json", &padded_manifest(&m)?)?;
        for (i, t) in schemas.iter().enumerate() {
            tar_append(&mut tar, &format!("schema/{i:04}.json"), &serde_json::to_vec_pretty(t).map_err(|e| Error::Io(e.to_string()))?)?;
        }
        tar_append(&mut tar, "views.json", &serde_json::to_vec_pretty(&views).map_err(|e| Error::Io(e.to_string()))?)?;
        // Workers read and compress; this thread appends to the archive in arrival order.
        let workers = if snapshot || d == Dialect::Sqlite { 1 } else { 4 }.min(schemas.len().max(1));
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let rows_total = AtomicU64::new(0);
        let failure: Mutex<Option<Error>> = Mutex::new(None);
        let (tx, rx) = sync_channel::<(usize, usize, Vec<u8>)>(8);
        let cancel = dbcraft_drivers::current_cancel();
        let mut chunks = 0u64;
        let write_result = std::thread::scope(|scope| {
            for _ in 0..workers {
                let tx = tx.clone();
                let (reader, schemas, next, stop, rows_total, failure, cancel, progress) =
                    (&reader, &schemas, &next, &stop, &rows_total, &failure, &cancel, &spec.progress);
                scope.spawn(move || {
                    let work = || {
                        while !stop.load(Ordering::SeqCst) {
                            let i = next.fetch_add(1, Ordering::SeqCst);
                            let Some(t) = schemas.get(i) else { return };
                            report(progress, &format!("[{}/{}] {}", i + 1, schemas.len(), t.name));
                            let name = target_name(d, t);
                            let mut rr = RowReader::new(reader.clone(), db, t, 2000);
                            let (mut buf, mut size, mut n, mut rows) = (Vec::new(), 0usize, 0usize, 0u64);
                            let r = (|| -> Result<()> {
                                while let Some(batch) = rr.next_batch()? {
                                    rows += batch.len() as u64;
                                    for st in insert_statements(d, &name, t, &batch) {
                                        size += st.len();
                                        buf.push(st);
                                    }
                                    if size >= CHUNK {
                                        tx.send((i, n, chunk_bytes(&buf)?)).map_err(|_| Error::Cancelled)?;
                                        n += 1;
                                        buf.clear();
                                        size = 0;
                                        report(progress, &format!("  {}: {rows} rows", t.name));
                                    }
                                }
                                if !buf.is_empty() {
                                    tx.send((i, n, chunk_bytes(&buf)?)).map_err(|_| Error::Cancelled)?;
                                }
                                Ok(())
                            })();
                            rows_total.fetch_add(rows, Ordering::SeqCst);
                            if let Err(e) = r {
                                stop.store(true, Ordering::SeqCst);
                                let mut f = failure.lock().unwrap_or_else(PoisonError::into_inner);
                                if f.is_none() {
                                    *f = Some(Error::Database(format!("{}: {e}", t.name)));
                                }
                                return;
                            }
                            report(progress, &format!("  {}: {rows} rows", t.name));
                        }
                    };
                    match cancel {
                        Some(c) => dbcraft_drivers::with_cancel(c, work),
                        None => work(),
                    }
                });
            }
            drop(tx);
            for (table, n, bytes) in rx {
                if let Err(e) = tar_append(&mut tar, &format!("data/{table:04}/{n:05}.sql.gz"), &bytes) {
                    stop.store(true, Ordering::SeqCst);
                    return Err(e);
                }
                chunks += 1;
            }
            Ok(())
        });
        write_result?;
        if let Some(e) = failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(e);
        }
        m.rows = rows_total.load(Ordering::SeqCst);
        m.chunks = chunks;
        Ok(())
    })();
    let finish = outcome.and_then(|()| {
        let w = tar.into_inner()?;
        let mut file = w.into_inner().map_err(|e| Error::Io(e.to_string()))?;
        // Complete the manifest in place (its data starts after the 512-byte tar header).
        m.complete = true;
        file.seek(SeekFrom::Start(512))?;
        file.write_all(&padded_manifest(&m)?)?;
        file.sync_all()?;
        Ok(())
    });
    match finish {
        Ok(()) => {
            std::fs::rename(&part, path)?;
            Ok(m)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// The manifest, read from the archive's first entry.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let file = std::fs::File::open(path)?;
    let mut ar = tar::Archive::new(file);
    let mut entries = ar.entries()?;
    let mut first = entries.next().ok_or_else(|| Error::Io("empty backup file".into()))??;
    if first.path()?.to_string_lossy() != "manifest.json" {
        return Err(Error::Io("not a DbCraft backup (no manifest)".into()));
    }
    let mut buf = Vec::new();
    first.by_ref().take(MANIFEST_SIZE as u64 + 1).read_to_end(&mut buf)?;
    let m: Manifest = serde_json::from_slice(buf.trim_ascii_end()).map_err(|e| Error::Io(format!("damaged backup manifest: {e}")))?;
    if m.format != FORMAT {
        return Err(Error::Io("not a DbCraft backup".into()));
    }
    Ok(m)
}

/// How to restore.
pub struct RestoreSpec<'a> {
    pub conn: Arc<dyn Connection>,
    pub database: &'a str,
    /// Only these tables (`None`: all in the backup).
    pub tables: Option<Vec<String>>,
    pub continue_on_error: bool,
    pub progress: Option<ProgressSink>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreSummary {
    pub tables: usize,
    pub chunks: u64,
    pub statements: u64,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Restore `path` into `spec.database`: replaces the backed-up tables (and views).
pub fn restore(spec: &RestoreSpec, path: &Path) -> Result<RestoreSummary> {
    let m = read_manifest(path)?;
    if !m.complete {
        return Err(Error::InvalidParam("this backup was interrupted while it was written and can't be restored".into()));
    }
    let d = spec.conn.dialect();
    if m.dialect != dialect_name(d) {
        return Err(Error::InvalidParam(format!("this backup was made on {} and can't be restored on {}", m.dialect, dialect_name(d))));
    }
    let db = Some(spec.database);
    let wanted = |name: &str| spec.tables.as_ref().is_none_or(|t| t.iter().any(|x| x == name));
    let mut sum = RestoreSummary::default();
    let errors = Mutex::new(Vec::new());
    let fail = |what: String, e: Error| -> Result<()> {
        if matches!(e, Error::Cancelled) {
            return Err(e);
        }
        report(&spec.progress, &format!("  error: {what}: {e}"));
        if spec.continue_on_error {
            errors.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{what}: {e}"));
            Ok(())
        } else {
            Err(Error::Database(format!("{what}: {e}")))
        }
    };
    let exec = |sql: &str| spec.conn.execute_atomic(db, &[(sql.to_string(), None)]);
    let file = std::fs::File::open(path)?;
    let mut ar = tar::Archive::new(file);
    let mut schemas: Vec<(usize, TableSchema)> = Vec::new();
    let mut views: Vec<ViewInfo> = Vec::new();
    let mut prepared = false;
    let workers = if d == Dialect::Sqlite { 1 } else { 4 };
    let (tx, rx) = sync_channel::<(String, Vec<u8>)>(workers * 2);
    let rx = Mutex::new(rx);
    let stop = AtomicBool::new(false);
    let statements = AtomicU64::new(0);
    let chunks = AtomicU64::new(0);
    let cancel = dbcraft_drivers::current_cancel();
    let outcome: Result<()> = std::thread::scope(|scope| {
        // Chunk loaders: one transaction per chunk.
        for _ in 0..workers {
            let (rx, stop, statements, chunks, errors, cancel, conn) = (&rx, &stop, &statements, &chunks, &errors, &cancel, &spec.conn);
            let continue_on_error = spec.continue_on_error;
            scope.spawn(move || {
                let work = || {
                    loop {
                        let job = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
                        let Ok((table, bytes)) = job else { return };
                        if stop.load(Ordering::SeqCst) {
                            continue;
                        }
                        let r = chunk_statements(&bytes).and_then(|stmts| {
                            let n = stmts.len() as u64;
                            let pairs: Vec<(String, Option<u64>)> = stmts.into_iter().map(|s| (s, None)).collect();
                            conn.execute_atomic(db, &pairs).map(|_| n)
                        });
                        match r {
                            Ok(n) => {
                                statements.fetch_add(n, Ordering::SeqCst);
                                chunks.fetch_add(1, Ordering::SeqCst);
                            }
                            Err(e) => {
                                errors.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{table}: {e}"));
                                if !continue_on_error || matches!(e, Error::Cancelled) {
                                    stop.store(true, Ordering::SeqCst);
                                }
                            }
                        }
                    }
                };
                match cancel {
                    Some(c) => dbcraft_drivers::with_cancel(c, work),
                    None => work(),
                }
            });
        }
        let feed = (|| -> Result<()> {
            for entry in ar.entries()? {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let mut entry = entry?;
                let name = entry.path()?.to_string_lossy().to_string();
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                if let Some(rest) = name.strip_prefix("schema/") {
                    let i: usize = rest.trim_end_matches(".json").parse().map_err(|_| Error::Io(format!("damaged backup entry {name}")))?;
                    let t: TableSchema = serde_json::from_slice(&buf).map_err(|e| Error::Io(format!("{name}: {e}")))?;
                    schemas.push((i, t));
                } else if name == "views.json" {
                    views = serde_json::from_slice(&buf).map_err(|e| Error::Io(format!("views.json: {e}")))?;
                } else if let Some(rest) = name.strip_prefix("data/") {
                    if !prepared {
                        prepare(spec, d, &schemas, &views, &wanted, &exec, &fail, &mut sum)?;
                        prepared = true;
                    }
                    let i: usize = rest.split('/').next().and_then(|x| x.parse().ok()).ok_or_else(|| Error::Io(format!("damaged backup entry {name}")))?;
                    if let Some((_, t)) = schemas.iter().find(|(j, _)| *j == i)
                        && wanted(&t.name)
                    {
                        tx.send((t.name.clone(), buf)).map_err(|_| Error::Cancelled)?;
                    }
                }
            }
            Ok(())
        })();
        drop(tx);
        feed
    });
    outcome?;
    let mut errs = errors.into_inner().unwrap_or_else(PoisonError::into_inner);
    if stop.load(Ordering::SeqCst) && !spec.continue_on_error {
        return Err(Error::Database(errs.first().cloned().unwrap_or_else(|| "restore stopped".into())));
    }
    if !prepared {
        let errors_before = Mutex::new(Vec::new());
        let fail2 = |what: String, e: Error| -> Result<()> {
            if spec.continue_on_error {
                errors_before.lock().unwrap_or_else(PoisonError::into_inner).push(format!("{what}: {e}"));
                Ok(())
            } else {
                Err(Error::Database(format!("{what}: {e}")))
            }
        };
        prepare(spec, d, &schemas, &views, &wanted, &exec, &fail2, &mut sum)?;
        errs.extend(errors_before.into_inner().unwrap_or_else(PoisonError::into_inner));
    }
    // Keys, sequences, triggers and views after the data.
    let mut ordered: Vec<&TableSchema> = schemas.iter().map(|(_, t)| t).filter(|t| wanted(&t.name)).collect();
    ordered.sort_by_key(|t| m.tables.iter().position(|n| n == &t.name));
    let mut late = Vec::new();
    for t in &ordered {
        let name = target_name(d, t);
        for i in &t.indexes {
            late.push((
                t.name.clone(),
                match &i.native_ddl {
                    Some(native) if i.columns.is_empty() => native.clone(),
                    _ => ddl::index_sql(i, &name, d, d),
                },
            ));
        }
        for s in ddl::reset_sequences(t, &name, d) {
            late.push((t.name.clone(), s));
        }
    }
    for t in &ordered {
        let name = target_name(d, t);
        for s in ddl::add_foreign_keys(t, &name, d, &|f| ref_name(d, f, t.schema.as_deref())) {
            late.push((t.name.clone(), s));
        }
        for s in ddl::index_comments(t, t.schema.as_deref(), d).into_iter().chain(ddl::create_triggers(t, &name, d, d, &mut sum.warnings)) {
            late.push((t.name.clone(), s));
        }
    }
    for (what, sql) in late {
        if let Err(e) = exec(&sql) {
            if matches!(e, Error::Cancelled) {
                return Err(e);
            }
            report(&spec.progress, &format!("  error: {what}: {e}"));
            if !spec.continue_on_error {
                return Err(Error::Database(format!("{what}: {e}")));
            }
            errs.push(format!("{what}: {e}"));
        }
    }
    for v in &views {
        let name = qualify(d, None, if d == Dialect::Postgres { v.schema.as_deref() } else { None }, &v.name);
        // A view may belong to another user or use engine features: report, don't fail.
        if let Err(e) = exec(&ddl::create_view(v, &name)) {
            sum.warnings.push(format!("view `{}` was not created: {e}", v.name));
        }
    }
    sum.tables = ordered.len();
    sum.chunks = chunks.load(Ordering::SeqCst);
    sum.statements = statements.load(Ordering::SeqCst);
    sum.errors = errs;
    report(
        &spec.progress,
        &format!("Restore finished: {} tables, {} chunks, {} errors, {} warnings", sum.tables, sum.chunks, sum.errors.len(), sum.warnings.len()),
    );
    Ok(sum)
}

/// Drop what the backup replaces (views first, children before parents), then create the tables.
#[allow(clippy::too_many_arguments)]
fn prepare(
    spec: &RestoreSpec,
    d: Dialect,
    schemas: &[(usize, TableSchema)],
    views: &[ViewInfo],
    wanted: &dyn Fn(&str) -> bool,
    exec: &dyn Fn(&str) -> Result<Vec<u64>>,
    fail: &dyn Fn(String, Error) -> Result<()>,
    sum: &mut RestoreSummary,
) -> Result<()> {
    let mut sorted: Vec<&(usize, TableSchema)> = schemas.iter().filter(|(_, t)| wanted(&t.name)).collect();
    sorted.sort_by_key(|(i, _)| *i);
    for v in views {
        let name = qualify(d, None, if d == Dialect::Postgres { v.schema.as_deref() } else { None }, &v.name);
        if let Err(e) = exec(&format!("DROP VIEW IF EXISTS {name}")) {
            sum.warnings.push(format!("view `{}` could not be replaced: {e}", v.name));
        }
    }
    for (_, t) in sorted.iter().rev() {
        let cascade = if d == Dialect::Postgres { " CASCADE" } else { "" };
        if let Err(e) = exec(&format!("DROP TABLE IF EXISTS {}{cascade}", target_name(d, t))) {
            fail(t.name.clone(), e)?;
        }
    }
    for (_, t) in &sorted {
        report(&spec.progress, &format!("Create {}", t.name));
        let name = target_name(d, t);
        let sql = ddl::create_table(t, &name, d, d, &|f| ref_name(d, f, t.schema.as_deref()), &mut sum.warnings);
        if let Err(e) = exec(&sql) {
            fail(t.name.clone(), e)?;
            continue;
        }
        for c in ddl::comments(t, &name, d) {
            if let Err(e) = exec(&c) {
                fail(t.name.clone(), e)?;
            }
        }
    }
    Ok(())
}

/// Navicat's "Extract SQL": the backup as one SQL script.
pub fn extract_sql(path: &Path, out: &mut dyn Write) -> Result<Manifest> {
    let m = read_manifest(path)?;
    let d = match m.dialect.as_str() {
        "mysql" => Dialect::Mysql,
        "postgres" => Dialect::Postgres,
        _ => Dialect::Sqlite,
    };
    writeln!(out, "-- DbCraft backup of `{}` ({}), {}\n-- {}", m.database, m.dialect, m.created, m.comment.replace('\n', " "))?;
    let file = std::fs::File::open(path)?;
    let mut ar = tar::Archive::new(file);
    let mut schemas: Vec<TableSchema> = Vec::new();
    let mut views: Vec<ViewInfo> = Vec::new();
    let mut warnings = Vec::new();
    let mut created = false;
    for entry in ar.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        if name.starts_with("schema/") {
            schemas.push(serde_json::from_slice(&buf).map_err(|e| Error::Io(format!("{name}: {e}")))?);
        } else if name == "views.json" {
            views = serde_json::from_slice(&buf).map_err(|e| Error::Io(e.to_string()))?;
        } else if name.starts_with("data/") {
            if !created {
                for t in &schemas {
                    writeln!(out, "\n{};", ddl::create_table(t, &target_name(d, t), d, d, &|f| ref_name(d, f, t.schema.as_deref()), &mut warnings))?;
                }
                created = true;
            }
            for st in chunk_statements(&buf)? {
                writeln!(out, "{st};")?;
            }
        }
    }
    if !created {
        for t in &schemas {
            writeln!(out, "\n{};", ddl::create_table(t, &target_name(d, t), d, d, &|f| ref_name(d, f, t.schema.as_deref()), &mut warnings))?;
        }
    }
    for t in &schemas {
        let name = target_name(d, t);
        for i in &t.indexes {
            if !i.columns.is_empty() {
                writeln!(out, "{};", ddl::index_sql(i, &name, d, d))?;
            }
        }
        for s in ddl::reset_sequences(t, &name, d).into_iter().chain(ddl::add_foreign_keys(t, &name, d, &|f| ref_name(d, f, t.schema.as_deref()))) {
            writeln!(out, "{s};")?;
        }
    }
    for v in &views {
        writeln!(out, "{};", ddl::create_view(v, &quote_ident(d, &v.name)))?;
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_round_trip_without_parsing_sql() {
        let stmts = vec!["INSERT INTO t VALUES ('a;\nb')".to_string(), String::new(), "x".repeat(100_000)];
        let bytes = chunk_bytes(&stmts).unwrap();
        assert!(bytes.len() < 2_000, "compressed: {}", bytes.len());
        assert_eq!(chunk_statements(&bytes).unwrap(), stmts);
        assert!(chunk_statements(b"not gzip").is_err());
    }

    #[test]
    fn manifest_always_fits() {
        let m = Manifest { comment: "c".repeat(200_000), warnings: vec!["w".repeat(1000); 100], ..Manifest::default() };
        assert_eq!(padded_manifest(&m).unwrap().len(), MANIFEST_SIZE);
    }
}
