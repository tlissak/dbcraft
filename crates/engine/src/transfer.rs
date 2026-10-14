//! Copy tables between connections or into a SQL file: the engine behind Data Transfer, Dump SQL
//! File and Backup. Structure goes through [`GenericType`] so it works across engines; columns,
//! NOT NULL and primary keys are carried over (indexes, foreign keys, defaults and auto-increment
//! are not yet: see ROADMAP.md).

use std::io::Write;

use dbcraft_core::{ColumnInfo, Error, GenericType as G, Result, Value};
use dbcraft_sql::{Dialect, bytes_literal, quote_ident, quote_literal};
use serde::{Deserialize, Serialize};

use crate::Session;

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
    pub batch_size: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options { create_tables: true, drop_existing: false, create_records: true, continue_on_error: false, batch_size: 500 }
    }
}

pub enum Sink<'a> {
    /// Another (or the same) saved connection.
    Conn { id: String, database: Option<String>, schema: Option<String> },
    /// A SQL script in `dialect`. `keep_schema`: qualify tables with their schema (backups).
    File { out: &'a mut dyn Write, dialect: Dialect, keep_schema: bool },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub tables: usize,
    pub rows: u64,
    pub errors: Vec<String>,
}

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

/// `CREATE TABLE` for `cols` on `d`.
pub fn create_table_sql(d: Dialect, table: &str, cols: &[ColumnInfo]) -> String {
    let mut lines: Vec<String> = cols
        .iter()
        .map(|c| {
            let ty = crate::transfer::render(d, c.generic, c.pk.is_some());
            let nn = if c.nullable && c.pk.is_none() { "" } else { " NOT NULL" };
            format!("  {} {ty}{nn}", quote_ident(d, &c.name))
        })
        .collect();
    let mut pk: Vec<&ColumnInfo> = cols.iter().filter(|c| c.pk.is_some()).collect();
    pk.sort_by_key(|c| c.pk);
    if !pk.is_empty() {
        lines.push(format!("  PRIMARY KEY ({})", pk.iter().map(|c| quote_ident(d, &c.name)).collect::<Vec<_>>().join(", ")));
    }
    format!("CREATE TABLE {table} (\n{}\n)", lines.join(",\n"))
}

pub use dbcraft_drivers::types::render;

fn qualify(d: Dialect, db: Option<&str>, schema: Option<&str>, table: &str) -> String {
    let t = quote_ident(d, table);
    match d {
        Dialect::Postgres | Dialect::Mssql => match schema {
            Some(s) => format!("{}.{t}", quote_ident(d, s)),
            None => t,
        },
        Dialect::Mysql | Dialect::Sqlite => match db {
            Some(db) => format!("{}.{t}", quote_ident(d, db)),
            None => t,
        },
    }
}

fn emit(s: &mut Session, sink: &mut Sink, sql: &str) -> Result<()> {
    match sink {
        Sink::Conn { id, database, .. } => {
            let (id, db) = (id.clone(), database.clone());
            s.conn_auto(&id)?.execute_atomic(db.as_deref(), &[(sql.to_string(), None)])?;
            Ok(())
        }
        Sink::File { out, .. } => writeln!(out, "{sql};").map_err(Error::from),
    }
}

