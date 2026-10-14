//! Table viewer: paged, filtered, sorted data, and live edits applied in one transaction.

use dbcraft_core::{Error, Result, ResultSet, Value};
use dbcraft_sql::{Dialect, bytes_literal, escape_like, quote_ident, quote_literal};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, has_open};
use crate::params::{str_opt, str_req, u64_opt};

pub const DEFAULT_PAGE: u64 = 1000;
const MAX_PAGE: u64 = 100_000;
const MAX_FILTERS: usize = 64;
const MAX_CHANGES: usize = 10_000;

/// Filter operators (Navicat's filter wizard wording in comments).
pub const FILTER_OPS: &[(&str, &str)] = &[
    ("=", "is equal to"),
    ("<>", "is not equal to"),
    ("<", "is less than"),
    ("<=", "is less than or equal to"),
    (">", "is greater than"),
    (">=", "is greater than or equal to"),
    ("contains", "contains"),
    ("not_contains", "does not contain"),
    ("begins_with", "begins with"),
    ("ends_with", "ends with"),
    ("is_null", "is null"),
    ("is_not_null", "is not null"),
    ("is_empty", "is empty"),
    ("is_not_empty", "is not empty"),
];

/// What `table.data` returns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TablePage {
    pub result: ResultSet,
    /// Rows matching the filter (for "Records x–y of N").
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
    /// Key columns used to address rows when editing (empty: every column is used).
    pub primary_key: Vec<String>,
    /// The SQL that produced the page (Navicat shows it in the status bar).
    pub sql: String,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "table.data",
            label: "Open Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str,"filters"?:[{"column":str,"op":"=|<>|<|<=|>|>=|contains|not_contains|begins_with|ends_with|is_null|is_not_null|is_empty|is_not_empty","value"?:str}],"match"?:"all|any","sort"?:[{"column":str,"desc"?:bool}],"limit"?:int,"offset"?:int} -> TablePage"#,
            enabled: has_open,
            run: data,
        },
        CommandSpec {
            id: "table.applyEdits",
            label: "Apply Changes",
            menu: "",
            shortcut: Some("Ctrl+S"),
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str,"changes":[{"op":"update","key":{col:val},"set":{col:val}} | {"op":"insert","values":{col:val}} | {"op":"delete","key":{col:val}}]} -> {"applied":int}. Values: JSON scalars or Value objects. All-or-nothing: an update/delete that doesn't hit exactly one row rolls everything back."#,
            enabled: has_open,
            run: apply_edits,
        },
    ]
}

fn qualified(d: Dialect, database: Option<&str>, schema: Option<&str>, table: &str) -> String {
    let t = quote_ident(d, table);
    match (d, database, schema) {
        (Dialect::Postgres | Dialect::Mssql, _, Some(s)) => format!("{}.{t}", quote_ident(d, s)),
        (Dialect::Mysql | Dialect::Sqlite, Some(db), _) => format!("{}.{t}", quote_ident(d, db)),
        _ => t,
    }
}

fn as_text(d: Dialect, col: &str) -> String {
    match d {
        Dialect::Postgres => format!("CAST({col} AS TEXT)"),
        _ => col.to_string(),
    }
}

fn filter_sql(d: Dialect, f: &Json) -> Result<String> {
    let column = str_req(f, "column")?;
    let op = str_req(f, "op")?;
    let col = quote_ident(d, column);
    let value = || -> Result<&str> { str_opt(f, "value")?.ok_or_else(|| Error::InvalidParam(format!("filter `{op}` on `{column}` needs a value"))) };
    let like =
        |pattern: String, negate: bool| format!("{} {}LIKE {} ESCAPE '!'", as_text(d, &col), if negate { "NOT " } else { "" }, quote_literal(d, &pattern));
    Ok(match op {
        "=" | "<>" | "<" | "<=" | ">" | ">=" => format!("{col} {op} {}", quote_literal(d, value()?)),
        "contains" => like(format!("%{}%", escape_like(value()?)), false),
        "not_contains" => like(format!("%{}%", escape_like(value()?)), true),
        "begins_with" => like(format!("{}%", escape_like(value()?)), false),
        "ends_with" => like(format!("%{}", escape_like(value()?)), false),
        "is_null" => format!("{col} IS NULL"),
        "is_not_null" => format!("{col} IS NOT NULL"),
        "is_empty" => format!("{} = ''", as_text(d, &col)),
        "is_not_empty" => format!("({col} IS NOT NULL AND {} <> '')", as_text(d, &col)),
        other => return Err(Error::InvalidParam(format!("unknown filter operator `{other}`"))),
    })
}

