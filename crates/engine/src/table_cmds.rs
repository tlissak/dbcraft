//! Table Designer and table actions: design, new, delete, empty, truncate, rename, duplicate.

use dbcraft_core::{Error, Result, TableSchema};
use dbcraft_drivers::Connection;
use dbcraft_sql::{Dialect, quote_ident};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::alter::{self, Place, Plan};
use crate::commands::{CommandSpec, has_open};
use crate::params::{str_opt, str_req};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "table.schema",
            label: "Design Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str} -> TableSchema (fields, indexes, foreign keys, uniques, checks, triggers, options, comment)"#,
            enabled: has_open,
            run: schema,
        },
        CommandSpec {
            id: "table.designSql",
            label: "SQL Preview",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":TableSchema,"original"?:TableSchema (absent: new table),"origins"?:[str|null] (saved name of each field)} -> {"statements":[str],"warnings":[str],"rebuild":bool}"#,
            enabled: has_open,
            run: design_sql,
        },
        CommandSpec {
            id: "table.save",
            label: "Save Table",
            menu: "",
            shortcut: Some("Ctrl+S"),
            params_doc: r#"same params as table.designSql; runs the statements -> {"statements":[str],"warnings":[str]}"#,
            enabled: has_open,
            run: save,
        },
        CommandSpec {
            id: "table.delete",
            label: "Delete Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"tables":[str]} -> {"dropped":int}"#,
            enabled: has_open,
            run: delete,
        },
        CommandSpec {
            id: "table.empty",
            label: "Empty Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str} -> {"deleted":int} (DELETE FROM: triggers fire, auto-increment kept)"#,
            enabled: has_open,
            run: empty,
        },
        CommandSpec {
            id: "table.truncate",
            label: "Truncate Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str} -> {} (TRUNCATE: fast, resets auto-increment; SQLite: DELETE FROM)"#,
            enabled: has_open,
            run: truncate,
        },
        CommandSpec {
            id: "table.rename",
            label: "Rename Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str,"new_name":str} -> {}"#,
            enabled: has_open,
            run: rename,
        },
        CommandSpec {
            id: "table.duplicate",
            label: "Duplicate Table",
            menu: "",
            shortcut: None,
            params_doc: r#"{"connection":str,"database"?:str,"schema"?:str,"table":str,"new_name"?:str (default `<table>_copy`),"data"?:bool (default true)} -> {"table":str}"#,
            enabled: has_open,
            run: duplicate,
        },
    ]
}

struct Target {
    conn: std::sync::Arc<dyn Connection>,
    id: String,
    database: Option<String>,
    schema: Option<String>,
}

impl Target {
    fn from(s: &Session, p: &Json) -> Result<Target> {
        let id = str_req(p, "connection")?.to_string();
        Ok(Target { conn: s.conn(&id)?, id, database: str_opt(p, "database")?.map(str::to_string), schema: str_opt(p, "schema")?.map(str::to_string) })
    }

    fn place(&self) -> Place<'_> {
        Place { dialect: self.conn.dialect(), database: self.database.as_deref(), schema: self.schema.as_deref() }
    }

    fn table(&self, name: &str) -> String {
        crate::transfer::qualify(self.conn.dialect(), self.database.as_deref(), self.schema.as_deref(), name)
    }

    /// The Objects tab and tree must re-read this database.
    fn invalidate(&self, s: &mut Session) {
        let keep = |c: &String, d: &String| !(c == &self.id && self.database.as_ref().is_none_or(|db| db == d));
        s.tables.retain(|(c, d), _| keep(c, d));
        s.objects.retain(|(c, d), _| keep(c, d));
    }
}

fn schema(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let sch = t.conn.table_schema(t.database.as_deref(), t.schema.as_deref(), str_req(p, "table")?)?;
    serde_json::to_value(sch).map_err(|e| Error::InvalidParam(e.to_string()))
}

fn table_param(p: &Json, key: &str) -> Result<Option<TableSchema>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| Error::InvalidParam(format!("`{key}`: {e}"))),
    }
}

fn plan(t: &Target, p: &Json) -> Result<Plan> {
    let table = table_param(p, "table")?.ok_or_else(|| Error::InvalidParam("`table` (the edited definition) is required".into()))?;
    match table_param(p, "original")? {
        None => alter::create_plan(&table, t.place()),
        Some(original) => {
            let origins: Vec<Option<String>> = match p.get("origins") {
                None | Some(Json::Null) => table.columns.iter().map(|c| original.columns.iter().any(|o| o.name == c.name).then(|| c.name.clone())).collect(),
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("`origins`: {e}")))?,
            };
            alter::alter_plan(&original, &table, &origins, t.place())
        }
    }
}

fn design_sql(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    serde_json::to_value(plan(&t, p)?).map_err(|e| Error::InvalidParam(e.to_string()))
}

