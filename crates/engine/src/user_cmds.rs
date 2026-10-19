//! Users and roles (Navicat: User button, User/Role designers, Privilege Manager).

use dbcraft_core::{Error, Result, UserInfo};
use dbcraft_drivers::users;
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, always};
use crate::params::{str_opt, str_req};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "user.list",
            label: "Users",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str} -> [UserInfo] (no grants)"#,
            enabled: always,
            run: list,
        },
        CommandSpec {
            id: "user.get",
            label: "Edit User",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"name":str,"host"?:str} -> UserInfo (with member_of and grants)"#,
            enabled: always,
            run: get,
        },
        CommandSpec {
            id: "user.save",
            label: "Save User",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"user":UserInfo,"original"?:UserInfo (edit),"password"?:str,"dry_run"?:bool} -> {"sql":[str]}"#,
            enabled: always,
            run: save,
        },
        CommandSpec {
            id: "user.delete",
            label: "Delete User",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"name":str,"host"?:str} -> {"sql":[str]}"#,
            enabled: always,
            run: delete,
        },
        CommandSpec {
            id: "user.privileges",
            label: "Privilege list",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str} -> {"server":[str],"database":[str],"table":[str],"schema":[str]}"#,
            enabled: always,
            run: privileges,
        },
    ]
}

fn user_param(p: &Json, key: &str) -> Result<Option<UserInfo>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| Error::InvalidParam(format!("`{key}`: {e}"))),
    }
}

fn list(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let v = users::list(s.conn_auto(&id)?.as_ref())?;
    serde_json::to_value(v).map_err(|e| Error::InvalidParam(e.to_string()))
}

fn get(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let u = users::get(s.conn_auto(&id)?.as_ref(), str_req(p, "name")?, str_opt(p, "host")?)?;
    serde_json::to_value(u).map_err(|e| Error::InvalidParam(e.to_string()))
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let user = user_param(p, "user")?.ok_or_else(|| Error::InvalidParam("`user` is required".into()))?;
    let original = user_param(p, "original")?;
    let password = str_opt(p, "password")?.filter(|pw| !pw.is_empty());
    let dry = p.get("dry_run").and_then(Json::as_bool).unwrap_or(false);
    let conn = s.conn_auto(&id)?;
    let kind = conn.kind();
    let maria = users::server_is_mariadb(conn.as_ref());
    let sql = users::save_sql(kind, maria, original.as_ref(), &user, password)?;
    if !dry {
        for stmt in &sql {
            conn.query(None, stmt)?;
        }
        s.messages.push(format!("Saved {}", user.display_name()));
    }
    // Never echo the password back.
    let shown: Vec<String> = sql
        .iter()
        .map(|q| match password {
            Some(pw) if !pw.is_empty() => q.replace(pw, "******"),
            _ => q.clone(),
        })
        .collect();
    Ok(json!({ "sql": shown }))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let conn = s.conn_auto(&id)?;
    let kind = conn.kind();
    let maria = users::server_is_mariadb(conn.as_ref());
    let user = users::get(conn.as_ref(), str_req(p, "name")?, str_opt(p, "host")?)?;
    let sql = users::drop_sql(kind, maria, &user)?;
    for stmt in &sql {
        conn.query(None, stmt)?;
    }
    Ok(json!({ "sql": sql }))
}

fn privileges(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let c = users::catalog(s.conn_auto(&id)?.kind())?;
    Ok(json!({ "server": c.server, "database": c.database, "table": c.table, "schema": c.schema }))
}
