//! Copy tables between connections or into a SQL file: the engine behind Data Transfer, Dump SQL
//! File and Backup.
//!
//! - **Complete:** tables are recreated from the source's [`TableSchema`] (types, defaults,
//!   auto-increment, primary keys, indexes, foreign keys, comments) and optionally its views. On
//!   the same kind of engine native types are kept; across engines they go through
//!   [`GenericType`](dbcraft_core::GenericType) ([`crate::ddl`]).
//! - **Fast:** rows are read with keyset paging (no growing `OFFSET`), written as multi-row
//!   `INSERT`s capped in size, one transaction per batch, with reading and writing overlapped and
//!   several tables copied at once. Indexes and foreign keys are created after the data.

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex, PoisonError};

use dbcraft_core::{Error, ForeignKeyInfo, GenericType as G, Result, TableSchema, Value};
use dbcraft_drivers::Connection;
use dbcraft_sql::{Dialect, bytes_literal, quote_ident, quote_literal};
use serde::{Deserialize, Serialize};

use crate::{ProgressSink, Session, ddl};

pub use dbcraft_drivers::types::render;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRef {
    #[serde(default)]
    pub schema: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub create_tables: bool,
    pub drop_existing: bool,
    pub create_records: bool,
    pub continue_on_error: bool,
    /// Rows read per round trip (and written per transaction).
    pub batch_size: u64,
    pub indexes: bool,
    pub foreign_keys: bool,
    /// Also copy the source database's views (best effort across engines).
    pub views: bool,
    /// Tables copied at the same time (1–8).
    pub parallel: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            create_tables: true,
            drop_existing: false,
            create_records: true,
            continue_on_error: false,
            batch_size: 1000,
            indexes: true,
            foreign_keys: true,
            views: false,
            parallel: 4,
        }
    }
}

