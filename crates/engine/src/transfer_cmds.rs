//! Tools › Data Transfer.

use dbcraft_core::{DbKind, Error, Result};
use dbcraft_sql::Dialect;
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, always};
use crate::params::{str_opt, str_req};
use crate::transfer::{self, Options, Sink, TableRef};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "tools.dataTransfer",
        label: "Data Transfer",
        menu: "Tools/Data Transfer",
        shortcut: None,
        params_doc: r#"{"source":{"connection":str,"database"?:str},"target":{"connection":str,"database"?:str,"schema"?:str} | {"file":path,"dialect":"mysql|postgres|sqlite"},"tables"?:[{"name":str,"schema"?:str}] (default: all tables),"options"?:{"create_tables":bool,"drop_existing":bool,"create_records":bool,"continue_on_error":bool,"batch_size":int}} -> {"tables","rows","errors"}"#,
        enabled: always,
        run: data_transfer,
    }]
}

pub fn dialect_param(name: &str) -> Result<Dialect> {
    match DbKind::parse(name) {
        Some(DbKind::Sqlite) => Ok(Dialect::Sqlite),
        Some(DbKind::Postgres) => Ok(Dialect::Postgres),
        Some(DbKind::Mysql | DbKind::Mariadb) => Ok(Dialect::Mysql),
        _ => Err(Error::InvalidParam(format!("unsupported SQL dialect `{name}` (mysql, postgres or sqlite)"))),
    }
}

/// `tables` param, or every base table of the source database.
pub fn tables_param(s: &mut Session, p: &Json, src: &str, src_db: Option<&str>) -> Result<Vec<TableRef>> {
    match p.get("tables") {
        None | Some(Json::Null) => {
            let db = match src_db {
                Some(d) => d.to_string(),
                None => s.conn_auto(src)?.list_databases()?.into_iter().next().ok_or_else(|| Error::InvalidParam("`source.database` is required".into()))?,
            };
            Ok(s.conn_auto(src)?.table_info(&db)?.into_iter().filter(|t| t.kind == "table").map(|t| TableRef { schema: t.schema, name: t.name }).collect())
        }
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("`tables`: {e}"))),
    }
}

pub fn options_param(p: &Json) -> Result<Options> {
    match p.get("options") {
        None | Some(Json::Null) => Ok(Options::default()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("`options`: {e}"))),
    }
}

fn data_transfer(s: &mut Session, p: &Json) -> Result<Json> {
    let source = p.get("source").ok_or_else(|| Error::InvalidParam("`source` is required".into()))?;
    let target = p.get("target").ok_or_else(|| Error::InvalidParam("`target` is required".into()))?;
    let src = str_req(source, "connection")?.to_string();
    let src_db = str_opt(source, "database")?.map(str::to_string);
    let opts = options_param(p)?;
    let tables = tables_param(s, p, &src, src_db.as_deref())?;
    let summary = if let Some(path) = str_opt(target, "file")? {
        if path.trim().is_empty() {
            return Err(Error::InvalidParam("`target.file` must not be empty".into()));
        }
        let dialect = dialect_param(str_req(target, "dialect")?)?;
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        use std::io::Write;
        writeln!(file, "-- DbCraft Data Transfer\n-- Source: {src} {}", src_db.as_deref().unwrap_or(""))?;
        let sum = transfer::run(s, &src, src_db.as_deref(), &tables, &mut Sink::File { out: &mut file, dialect, keep_schema: false }, &opts)?;
        file.flush()?;
        sum
    } else {
        let id = str_req(target, "connection")?.to_string();
        let database = str_opt(target, "database")?.map(str::to_string);
        let schema = str_opt(target, "schema")?.map(str::to_string);
        if id == src && database == src_db && opts.drop_existing {
            return Err(Error::InvalidParam("source and target are the same database: refusing to drop the source tables".into()));
        }
        transfer::run(s, &src, src_db.as_deref(), &tables, &mut Sink::Conn { id, database, schema }, &opts)?
    };
    Ok(json!({ "tables": summary.tables, "rows": summary.rows, "errors": summary.errors }))
}