fn where_clause(d: Dialect, p: &Json) -> Result<String> {
    let filters = match p.get("filters") {
        None | Some(Json::Null) => return Ok(String::new()),
        Some(Json::Array(a)) => a,
        Some(_) => return Err(Error::InvalidParam("`filters` must be an array".into())),
    };
    if filters.len() > MAX_FILTERS {
        return Err(Error::InvalidParam(format!("at most {MAX_FILTERS} filters")));
    }
    if filters.is_empty() {
        return Ok(String::new());
    }
    let joiner = match str_opt(p, "match")?.unwrap_or("all") {
        "all" => " AND ",
        "any" => " OR ",
        other => return Err(Error::InvalidParam(format!("`match` must be `all` or `any`, not `{other}`"))),
    };
    let parts = filters.iter().map(|f| filter_sql(d, f)).collect::<Result<Vec<_>>>()?;
    Ok(format!(" WHERE {}", parts.iter().map(|s| format!("({s})")).collect::<Vec<_>>().join(joiner)))
}

fn order_clause(d: Dialect, p: &Json) -> Result<String> {
    let sorts = match p.get("sort") {
        None | Some(Json::Null) => return Ok(String::new()),
        Some(Json::Array(a)) => a,
        Some(_) => return Err(Error::InvalidParam("`sort` must be an array".into())),
    };
    let parts = sorts
        .iter()
        .take(MAX_FILTERS)
        .map(|s| {
            let desc = s.get("desc").and_then(Json::as_bool).unwrap_or(false);
            Ok(format!("{}{}", quote_ident(d, str_req(s, "column")?), if desc { " DESC" } else { "" }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(if parts.is_empty() { String::new() } else { format!(" ORDER BY {}", parts.join(", ")) })
}

fn data(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?;
    let db = str_opt(p, "database")?;
    let schema = str_opt(p, "schema")?;
    let table = str_req(p, "table")?;
    let limit = u64_opt(p, "limit")?.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let offset = u64_opt(p, "offset")?.unwrap_or(0);
    let conn = s.conn(id)?;
    let d = conn.dialect();
    let from = qualified(d, db, schema, table);
    let filter = where_clause(d, p)?;
    let order = order_clause(d, p)?;
    let sql = format!("SELECT * FROM {from}{filter}{order} LIMIT {limit} OFFSET {offset}");
    let result = conn.query(db, &sql)?;
    let count = conn.query(db, &format!("SELECT COUNT(*) FROM {from}{filter}"))?;
    let total = match count.rows.first().and_then(|r| r.first()) {
        Some(Value::Int(n)) => u64::try_from(*n).unwrap_or(0),
        Some(Value::UInt(n)) => *n,
        _ => 0,
    };
    let primary_key = conn.primary_key(db, schema, table).unwrap_or_default();
    let page = TablePage { result, total, offset, limit, primary_key, sql };
    serde_json::to_value(&page).map_err(|e| Error::InvalidParam(e.to_string()))
}

/// Accept a tagged `Value` (`{"t":"Text","v":"x"}`) or a plain JSON scalar.
fn json_to_value(v: &Json) -> Result<Value> {
    if let Json::Object(o) = v
        && o.contains_key("t")
    {
        return serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("bad value {v}: {e}")));
    }
    Ok(match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => Value::Int(i),
            (_, Some(u), _) => Value::UInt(u),
            (_, _, Some(f)) => Value::Float(f),
            _ => return Err(Error::InvalidParam(format!("bad number {n}"))),
        },
        Json::String(s) => Value::Text(s.clone()),
        other => return Err(Error::InvalidParam(format!("unsupported value {other}"))),
    })
}

