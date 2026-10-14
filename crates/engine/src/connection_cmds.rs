//! Connection pane: create, edit, delete, open, close, browse.

use dbcraft_core::{ConnectionConfig, DbKind, Error, Result, SshAuth, SshConfig};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, always, has_open};
use crate::params::{str_opt, str_req, u64_opt};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "connection.save",
            label: "New Connection",
            menu: "File/New Connection",
            shortcut: None,
            params_doc: r#"{"kind":"sqlite|postgres|mysql|mariadb|…","name":str,"id"?:str (edit existing),"host"?:str,"port"?:int,"user"?:str,"password"?:str,"database"?:str (SQLite: file path),"ssh"?:{"host":str,"port"?:int,"user":str,"auth"?:"password|publickey","password"?:str (or key passphrase),"private_key"?:path}} -> {"id"}"#,
            enabled: always,
            run: save,
        },
        CommandSpec {
            id: "connection.test",
            label: "Test Connection",
            menu: "",
            shortcut: None,
            params_doc: r#"same params as connection.save (nothing is saved) -> {"ok":true,"version":str,"via_ssh":bool}"#,
            enabled: always,
            run: test,
        },
        CommandSpec {
            id: "database.tables",
            label: "Tables",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database":str} -> [{"name","schema"?,"kind","rows","data_length","engine","owner","created","modified","collation","comment"}]"#,
            enabled: has_open,
            run: tables,
        },
        CommandSpec {
            id: "table.ddl",
            label: "DDL",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database":str,"table":str,"schema"?:str} -> {"ddl":str}"#,
            enabled: has_open,
            run: ddl,
        },
        CommandSpec {
            id: "connection.delete",
            label: "Delete Connection",
            menu: "File/Delete Connection",
            shortcut: None,
            params_doc: r#"{"id":str}"#,
            enabled: always,
            run: delete,
        },
        CommandSpec {
            id: "connection.list",
            label: "List Connections",
            menu: "",
            shortcut: None,
            params_doc: r#"{} -> [{"id","name","kind","host","port","user","database","open"}] (no passwords)"#,
            enabled: always,
            run: list,
        },
        CommandSpec {
            id: "connection.open",
            label: "Open Connection",
            menu: "File/Open Connection",
            shortcut: None,
            params_doc: r#"{"id":str} -> {"version":str,"databases":[str]}"#,
            enabled: always,
            run: open,
        },
        CommandSpec {
            id: "connection.close",
            label: "Close Connection",
            menu: "File/Close Connection",
            shortcut: None,
            params_doc: r#"{"id":str}"#,
            enabled: has_open,
            run: close,
        },
        CommandSpec {
            id: "database.objects",
            label: "Refresh",
            menu: "View/Refresh",
            shortcut: Some("F5"),
            params_doc: r#"{"connection":str,"database":str} -> [{"kind","name","schema"?}]"#,
            enabled: has_open,
            run: objects,
        },
    ]
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let id = match str_opt(p, "id")? {
        Some(id) => id.to_string(),
        None => s.workspace.next_id(),
    };
    let cfg = config_from_params(p, id.clone())?;
    s.workspace.upsert(cfg);
    s.save_workspace()?;
    Ok(json!({ "id": id }))
}

fn port_param(p: &Json, key: &str) -> Result<Option<u16>> {
    u64_opt(p, key)?.map(|n| u16::try_from(n).map_err(|_| Error::InvalidParam(format!("`{key}` {n} is out of range")))).transpose()
}

