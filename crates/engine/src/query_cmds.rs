//! Query editor and table viewer.

use dbcraft_core::{Error, Result, ResultSet};
use dbcraft_sql::quote_ident;
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, has_open};
use crate::params::{str_opt, str_req, u64_opt};

/// Rows the table viewer loads per page (Navicat's default is 1000).
pub const DEFAULT_PAGE: u64 = 1000;
const MAX_PAGE: u64 = 100_000;
/// Result tabs kept per run (Navicat shows up to 20).
const MAX_RESULTS: usize = 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "query.run",
            label: "Run",
            menu: "Query/Run",
            shortcut: Some("Ctrl+R"),
            params_doc: r#"{"connection":str,"database"?:str,"sql":str (one or more statements)} -> {"statements":int,"result":ResultSet (last statement),"results":[ResultSet] (one per row-returning statement, max 20)}"#,
            enabled: has_open,
            run: run_sql,
        },
        CommandSpec {
            id: "table.open",
            label: "Open Table",
            menu: "Table/Open Table",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"table":str,"schema"?:str,"limit"?:int (default 1000),"offset"?:int} -> ResultSet"#,
            enabled: has_open,
            run: open_table,
        },
    ]
}

fn run_sql(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let db = str_opt(p, "database")?.map(str::to_string);
    let sql = str_req(p, "sql")?;
    let conn = s.conn(&id)?;
    let stmts = dbcraft_sql::split_statements(conn.dialect(), sql);
    if stmts.is_empty() {
        return Err(Error::InvalidParam("no SQL to run".into()));
    }
    let mut last = ResultSet::default();
    let mut results = Vec::new();
    let mut log = Vec::new();
    for stmt in &stmts {
        let rs = match conn.query(db.as_deref(), stmt) {
            Ok(rs) => rs,
            Err(e) => {
                log.push(format!("{stmt}\n> {e}"));
                s.messages.extend(log);
                return Err(e);
            }
        };
        log.push(if rs.columns.is_empty() {
            format!("{stmt}\n> Affected rows: {}\n> Time: {}ms", rs.rows_affected, rs.elapsed_ms)
        } else {
            format!("{stmt}\n> OK, {} rows\n> Time: {}ms", rs.rows.len(), rs.elapsed_ms)
        });
        // One result tab per row-returning statement, up to Navicat's 20.
        if !rs.columns.is_empty() && results.len() < MAX_RESULTS {
            results.push(rs.clone());
        }
        last = rs;
    }
    if results.is_empty() {
        results.push(last.clone());
    }
    s.messages.extend(log);
    let out = json!({ "statements": stmts.len(), "result": last, "results": results });
    s.last_result = Some(last);
    Ok(out)
}

fn open_table(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let db = str_opt(p, "database")?.map(str::to_string);
    let table = str_req(p, "table")?;
    let schema = str_opt(p, "schema")?;
    let limit = u64_opt(p, "limit")?.unwrap_or(DEFAULT_PAGE).min(MAX_PAGE);
    let offset = u64_opt(p, "offset")?.unwrap_or(0);
    let conn = s.conn(&id)?;
    let d = conn.dialect();
    let name = match schema {
        Some(sch) => format!("{}.{}", quote_ident(d, sch), quote_ident(d, table)),
        None => quote_ident(d, table),
    };
    let sql = format!("SELECT * FROM {name} LIMIT {limit} OFFSET {offset}");
    let rs = conn.query(db.as_deref(), &sql)?;
    let out = serde_json::to_value(&rs).map_err(|e| Error::InvalidParam(e.to_string()))?;
    s.last_result = Some(rs);
    Ok(out)
}