fn literal(d: Dialect, v: &Value) -> Result<String> {
    Ok(match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) if f.is_finite() => f.to_string(),
        Value::Float(f) => return Err(Error::InvalidParam(format!("{f} can't be stored"))),
        Value::Text(s) => quote_literal(d, s),
        Value::Bytes(b) => bytes_literal(d, b),
    })
}

fn object<'a>(c: &'a Json, key: &str) -> Result<&'a serde_json::Map<String, Json>> {
    c.get(key).and_then(Json::as_object).ok_or_else(|| Error::InvalidParam(format!("change needs a `{key}` object")))
}

fn key_where(d: Dialect, key: &serde_json::Map<String, Json>) -> Result<String> {
    if key.is_empty() {
        return Err(Error::InvalidParam("`key` must name at least one column".into()));
    }
    let parts = key
        .iter()
        .map(|(col, v)| {
            let c = quote_ident(d, col);
            Ok(match json_to_value(v)? {
                Value::Null => format!("{c} IS NULL"),
                val => format!("{c} = {}", literal(d, &val)?),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(parts.join(" AND "))
}

fn apply_edits(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?;
    let db = str_opt(p, "database")?;
    let schema = str_opt(p, "schema")?;
    let table = str_req(p, "table")?;
    let changes = p.get("changes").and_then(Json::as_array).ok_or_else(|| Error::InvalidParam("`changes` (array) is required".into()))?;
    if changes.is_empty() {
        return Ok(json!({ "applied": 0 }));
    }
    if changes.len() > MAX_CHANGES {
        return Err(Error::InvalidParam(format!("at most {MAX_CHANGES} changes per apply")));
    }
    let conn = s.conn(id)?;
    let d = conn.dialect();
    let t = qualified(d, db, schema, table);
    let mut stmts = Vec::with_capacity(changes.len());
    for c in changes {
        stmts.push(match str_req(c, "op")? {
            "update" => {
                let set = object(c, "set")?;
                if set.is_empty() {
                    continue;
                }
                let assigns =
                    set.iter().map(|(col, v)| Ok(format!("{} = {}", quote_ident(d, col), literal(d, &json_to_value(v)?)?))).collect::<Result<Vec<_>>>()?;
                (format!("UPDATE {t} SET {} WHERE {}", assigns.join(", "), key_where(d, object(c, "key")?)?), Some(1))
            }
            "delete" => (format!("DELETE FROM {t} WHERE {}", key_where(d, object(c, "key")?)?), Some(1)),
            "insert" => {
                let values = object(c, "values")?;
                let sql = if values.is_empty() {
                    match d {
                        Dialect::Mysql => format!("INSERT INTO {t} () VALUES ()"),
                        _ => format!("INSERT INTO {t} DEFAULT VALUES"),
                    }
                } else {
                    let cols: Vec<String> = values.keys().map(|k| quote_ident(d, k)).collect();
                    let vals = values.values().map(|v| literal(d, &json_to_value(v)?)).collect::<Result<Vec<_>>>()?;
                    format!("INSERT INTO {t} ({}) VALUES ({})", cols.join(", "), vals.join(", "))
                };
                (sql, None)
            }
            other => return Err(Error::InvalidParam(format!("unknown change op `{other}`"))),
        });
    }
    let counts = conn.execute_atomic(db, &stmts)?;
    for (sql, _) in &stmts {
        s.messages.push(sql.clone());
    }
    s.messages.push(format!("> Applied {} changes", counts.len()));
    Ok(json!({ "applied": counts.len() }))
}