pub enum Sink<'a> {
    /// Another (or the same) saved connection.
    Conn { id: String, database: Option<String>, schema: Option<String> },
    /// A SQL script in `dialect`. `keep_schema`: qualify tables with their schema.
    File { out: &'a mut dyn Write, dialect: Dialect, keep_schema: bool },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub tables: usize,
    pub rows: u64,
    pub errors: Vec<String>,
    /// Things carried over only partly (an engine-specific default, an expression index…).
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Largest single `INSERT` written (MySQL's smallest common `max_allowed_packet` is 4 MiB).
const MAX_STATEMENT: usize = 1 << 20;

/// A value as a literal for a column of type `g` on `dialect`.
pub fn literal_for(d: Dialect, g: G, v: &Value) -> String {
    let numeric = matches!(g, G::SmallInt | G::Integer | G::BigInt | G::Decimal { .. } | G::Float | G::Double);
    let boolean = |b: bool| match d {
        Dialect::Postgres => if b { "TRUE" } else { "FALSE" }.to_string(),
        _ => if b { "1" } else { "0" }.to_string(),
    };
    match (g, v) {
        (_, Value::Null) => "NULL".into(),
        (G::Bool, Value::Bool(b)) => boolean(*b),
        (G::Bool, Value::Int(i)) => boolean(*i != 0),
        (G::Bool, Value::UInt(u)) => boolean(*u != 0),
        (G::Bool, Value::Text(t)) => boolean(matches!(t.trim().to_ascii_lowercase().as_str(), "1" | "t" | "true" | "y" | "yes" | "on")),
        (_, Value::Bool(b)) if numeric => if *b { "1" } else { "0" }.into(),
        (_, Value::Bool(b)) => quote_literal(d, if *b { "true" } else { "false" }),
        (G::Blob, Value::Text(t)) => bytes_literal(d, t.as_bytes()),
        (G::Blob, Value::Bytes(b)) => bytes_literal(d, b),
        (_, Value::Bytes(b)) if d == Dialect::Postgres => quote_literal(d, &String::from_utf8_lossy(b)),
        (_, Value::Bytes(b)) => bytes_literal(d, b),
        (_, Value::Float(f)) if !f.is_finite() => "NULL".into(),
        (_, Value::Float(f)) if numeric => f.to_string(),
        (_, Value::Int(i)) if numeric => i.to_string(),
        (_, Value::UInt(u)) if numeric => u.to_string(),
        (G::DateTime, Value::Text(t)) if d == Dialect::Mysql => quote_literal(d, &mysql_datetime(t)),
        (_, other) => quote_literal(d, &other.display()),
    }
}

/// MySQL/MariaDB reject `T` separators with time-zone suffixes in strict mode: keep the wall time.
fn mysql_datetime(t: &str) -> String {
    let t = t.trim().replacen('T', " ", 1);
    let cut = t.get(10..).and_then(|rest| rest.find(['+', 'Z']).or_else(|| rest.rfind('-').filter(|i| *i > 5))).map(|i| i + 10);
    match cut {
        Some(i) => t.get(..i).unwrap_or(&t).to_string(),
        None => t,
    }
}

/// A table name for `d`. SQLite DDL can't qualify foreign-key targets or index tables, and
/// DbCraft opens one SQLite file per connection, so SQLite names stay bare.
pub fn qualify(d: Dialect, db: Option<&str>, schema: Option<&str>, table: &str) -> String {
    let t = quote_ident(d, table);
    match d {
        Dialect::Postgres | Dialect::Mssql => match schema {
            Some(s) => format!("{}.{t}", quote_ident(d, s)),
            None => t,
        },
        Dialect::Mysql => match db {
            Some(db) => format!("{}.{t}", quote_ident(d, db)),
            None => t,
        },
        Dialect::Sqlite => t,
    }
}

/// Multi-row `INSERT`s for `rows`, each at most [`MAX_STATEMENT`] bytes (a single huge row is
/// still written, alone).
pub fn insert_statements(d: Dialect, target: &str, schema: &TableSchema, rows: &[Vec<Value>]) -> Vec<String> {
    let cols = schema.columns.iter().map(|c| quote_ident(d, &c.name)).collect::<Vec<_>>().join(", ");
    let head = format!("INSERT INTO {target} ({cols}) VALUES\n");
    let mut out = Vec::new();
    let mut cur = String::new();
    for row in rows {
        let tuple = format!(
            "({})",
            schema.columns.iter().enumerate().map(|(i, c)| literal_for(d, c.generic, row.get(i).unwrap_or(&Value::Null))).collect::<Vec<_>>().join(", ")
        );
        if !cur.is_empty() && cur.len() + tuple.len() + 2 > MAX_STATEMENT {
            out.push(std::mem::take(&mut cur));
        }
        if cur.is_empty() {
            cur.push_str(&head);
        } else {
            cur.push_str(",\n");
        }
        cur.push_str(&tuple);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Reads one table in batches: keyset paging on the primary key (`WHERE pk > last ORDER BY pk`),
/// `LIMIT/OFFSET` for tables without one.
pub struct RowReader {
    conn: Arc<dyn Connection>,
    db: Option<String>,
    select: String,
    order: String,
    /// Positions of the key columns in the row, and their quoted names.
    key: Vec<(usize, String, G)>,
    src: Dialect,
    batch: u64,
    last: Option<Vec<Value>>,
    offset: u64,
    done: bool,
}

impl RowReader {
    pub fn new(conn: Arc<dyn Connection>, db: Option<&str>, t: &TableSchema, batch: u64) -> RowReader {
        let src = conn.dialect();
        let from = match src {
            Dialect::Sqlite => match db {
                Some(d) => format!("{}.{}", quote_ident(src, d), quote_ident(src, &t.name)),
                None => quote_ident(src, &t.name),
            },
            _ => qualify(src, db, t.schema.as_deref(), &t.name),
        };
        let cols = t.columns.iter().map(|c| quote_ident(src, &c.name)).collect::<Vec<_>>().join(", ");
        let key: Vec<(usize, String, G)> =
            t.primary_key().iter().filter_map(|k| t.columns.iter().position(|c| c.name == k.name).map(|i| (i, quote_ident(src, &k.name), k.generic))).collect();
        let order = if key.is_empty() { String::new() } else { format!(" ORDER BY {}", key.iter().map(|k| k.1.clone()).collect::<Vec<_>>().join(", ")) };
        RowReader {
            conn,
            db: db.map(str::to_string),
            select: format!("SELECT {cols} FROM {from}"),
            order,
            key,
            src,
            batch: batch.clamp(1, 50_000),
            last: None,
            offset: 0,
            done: false,
        }
    }

    /// The next batch, or `None` when the table is exhausted.
    pub fn next_batch(&mut self) -> Result<Option<Vec<Vec<Value>>>> {
        if self.done {
            return Ok(None);
        }
        let sql = match (&self.last, self.key.is_empty()) {
            (_, true) => format!("{} LIMIT {} OFFSET {}", self.select, self.batch, self.offset),
            (None, false) => format!("{}{} LIMIT {}", self.select, self.order, self.batch),
            (Some(last), false) => {
                let lits: Vec<String> = self.key.iter().map(|(i, _, g)| literal_for(self.src, *g, last.get(*i).unwrap_or(&Value::Null))).collect();
                let names: Vec<String> = self.key.iter().map(|k| k.1.clone()).collect();
                let cond = if names.len() == 1 {
                    format!("{} > {}", names.join(""), lits.join(""))
                } else {
                    format!("({}) > ({})", names.join(", "), lits.join(", "))
                };
                format!("{} WHERE {cond}{} LIMIT {}", self.select, self.order, self.batch)
            }
        };
        let rs = self.conn.query(self.db.as_deref(), &sql)?;
        let n = rs.rows.len() as u64;
        if n < self.batch {
            self.done = true;
        }
        if n == 0 {
            return Ok(None);
        }
        self.offset += n;
        self.last = rs.rows.last().cloned();
        Ok(Some(rs.rows))
    }
}

/// Copy one table's rows into a connection: a reader thread fetches the next batch while this
/// thread writes the current one (one transaction per batch).
#[allow(clippy::too_many_arguments)]
fn copy_rows(
    src: Arc<dyn Connection>,
    src_db: Option<&str>,
    t: &TableSchema,
    dst: &dyn Connection,
    dst_db: Option<&str>,
    target: &str,
    batch: u64,
    report: &dyn Fn(&str),
) -> Result<u64> {
    let d = dst.dialect();
    let mut reader = RowReader::new(src, src_db, t, batch);
    let (tx, rx) = sync_channel::<Result<Vec<Vec<Value>>>>(2);
    let cancel = dbcraft_drivers::current_cancel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut pump = || {
                loop {
                    match reader.next_batch() {
                        Ok(Some(rows)) => {
                            if tx.send(Ok(rows)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => return,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return;
                        }
                    }
                }
            };
            match &cancel {
                Some(c) => dbcraft_drivers::with_cancel(c, pump),
                None => pump(),
            }
        });
        let mut copied = 0u64;
        for rows in rx {
            let rows = rows?;
            let stmts: Vec<(String, Option<u64>)> = insert_statements(d, target, t, &rows).into_iter().map(|s| (s, None)).collect();
            dst.execute_atomic(dst_db, &stmts)?;
            copied += rows.len() as u64;
            report(&format!("  {}: {copied} rows", t.name));
        }
        Ok(copied)
    })
}

/// Shared state of a run (workers report through it).
struct Run {
    progress: Option<ProgressSink>,
    log: Mutex<Vec<String>>,
}

impl Run {
    fn report(&self, msg: &str) {
        if let Some(p) = &self.progress {
            p(msg);
        }
        self.log.lock().unwrap_or_else(PoisonError::into_inner).push(msg.to_string());
    }
}

/// Copy `tables` (and with `opts.views` the views) from `src` (database `src_db`) into `sink`.
pub fn run(s: &mut Session, src: &str, src_db: Option<&str>, tables: &[TableRef], sink: &mut Sink, opts: &Options) -> Result<Summary> {
    if tables.is_empty() {
        return Err(Error::InvalidParam("no tables selected".into()));
    }
    let src_conn = s.conn_auto(src)?;
    let src_dialect = src_conn.dialect();
    let mut sum = Summary::default();
    // 1. Structure of every table, then parents before children.
    let mut schemas = Vec::with_capacity(tables.len());
    for t in tables {
        match src_conn.table_schema(src_db, t.schema.as_deref(), &t.name) {
            Ok(sch) => schemas.push(sch),
            Err(e) if opts.continue_on_error => {
                s.report(&format!("{}: {e}", t.name));
                sum.errors.push(format!("{}: {e}", t.name));
            }
            Err(e) => return Err(Error::Database(format!("{}: {e}", t.name))),
        }
    }
    let order = ddl::dependency_order(&schemas);
    let schemas: Vec<TableSchema> = order.into_iter().filter_map(|i| schemas.get(i).cloned()).collect();
    let views = if opts.views {
        let db = match src_db {
            Some(d) => d.to_string(),
            None => src_conn.list_databases()?.into_iter().next().unwrap_or_default(),
        };
        src_conn.views(&db)?
    } else {
        Vec::new()
    };
    let run = Run { progress: s.progress.clone(), log: Mutex::new(Vec::new()) };
    let result = match sink {
        Sink::Conn { id, database, schema } => {
            let dst = s.conn_auto(id)?;
            to_conn(&run, src_conn, src_db, &schemas, &views, dst, database.as_deref(), schema.as_deref(), opts, &mut sum)
        }
        Sink::File { out, dialect, keep_schema } => {
            to_file(&run, src_conn, src_db, &schemas, &views, &mut **out, *dialect, *keep_schema, src_dialect, opts, &mut sum)
        }
    };
    s.messages.extend(run.log.into_inner().unwrap_or_else(PoisonError::into_inner));
    result?;
    s.report(&format!("Finished: {} tables, {} rows, {} errors, {} warnings", sum.tables, sum.rows, sum.errors.len(), sum.warnings.len()));
    for w in &sum.warnings {
        s.report(&format!("  warning: {w}"));
    }
    Ok(sum)
}

/// Run one DDL statement; with `continue_on_error`, record a failure instead of stopping.
fn exec_ddl(run: &Run, dst: &dyn Connection, db: Option<&str>, sql: &str, what: &str, opts: &Options, sum: &mut Summary) -> Result<()> {
    match dst.execute_atomic(db, &[(sql.to_string(), None)]) {
        Ok(_) => Ok(()),
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Err(e) if opts.continue_on_error => {
            run.report(&format!("  error ({what}): {e}"));
            sum.errors.push(format!("{what}: {e}"));
            Ok(())
        }
        Err(e) => Err(Error::Database(format!("{what}: {e}"))),
    }
}

#[allow(clippy::too_many_arguments)]
fn to_conn(
    run: &Run,
    src: Arc<dyn Connection>,
    src_db: Option<&str>,
    schemas: &[TableSchema],
    views: &[dbcraft_core::ViewInfo],
    dst: Arc<dyn Connection>,
    db: Option<&str>,
    schema: Option<&str>,
    opts: &Options,
    sum: &mut Summary,
) -> Result<()> {
    let (sd, d) = (src.dialect(), dst.dialect());
    let name = |t: &TableSchema| qualify(d, db, schema, &t.name);
    let ref_name = |f: &ForeignKeyInfo| qualify(d, db, f.ref_schema.as_deref().or(schema), &f.ref_table);
    // 2. Drop children before parents, then create parents before children.
    if opts.drop_existing {
        for v in views {
            exec_ddl(run, dst.as_ref(), db, &format!("DROP VIEW IF EXISTS {}", qualify(d, db, schema, &v.name)), &v.name, opts, sum)?;
        }
        for t in schemas.iter().rev() {
            let cascade = if d == Dialect::Postgres { " CASCADE" } else { "" };
            exec_ddl(run, dst.as_ref(), db, &format!("DROP TABLE IF EXISTS {}{cascade}", name(t)), &t.name, opts, sum)?;
        }
    }
    let mut created = Vec::new();
    for t in schemas {
        if opts.create_tables {
            let sql = ddl::create_table(t, &name(t), sd, d, &ref_name, &mut sum.warnings);
            let before = sum.errors.len();
            exec_ddl(run, dst.as_ref(), db, &sql, &t.name, opts, sum)?;
            if sum.errors.len() > before {
                continue;
            }
            for c in ddl::comments(t, &name(t), d) {
                exec_ddl(run, dst.as_ref(), db, &c, &t.name, opts, sum)?;
            }
        }
        created.push(t.clone());
    }
    // 3. Rows: several tables at once (SQLite checks foreign keys on insert: one by one, in order).
    if opts.create_records {
        let workers = if d == Dialect::Sqlite { 1 } else { opts.parallel.clamp(1, 8) as usize }.min(created.len().max(1));
        let next = AtomicUsize::new(0);
        let results: Mutex<Vec<(usize, Result<u64>)>> = Mutex::new(Vec::new());
        let cancel = dbcraft_drivers::current_cancel();
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    let work = || {
                        loop {
                            let i = next.fetch_add(1, Ordering::SeqCst);
                            let Some(t) = created.get(i) else { return };
                            run.report(&format!("[{}/{}] {}", i + 1, created.len(), t.name));
                            let r = copy_rows(src.clone(), src_db, t, dst.as_ref(), db, &name(t), opts.batch_size, &|m| run.report(m));
                            let failed = r.is_err();
                            results.lock().unwrap_or_else(PoisonError::into_inner).push((i, r));
                            if failed && !opts.continue_on_error {
                                // Stop handing out tables; the others finish their current one.
                                next.store(usize::MAX / 2, Ordering::SeqCst);
                            }
                        }
                    };
                    match &cancel {
                        Some(c) => dbcraft_drivers::with_cancel(c, work),
                        None => work(),
                    }
                });
            }
        });
        let mut results = results.into_inner().unwrap_or_else(PoisonError::into_inner);
        results.sort_by_key(|(i, _)| *i);
        for (i, r) in results {
            let tname = created.get(i).map(|t| t.name.clone()).unwrap_or_default();
            match r {
                Ok(n) => {
                    sum.tables += 1;
                    sum.rows += n;
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) if opts.continue_on_error => {
                    run.report(&format!("  error: {tname}: {e}"));
                    sum.errors.push(format!("{tname}: {e}"));
                }
                Err(e) => return Err(Error::Database(format!("{tname}: {e}"))),
            }
        }
    } else {
        sum.tables = created.len();
    }
    // 4. Indexes, foreign keys, sequences, views: after the data.
    for t in &created {
        if opts.indexes {
            for sql in ddl::create_indexes(t, &name(t), sd, d, &mut sum.warnings) {
                exec_ddl(run, dst.as_ref(), db, &sql, &format!("{} index", t.name), opts, sum)?;
            }
        }
        if opts.create_records {
            for sql in ddl::reset_sequences(t, &name(t), d) {
                exec_ddl(run, dst.as_ref(), db, &sql, &format!("{} sequence", t.name), opts, sum)?;
            }
        }
    }
    if opts.foreign_keys {
        for t in &created {
            for sql in ddl::add_foreign_keys(t, &name(t), d, &ref_name) {
                exec_ddl(run, dst.as_ref(), db, &sql, &format!("{} foreign key", t.name), opts, sum)?;
            }
        }
    }
    for v in views {
        let sql = ddl::create_view(v, &qualify(d, db, schema, &v.name));
        // A view's SQL is the source engine's: on another engine it may not parse.
        if let Err(e) = dst.execute_atomic(db, &[(sql, None)]) {
            if matches!(e, Error::Cancelled) {
                return Err(e);
            }
            sum.warnings.push(format!("view `{}` was not created: {e}", v.name));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn to_file(
    run: &Run,
    src: Arc<dyn Connection>,
    src_db: Option<&str>,
    schemas: &[TableSchema],
    views: &[dbcraft_core::ViewInfo],
    out: &mut dyn Write,
    d: Dialect,
    keep_schema: bool,
    sd: Dialect,
    opts: &Options,
    sum: &mut Summary,
) -> Result<()> {
    let name = |t: &TableSchema| qualify(d, None, if keep_schema { t.schema.as_deref() } else { None }, &t.name);
    let ref_name = |f: &ForeignKeyInfo| qualify(d, None, if keep_schema { f.ref_schema.as_deref() } else { None }, &f.ref_table);
    if opts.drop_existing {
        for v in views {
            stmt(out, &format!("DROP VIEW IF EXISTS {}", qualify(d, None, None, &v.name)))?;
        }
        for t in schemas.iter().rev() {
            let cascade = if d == Dialect::Postgres { " CASCADE" } else { "" };
            stmt(out, &format!("DROP TABLE IF EXISTS {}{cascade}", name(t)))?;
        }
    }
    for t in schemas {
        writeln!(out, "\n-- Table {}", t.name)?;
        if opts.create_tables {
            stmt(out, &ddl::create_table(t, &name(t), sd, d, &ref_name, &mut sum.warnings))?;
            for c in ddl::comments(t, &name(t), d) {
                stmt(out, &c)?;
            }
        }
        let mut rows = 0u64;
        if opts.create_records {
            let mut reader = RowReader::new(src.clone(), src_db, t, opts.batch_size);
            while let Some(batch) = reader.next_batch()? {
                for sql in insert_statements(d, &name(t), t, &batch) {
                    stmt(out, &sql)?;
                }
                rows += batch.len() as u64;
                run.report(&format!("  {}: {rows} rows", t.name));
            }
        }
        sum.tables += 1;
        sum.rows += rows;
    }
    for t in schemas {
        if opts.indexes {
            for sql in ddl::create_indexes(t, &name(t), sd, d, &mut sum.warnings) {
                stmt(out, &sql)?;
            }
        }
        if opts.create_records {
            for sql in ddl::reset_sequences(t, &name(t), d) {
                stmt(out, &sql)?;
            }
        }
    }
    if opts.foreign_keys {
        for t in schemas {
            for sql in ddl::add_foreign_keys(t, &name(t), d, &ref_name) {
                stmt(out, &sql)?;
            }
        }
    }
    for v in views {
        stmt(out, &ddl::create_view(v, &qualify(d, None, None, &v.name)))?;
    }
    Ok(())
}

/// One statement of a SQL script.
fn stmt(out: &mut dyn Write, sql: &str) -> Result<()> {
    writeln!(out, "{sql};").map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcraft_core::ColumnInfo;

    #[test]
    fn literals_follow_the_target_type() {
        assert_eq!(literal_for(Dialect::Postgres, G::Bool, &Value::Int(1)), "TRUE");
        assert_eq!(literal_for(Dialect::Mysql, G::Bool, &Value::Bool(false)), "0");
        assert_eq!(literal_for(Dialect::Postgres, G::Bool, &Value::Text("yes".into())), "TRUE");
        assert_eq!(literal_for(Dialect::Sqlite, G::Integer, &Value::Bool(true)), "1");
        assert_eq!(literal_for(Dialect::Postgres, G::Text, &Value::Bytes(b"hi".to_vec())), "'hi'");
        assert_eq!(literal_for(Dialect::Mysql, G::Blob, &Value::Text("hi".into())), "X'6869'");
        assert_eq!(literal_for(Dialect::Mysql, G::Double, &Value::Float(f64::NAN)), "NULL");
        assert_eq!(literal_for(Dialect::Mysql, G::Text, &Value::Int(5)), "'5'");
        assert_eq!(literal_for(Dialect::Mysql, G::DateTime, &Value::Text("2026-10-14T08:59:12.5+00:00".into())), "'2026-10-14 08:59:12.5'");
        assert_eq!(literal_for(Dialect::Mysql, G::DateTime, &Value::Text("2026-10-14 08:59:12".into())), "'2026-10-14 08:59:12'");
        assert_eq!(literal_for(Dialect::Sqlite, G::Text, &Value::Text("it's".into())), "'it''s'");
    }

    #[test]
    fn inserts_are_split_by_size() {
        let t = TableSchema { name: "t".into(), columns: vec![ColumnInfo::new("a", "text", G::Text, true, None)], ..TableSchema::default() };
        let big = "x".repeat(400 * 1024);
        let rows: Vec<Vec<Value>> = (0..5).map(|_| vec![Value::Text(big.clone())]).collect();
        let stmts = insert_statements(Dialect::Sqlite, "\"t\"", &t, &rows);
        assert_eq!(stmts.len(), 3, "2 + 2 + 1 rows under 1 MiB each");
        assert!(stmts.iter().all(|s| s.starts_with("INSERT INTO \"t\" (\"a\") VALUES\n(")));
        assert!(insert_statements(Dialect::Sqlite, "\"t\"", &t, &[]).is_empty());
    }
}
