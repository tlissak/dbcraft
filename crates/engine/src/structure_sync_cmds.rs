//! Tools › Structure Synchronization: compare two databases of the same kind of engine and make
//! the target's structure match the source (tables, keys, indexes, foreign keys, checks,
//! triggers, comments, views), with the deployment script shown first.

use std::sync::Arc;

use dbcraft_core::{Error, Result, TableSchema, ViewInfo};
use dbcraft_drivers::Connection;
use dbcraft_sql::Dialect;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::alter::{self, Place, Plan};
use crate::commands::{CommandSpec, always};
use crate::params::{str_opt, str_req};
use crate::transfer::qualify;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "tools.structureSync",
        label: "Structure Synchronization",
        menu: "Tools/Structure Synchronization",
        shortcut: None,
        params_doc: r#"{"source":{"connection":str,"database":str,"schema"?:str},"target":{"connection":str,"database":str,"schema"?:str},"options"?:{"drop_extra":bool (drop target tables/views not in the source),"views":bool},"deploy"?:bool,"objects"?:[str] (deploy only these)} -> {"differences":[{"object","type":"table|view","action":"create|alter|drop","statements":[str],"warnings":[str]}],"identical":[str],"deployed"?:int,"errors"?:[str]}"#,
        enabled: always,
        run: sync,
    }]
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    pub object: String,
    /// `table` or `view`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `create`, `alter` or `drop`.
    pub action: String,
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    /// SQLite: rebuild the table (foreign keys off).
    pub rebuild: bool,
}

struct Side {
    conn: Arc<dyn Connection>,
    database: String,
    schema: Option<String>,
}

fn side(s: &mut Session, p: &Json, key: &str) -> Result<Side> {
    let v = p.get(key).ok_or_else(|| Error::InvalidParam(format!("`{key}` is required")))?;
    let id = str_req(v, "connection")?.to_string();
    Ok(Side { conn: s.conn_auto(&id)?, database: str_req(v, "database")?.to_string(), schema: str_opt(v, "schema")?.map(str::to_string) })
}

/// Tables of a database, with full schemas (the given schema only, on PostgreSQL).
fn tables(side: &Side) -> Result<Vec<TableSchema>> {
    let infos = side.conn.table_info(&side.database)?;
    let mut out = Vec::new();
    for t in infos.into_iter().filter(|t| t.kind == "table") {
        if side.conn.dialect() == Dialect::Postgres && t.schema.as_deref() != Some(side.schema.as_deref().unwrap_or("public")) {
            continue;
        }
        out.push(side.conn.table_schema(Some(&side.database), t.schema.as_deref(), &t.name)?);
    }
    Ok(out)
}

/// What only makes sense on one server (row counters) or differs by name only.
fn comparable(mut t: TableSchema, schema: Option<&str>) -> TableSchema {
    t.options.remove("auto_increment");
    t.schema = schema.map(str::to_string);
    t
}

fn views(side: &Side) -> Result<Vec<ViewInfo>> {
    let all = side.conn.views(&side.database)?;
    Ok(all
        .into_iter()
        .filter(|v| side.conn.dialect() != Dialect::Postgres || v.schema.as_deref() == Some(side.schema.as_deref().unwrap_or("public")))
        .collect())
}

fn norm(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches(';').to_ascii_lowercase()
}