/// Build a connection config from `connection.save` / `connection.test` params.
fn config_from_params(p: &Json, id: String) -> Result<ConnectionConfig> {
    let kind_str = str_req(p, "kind")?;
    let kind = DbKind::parse(kind_str).ok_or_else(|| Error::InvalidParam(format!("unknown kind `{kind_str}`")))?;
    let name = str_req(p, "name")?.trim();
    if name.is_empty() {
        return Err(Error::InvalidParam("`name` must not be empty".into()));
    }
    let ssh = match p.get("ssh") {
        None | Some(Json::Null) => None,
        Some(sp) if sp.is_object() => {
            let auth = match str_opt(sp, "auth")?.unwrap_or("password") {
                "password" => SshAuth::Password,
                "publickey" => SshAuth::PublicKey,
                other => return Err(Error::InvalidParam(format!("ssh.auth must be `password` or `publickey`, not `{other}`"))),
            };
            Some(SshConfig {
                host: str_req(sp, "host")?.trim().to_string(),
                port: port_param(sp, "port")?.unwrap_or(22),
                user: str_req(sp, "user")?.trim().to_string(),
                auth,
                password: str_opt(sp, "password")?.map(str::to_string),
                private_key: str_opt(sp, "private_key")?.map(str::to_string),
            })
        }
        Some(_) => return Err(Error::InvalidParam("`ssh` must be an object".into())),
    };
    Ok(ConnectionConfig {
        id,
        name: name.to_string(),
        kind,
        host: str_opt(p, "host")?.unwrap_or("").trim().to_string(),
        port: port_param(p, "port")?.or(kind.default_port()),
        user: str_opt(p, "user")?.unwrap_or("").to_string(),
        password: str_opt(p, "password")?.map(str::to_string),
        database: str_opt(p, "database")?.unwrap_or("").to_string(),
        ssh,
    })
}

/// Navicat's "Test Connection": connect with unsaved settings, report the server, disconnect.
fn test(_: &mut Session, p: &Json) -> Result<Json> {
    let cfg = config_from_params(p, "test".into())?;
    let mut conn = dbcraft_drivers::connect(&cfg)?;
    let version = conn.server_version()?;
    Ok(json!({ "ok": true, "version": version, "via_ssh": cfg.ssh.is_some() }))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?;
    s.workspace.remove(id)?;
    s.open.remove(id);
    s.databases.remove(id);
    s.objects.retain(|(c, _), _| c != id);
    s.tables.retain(|(c, _), _| c != id);
    s.save_workspace()?;
    Ok(json!({}))
}

fn list(s: &mut Session, _: &Json) -> Result<Json> {
    Ok(Json::Array(
        s.workspace
            .connections
            .iter()
            .map(|c| {
                json!({ "id": c.id, "name": c.name, "kind": c.kind, "host": c.host, "port": c.port,
                        "user": c.user, "database": c.database, "open": s.open.contains_key(&c.id) })
            })
            .collect(),
    ))
}

fn open(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?.to_string();
    let cfg = s.workspace.connection(&id)?.clone();
    let mut conn = dbcraft_drivers::connect(&cfg)?;
    let version = conn.server_version()?;
    let dbs = conn.list_databases()?;
    s.open.insert(id.clone(), conn);
    s.databases.insert(id.clone(), dbs.clone());
    s.messages.push(format!("Connected to {} ({version})", cfg.name));
    Ok(json!({ "version": version, "databases": dbs }))
}

fn close(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?;
    if s.open.remove(id).is_none() {
        return Err(Error::InvalidParam(format!("connection `{id}` is not open")));
    }
    s.databases.remove(id);
    s.objects.retain(|(c, _), _| c != id);
    s.tables.retain(|(c, _), _| c != id);
    Ok(json!({}))
}

fn tables(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let db = str_req(p, "database")?.to_string();
    let infos = s.conn(&id)?.table_info(&db)?;
    let out = serde_json::to_value(&infos).map_err(|e| Error::InvalidParam(e.to_string()))?;
    s.tables.insert((id, db), infos);
    Ok(out)
}

fn ddl(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?;
    let db = str_req(p, "database")?;
    let table = str_req(p, "table")?;
    let schema = str_opt(p, "schema")?;
    Ok(json!({ "ddl": s.conn(id)?.table_ddl(db, schema, table)? }))
}

fn objects(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "connection")?.to_string();
    let db = str_req(p, "database")?.to_string();
    let objs = s.conn(&id)?.list_objects(&db)?;
    let out = serde_json::to_value(&objs).map_err(|e| Error::InvalidParam(e.to_string()))?;
    s.objects.insert((id, db), objs);
    Ok(out)
}
