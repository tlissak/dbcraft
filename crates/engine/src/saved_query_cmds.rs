//! Saved queries (Navicat: Queries under a database; Save / Save As in the query editor).

use dbcraft_core::{Error, Result};
use dbcraft_model::{SavedQuery, next_id};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, always};
use crate::params::{str_opt, str_req};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "query.save",
            label: "Save Query",
            menu: "File/Save",
            shortcut: Some("Ctrl+S"),
            params_doc: r#"{"name":str,"connection":str,"database"?:str,"sql":str,"id"?:str (overwrite)} -> {"id"}"#,
            enabled: always,
            run: save,
        },
        CommandSpec {
            id: "query.list",
            label: "Queries",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection"?:str,"database"?:str} -> [SavedQuery]"#,
            enabled: always,
            run: list,
        },
        CommandSpec { id: "query.delete", label: "Delete Query", menu: "", shortcut: None, params_doc: r#"{"id":str}"#, enabled: always, run: delete },
    ]
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let name = str_req(p, "name")?.trim();
    if name.is_empty() {
        return Err(Error::InvalidParam("query name must not be empty".into()));
    }
    let connection = str_req(p, "connection")?;
    s.workspace.connection(connection)?;
    let database = str_opt(p, "database")?.map(str::to_string);
    let sql = str_req(p, "sql")?.to_string();
    let id = match str_opt(p, "id")? {
        Some(id) => id.to_string(),
        None => {
            // Navicat refuses a duplicate name in the same place; so do we.
            let dup = s.workspace.queries.iter().any(|q| q.name.eq_ignore_ascii_case(name) && q.connection == connection && q.database == database);
            if dup {
                return Err(Error::InvalidParam(format!("a query named `{name}` already exists here")));
            }
            next_id("query", s.workspace.queries.iter().map(|q| q.id.as_str()))
        }
    };
    let q = SavedQuery { id: id.clone(), name: name.to_string(), connection: connection.to_string(), database, sql, modified: now() };
    match s.workspace.queries.iter_mut().find(|x| x.id == id) {
        Some(slot) => *slot = q,
        None => s.workspace.queries.push(q),
    }
    s.save_workspace()?;
    Ok(json!({ "id": id }))
}

fn list(s: &mut Session, p: &Json) -> Result<Json> {
    let conn = str_opt(p, "connection")?;
    let db = str_opt(p, "database")?;
    let v: Vec<&SavedQuery> =
        s.workspace.queries.iter().filter(|q| conn.is_none_or(|c| q.connection == c)).filter(|q| db.is_none_or(|d| q.database.as_deref() == Some(d))).collect();
    serde_json::to_value(v).map_err(|e| Error::InvalidParam(e.to_string()))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?;
    let before = s.workspace.queries.len();
    s.workspace.queries.retain(|q| q.id != id);
    if s.workspace.queries.len() == before {
        return Err(Error::NotFound(format!("query `{id}`")));
    }
    s.save_workspace()?;
    Ok(json!({}))
}