fn copy_table(s: &mut Session, src: &str, src_db: Option<&str>, t: &TableRef, sink: &mut Sink, opts: &Options) -> Result<u64> {
    let src_dialect = s.conn_auto(src)?.dialect();
    let cols = s.conn_auto(src)?.columns(src_db, t.schema.as_deref(), &t.name)?;
    let (d, target) = match sink {
        Sink::Conn { id, database, schema } => {
            let (id, db, sch) = (id.clone(), database.clone(), schema.clone());
            let d = s.conn_auto(&id)?.dialect();
            (d, qualify(d, db.as_deref(), sch.as_deref(), &t.name))
        }
        Sink::File { dialect, keep_schema, .. } => {
            let d = *dialect;
            let schema = if *keep_schema { t.schema.as_deref() } else { None };
            (d, qualify(d, None, schema, &t.name))
        }
    };
    if let Sink::File { out, .. } = sink {
        writeln!(out, "\n-- Table {}", t.name)?;
    }
    if opts.drop_existing {
        emit(s, sink, &format!("DROP TABLE IF EXISTS {target}"))?;
    }
    if opts.create_tables {
        emit(s, sink, &create_table_sql(d, &target, &cols))?;
    }
    if !opts.create_records {
        return Ok(0);
    }
    let src_table = qualify(src_dialect, src_db, t.schema.as_deref(), &t.name);
    let select_cols = cols.iter().map(|c| quote_ident(src_dialect, &c.name)).collect::<Vec<_>>().join(", ");
    let mut pk: Vec<&ColumnInfo> = cols.iter().filter(|c| c.pk.is_some()).collect();
    pk.sort_by_key(|c| c.pk);
    let order = if pk.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", pk.iter().map(|c| quote_ident(src_dialect, &c.name)).collect::<Vec<_>>().join(", "))
    };
    let insert_cols = cols.iter().map(|c| quote_ident(d, &c.name)).collect::<Vec<_>>().join(", ");
    let batch = opts.batch_size.clamp(1, 10_000);
    let mut offset = 0u64;
    let mut copied = 0u64;
    loop {
        let rs = s.conn_auto(src)?.query(src_db, &format!("SELECT {select_cols} FROM {src_table}{order} LIMIT {batch} OFFSET {offset}"))?;
        if rs.rows.is_empty() {
            break;
        }
        let values: Vec<String> = rs
            .rows
            .iter()
            .map(|row| {
                format!(
                    "({})",
                    cols.iter().enumerate().map(|(i, c)| literal_for(d, c.generic, row.get(i).unwrap_or(&Value::Null))).collect::<Vec<_>>().join(", ")
                )
            })
            .collect();
        emit(s, sink, &format!("INSERT INTO {target} ({insert_cols}) VALUES\n{}", values.join(",\n")))?;
        let n = rs.rows.len() as u64;
        copied += n;
        offset += n;
        s.report(&format!("  {}: {copied} rows", t.name));
        if n < batch {
            break;
        }
    }
    Ok(copied)
}

/// Copy `tables` from `src` (database `src_db`) into `sink`.
pub fn run(s: &mut Session, src: &str, src_db: Option<&str>, tables: &[TableRef], sink: &mut Sink, opts: &Options) -> Result<Summary> {
    if tables.is_empty() {
        return Err(Error::InvalidParam("no tables selected".into()));
    }
    let mut sum = Summary::default();
    for t in tables {
        s.report(&format!("[{}/{}] {}", sum.tables + sum.errors.len() + 1, tables.len(), t.name));
        match copy_table(s, src, src_db, t, sink, opts) {
            Ok(rows) => {
                sum.tables += 1;
                sum.rows += rows;
            }
            Err(e) if opts.continue_on_error => {
                let msg = format!("{}: {e}", t.name);
                s.report(&format!("  error: {e}"));
                sum.errors.push(msg);
            }
            Err(e) => return Err(Error::Database(format!("{}: {e}", t.name))),
        }
    }
    s.report(&format!("Finished: {} tables, {} rows, {} errors", sum.tables, sum.rows, sum.errors.len()));
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn create_table_carries_types_nullability_and_key() {
        let cols = vec![
            ColumnInfo { name: "id".into(), native_type: "int".into(), generic: G::Integer, nullable: false, pk: Some(1) },
            ColumnInfo { name: "code".into(), native_type: "text".into(), generic: G::Text, nullable: true, pk: Some(2) },
            ColumnInfo { name: "note".into(), native_type: "text".into(), generic: G::Text, nullable: true, pk: None },
        ];
        assert_eq!(
            create_table_sql(Dialect::Mysql, "`t`", &cols),
            "CREATE TABLE `t` (\n  `id` INT NOT NULL,\n  `code` VARCHAR(255) NOT NULL,\n  `note` LONGTEXT,\n  PRIMARY KEY (`id`, `code`)\n)"
        );
    }
}