/// Compare and build each difference's statements.
fn compare(src: &Side, dst: &Side, drop_extra: bool, with_views: bool) -> Result<(Vec<Difference>, Vec<String>)> {
    let d = dst.conn.dialect();
    if src.conn.dialect() != d {
        return Err(Error::Unsupported(
            "Structure Synchronization compares databases of the same kind of engine; use Data Transfer to copy between engines".into(),
        ));
    }
    let tschema = if d == Dialect::Postgres { Some(dst.schema.clone().unwrap_or_else(|| "public".into())) } else { None };
    let at = Place { dialect: d, database: Some(&dst.database), schema: tschema.as_deref() };
    let src_tables: Vec<TableSchema> = tables(src)?.into_iter().map(|t| comparable(t, tschema.as_deref())).collect();
    let dst_tables: Vec<TableSchema> = tables(dst)?.into_iter().map(|t| comparable(t, tschema.as_deref())).collect();
    let order = crate::ddl::dependency_order(&src_tables);
    let mut diffs = Vec::new();
    let mut identical = Vec::new();
    for i in order {
        let Some(t) = src_tables.get(i) else { continue };
        let plan: Plan = match dst_tables.iter().find(|x| x.name == t.name) {
            None => {
                let p = alter::create_plan(t, at)?;
                diffs.push(Difference {
                    object: t.name.clone(),
                    kind: "table".into(),
                    action: "create".into(),
                    statements: p.statements,
                    warnings: p.warnings,
                    rebuild: false,
                });
                continue;
            }
            Some(existing) => {
                let mut target_view = t.clone();
                // Keep the target's own names for things only the engine names (PK constraint).
                target_view.primary_key_name = existing.primary_key_name.clone();
                let origins: Vec<Option<String>> =
                    target_view.columns.iter().map(|c| existing.columns.iter().any(|o| o.name == c.name).then(|| c.name.clone())).collect();
                alter::alter_plan(existing, &target_view, &origins, at)?
            }
        };
        if plan.statements.is_empty() {
            identical.push(t.name.clone());
        } else {
            diffs.push(Difference {
                object: t.name.clone(),
                kind: "table".into(),
                action: "alter".into(),
                statements: plan.statements,
                warnings: plan.warnings,
                rebuild: plan.rebuild,
            });
        }
    }
    if with_views {
        let (sv, dv) = (views(src)?, views(dst)?);
        for v in &sv {
            let name = qualify(d, Some(&dst.database), tschema.as_deref(), &v.name);
            match dv.iter().find(|x| x.name == v.name) {
                Some(x) if norm(&x.definition) == norm(&v.definition) => identical.push(v.name.clone()),
                found => {
                    let statements = match (d, found.is_some()) {
                        (Dialect::Sqlite, true) => vec![format!("DROP VIEW {name}"), crate::ddl::create_view(v, &name)],
                        (_, true) => vec![format!("CREATE OR REPLACE VIEW {name} AS\n{}", v.definition)],
                        (_, false) => vec![crate::ddl::create_view(v, &name)],
                    };
                    let action = if found.is_some() { "alter" } else { "create" };
                    diffs.push(Difference { object: v.name.clone(), kind: "view".into(), action: action.into(), statements, ..Difference::default() });
                }
            }
        }
        if drop_extra {
            for v in dv.iter().filter(|x| !sv.iter().any(|s| s.name == x.name)) {
                let name = qualify(d, Some(&dst.database), tschema.as_deref(), &v.name);
                diffs.push(Difference {
                    object: v.name.clone(),
                    kind: "view".into(),
                    action: "drop".into(),
                    statements: vec![format!("DROP VIEW {name}")],
                    ..Difference::default()
                });
            }
        }
    }
    if drop_extra {
        let extra: Vec<TableSchema> = dst_tables.iter().filter(|x| !src_tables.iter().any(|s| s.name == x.name)).cloned().collect();
        for i in crate::ddl::dependency_order(&extra).into_iter().rev() {
            let Some(t) = extra.get(i) else { continue };
            let cascade = if d == Dialect::Postgres { " CASCADE" } else { "" };
            let name = qualify(d, Some(&dst.database), tschema.as_deref(), &t.name);
            diffs.push(Difference {
                object: t.name.clone(),
                kind: "table".into(),
                action: "drop".into(),
                statements: vec![format!("DROP TABLE {name}{cascade}")],
                ..Difference::default()
            });
        }
    }
    Ok((diffs, identical))
}

fn sync(s: &mut Session, p: &Json) -> Result<Json> {
    let src = side(s, p, "source")?;
    let dst = side(s, p, "target")?;
    let opts = p.get("options").cloned().unwrap_or_else(|| json!({}));
    let drop_extra = opts.get("drop_extra").and_then(Json::as_bool).unwrap_or(false);
    let with_views = opts.get("views").and_then(Json::as_bool).unwrap_or(true);
    s.report(&format!("Compare {} -> {}", src.database, dst.database));
    let (diffs, identical) = compare(&src, &dst, drop_extra, with_views)?;
    s.report(&format!("{} difference(s), {} identical object(s)", diffs.len(), identical.len()));
    let mut out = json!({ "differences": diffs, "identical": identical });
    if p.get("deploy").and_then(Json::as_bool).unwrap_or(false) {
        let only: Option<Vec<String>> = match p.get("objects") {
            None | Some(Json::Null) => None,
            Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("`objects`: {e}")))?),
        };
        let continue_on_error = opts.get("continue_on_error").and_then(Json::as_bool).unwrap_or(false);
        let mut deployed = 0;
        let mut errors = Vec::new();
        // Creates and alters in dependency order (as compared), drops last (already ordered).
        for diff in diffs.iter().filter(|x| only.as_ref().is_none_or(|o| o.contains(&x.object))) {
            s.report(&format!("{} {} {}", diff.action, diff.kind, diff.object));
            let plan = Plan { statements: diff.statements.clone(), warnings: Vec::new(), rebuild: diff.rebuild };
            match crate::table_cmds::run_plan(dst.conn.as_ref(), Some(&dst.database), &plan) {
                Ok(_) => deployed += 1,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => {
                    let msg = format!("{}: {e}", diff.object);
                    s.report(&format!("  error: {msg}"));
                    if !continue_on_error {
                        return Err(Error::Database(msg));
                    }
                    errors.push(msg);
                }
            }
        }
        s.tables.clear();
        s.objects.clear();
        s.report(&format!("Deployed {deployed} change(s), {} error(s)", errors.len()));
        out["deployed"] = json!(deployed);
        out["errors"] = json!(errors);
    }
    Ok(out)
}