/// Run a plan: one transaction where the engine allows it (PostgreSQL, SQLite); statement by
/// statement on MySQL (its DDL commits by itself); SQLite rebuilds with foreign keys off.
pub fn run_plan(conn: &dyn Connection, db: Option<&str>, plan: &Plan) -> Result<Vec<String>> {
    let mut warnings = plan.warnings.clone();
    if plan.statements.is_empty() {
        return Ok(warnings);
    }
    let stmts: Vec<(String, Option<u64>)> = plan.statements.iter().map(|s| (s.clone(), None)).collect();
    if plan.rebuild {
        // One session: PRAGMAs apply to the connection and can't change inside a transaction.
        let own = conn.dedicated()?;
        own.query(db, "PRAGMA foreign_keys = OFF")?;
        own.query(db, "PRAGMA legacy_alter_table = ON")?;
        let r = own.execute_atomic(db, &stmts);
        let check = own.query(db, "PRAGMA foreign_key_check");
        let _ = own.query(db, "PRAGMA legacy_alter_table = OFF");
        let _ = own.query(db, "PRAGMA foreign_keys = ON");
        r?;
        if let Ok(rs) = check
            && !rs.rows.is_empty()
        {
            warnings.push(format!("{} row(s) now break a foreign key (PRAGMA foreign_key_check)", rs.rows.len()));
        }
        return Ok(warnings);
    }
    if conn.dialect() == Dialect::Mysql {
        for (i, st) in stmts.iter().enumerate() {
            conn.execute_atomic(db, std::slice::from_ref(st)).map_err(|e| {
                let done = if i == 0 { String::new() } else { format!(" (the {i} statement(s) before it were applied)") };
                Error::Database(format!("statement {} failed{done}: {e}", i + 1))
            })?;
        }
        return Ok(warnings);
    }
    conn.execute_atomic(db, &stmts)?;
    Ok(warnings)
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let plan = plan(&t, p)?;
    let warnings = run_plan(t.conn.as_ref(), t.database.as_deref(), &plan)?;
    for st in &plan.statements {
        s.messages.push(format!("{st};"));
    }
    t.invalidate(s);
    Ok(json!({ "statements": plan.statements, "warnings": warnings }))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let names: Vec<String> = match (p.get("tables"), str_opt(p, "table")?) {
        (Some(v), _) => serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("`tables`: {e}")))?,
        (None, Some(one)) => vec![one.to_string()],
        (None, None) => return Err(Error::InvalidParam("`tables` is required".into())),
    };
    if names.is_empty() {
        return Err(Error::InvalidParam("`tables` is empty".into()));
    }
    let mut dropped = 0;
    for n in &names {
        let sql = format!("DROP TABLE {}", t.table(n));
        t.conn.execute_atomic(t.database.as_deref(), &[(sql.clone(), None)]).map_err(|e| Error::Database(format!("{n}: {e}")))?;
        s.messages.push(format!("{sql};"));
        dropped += 1;
    }
    t.invalidate(s);
    Ok(json!({ "dropped": dropped }))
}

fn empty(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let n = t.conn.execute_atomic(t.database.as_deref(), &[(format!("DELETE FROM {}", t.table(str_req(p, "table")?)), None)])?;
    t.invalidate(s);
    Ok(json!({ "deleted": n.first().copied().unwrap_or(0) }))
}

fn truncate(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let name = t.table(str_req(p, "table")?);
    let sql = match t.conn.dialect() {
        Dialect::Sqlite => format!("DELETE FROM {name}"),
        _ => format!("TRUNCATE TABLE {name}"),
    };
    t.conn.execute_atomic(t.database.as_deref(), &[(sql, None)])?;
    if t.conn.dialect() == Dialect::Sqlite {
        // Reset AUTOINCREMENT counters too, like TRUNCATE (the table may not have one).
        let _ = t.conn.execute_atomic(
            t.database.as_deref(),
            &[(format!("DELETE FROM sqlite_sequence WHERE name = {}", dbcraft_sql::quote_literal(Dialect::Sqlite, str_req(p, "table")?)), None)],
        );
    }
    t.invalidate(s);
    Ok(json!({}))
}

fn rename(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let new_name = str_req(p, "new_name")?.trim();
    if new_name.is_empty() {
        return Err(Error::InvalidParam("`new_name` must not be empty".into()));
    }
    let d = t.conn.dialect();
    let to = if d == Dialect::Mysql { t.table(new_name) } else { quote_ident(d, new_name) };
    t.conn.execute_atomic(t.database.as_deref(), &[(format!("ALTER TABLE {} RENAME TO {to}", t.table(str_req(p, "table")?)), None)])?;
    t.invalidate(s);
    Ok(json!({}))
}

fn duplicate(s: &mut Session, p: &Json) -> Result<Json> {
    let t = Target::from(s, p)?;
    let src = str_req(p, "table")?;
    let new_name = str_opt(p, "new_name")?.map(str::to_string).unwrap_or_else(|| format!("{src}_copy"));
    let with_data = p.get("data").and_then(Json::as_bool).unwrap_or(true);
    let mut sch = t.conn.table_schema(t.database.as_deref(), t.schema.as_deref(), src)?;
    sch.name = new_name.clone();
    // Index, constraint and trigger names must be unique per schema/database: suffix them.
    let suffix = |n: &mut String| *n = format!("{n}_{new_name}");
    sch.indexes.iter_mut().for_each(|i| {
        suffix(&mut i.name);
        // An expression index's native DDL names the old table and index: not reusable.
        i.native_ddl = None;
    });
    sch.indexes.retain(|i| !i.columns.is_empty());
    sch.uniques.iter_mut().for_each(|u| suffix(&mut u.name));
    sch.checks.iter_mut().for_each(|c| suffix(&mut c.name));
    sch.foreign_keys.iter_mut().for_each(|f| suffix(&mut f.name));
    sch.triggers.clear();
    sch.primary_key_name = None;
    let mut plan = alter::create_plan(&sch, t.place())?;
    if with_data {
        let cols = sch.columns.iter().map(|c| quote_ident(t.conn.dialect(), &c.name)).collect::<Vec<_>>().join(", ");
        plan.statements.insert(1, format!("INSERT INTO {} ({cols}) SELECT {cols} FROM {}", t.table(&new_name), t.table(src)));
        plan.statements.extend(crate::ddl::reset_sequences(&sch, &t.table(&new_name), t.conn.dialect()));
    }
    run_plan(t.conn.as_ref(), t.database.as_deref(), &plan)?;
    t.invalidate(s);
    Ok(json!({ "table": new_name }))
}
