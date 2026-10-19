//! Table Designer SQL: `CREATE TABLE` for a new table, or the statements that turn the saved
//! definition into the edited one (SQL Preview, then Save).
//!
//! - PostgreSQL: `ALTER TABLE … RENAME / ALTER COLUMN / ADD|DROP CONSTRAINT`, run in one
//!   transaction. Columns can't be reordered (reported).
//! - MySQL/MariaDB: one `ALTER TABLE` with `CHANGE COLUMN … AFTER` (so reordering works), with
//!   foreign keys dropped before and added after.
//! - SQLite: native `ALTER` for renames, appended columns and dropped plain columns; anything else
//!   uses SQLite's documented rebuild (new table, copy, drop, rename) with foreign keys off.

use std::collections::{HashMap, HashSet};

use dbcraft_core::{ColumnInfo, Error, ForeignKeyInfo, IndexInfo, Result, TableSchema};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use serde::{Deserialize, Serialize};

use crate::ddl;
use crate::transfer::qualify;

/// What Save will run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub statements: Vec<String>,
    pub warnings: Vec<String>,
    /// SQLite table rebuild: run on one connection with foreign keys off.
    pub rebuild: bool,
}

/// Where the table lives.
#[derive(Debug, Clone, Copy)]
pub struct Place<'a> {
    pub dialect: Dialect,
    pub database: Option<&'a str>,
    pub schema: Option<&'a str>,
}

impl Place<'_> {
    fn table(&self, name: &str) -> String {
        qualify(self.dialect, self.database, self.schema, name)
    }

    fn ref_name(&self, f: &ForeignKeyInfo) -> String {
        qualify(self.dialect, self.database, f.ref_schema.as_deref().or(self.schema), &f.ref_table)
    }

    /// A schema-level object (PostgreSQL index) or database-level one (MySQL trigger).
    fn object(&self, name: &str) -> String {
        match self.dialect {
            Dialect::Postgres => qualify(self.dialect, None, self.schema, name),
            Dialect::Mysql => qualify(self.dialect, self.database, None, name),
            _ => quote_ident(self.dialect, name),
        }
    }
}

/// Reject definitions no engine would accept, with a message the user can act on.
pub fn validate(t: &TableSchema) -> Result<()> {
    if t.name.trim().is_empty() {
        return Err(Error::InvalidParam("the table needs a name".into()));
    }
    if t.columns.is_empty() {
        return Err(Error::InvalidParam("a table needs at least one field".into()));
    }
    let mut seen = HashSet::new();
    for c in &t.columns {
        if c.name.trim().is_empty() {
            return Err(Error::InvalidParam("every field needs a name".into()));
        }
        if c.native_type.trim().is_empty() {
            return Err(Error::InvalidParam(format!("field `{}` needs a type", c.name)));
        }
        if !seen.insert(c.name.to_ascii_lowercase()) {
            return Err(Error::InvalidParam(format!("field `{}` appears twice", c.name)));
        }
    }
    let cols: HashSet<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
    let check_cols = |what: &str, name: &str, list: &[String], allow_empty: bool| -> Result<()> {
        if list.is_empty() && !allow_empty {
            return Err(Error::InvalidParam(format!("{what} `{name}` has no fields")));
        }
        if let Some(missing) = list.iter().find(|c| !cols.contains(c.as_str())) {
            return Err(Error::InvalidParam(format!("{what} `{name}` uses `{missing}`, which is not a field of the table")));
        }
        Ok(())
    };
    let mut names = HashSet::new();
    for i in &t.indexes {
        if i.name.trim().is_empty() {
            return Err(Error::InvalidParam("every index needs a name".into()));
        }
        check_cols("index", &i.name, &i.columns, i.native_ddl.is_some())?;
        names.insert(i.name.to_ascii_lowercase());
    }
    for u in &t.uniques {
        if u.name.trim().is_empty() {
            return Err(Error::InvalidParam("every unique needs a name".into()));
        }
        check_cols("unique", &u.name, &u.columns, false)?;
    }
    for f in &t.foreign_keys {
        if f.name.trim().is_empty() || f.ref_table.trim().is_empty() {
            return Err(Error::InvalidParam("every foreign key needs a name and a referenced table".into()));
        }
        check_cols("foreign key", &f.name, &f.columns, false)?;
        if f.columns.len() != f.ref_columns.len() {
            return Err(Error::InvalidParam(format!("foreign key `{}`: {} field(s) but {} referenced field(s)", f.name, f.columns.len(), f.ref_columns.len())));
        }
    }
    for c in &t.checks {
        if c.name.trim().is_empty() || c.expression.trim().is_empty() {
            return Err(Error::InvalidParam("every check needs a name and an expression".into()));
        }
    }
    for tr in &t.triggers {
        if tr.name.trim().is_empty() || tr.body.trim().is_empty() || tr.events.is_empty() {
            return Err(Error::InvalidParam("every trigger needs a name, at least one event and a definition".into()));
        }
    }
    Ok(())
}

/// A new table: `CREATE TABLE` and its indexes, foreign keys, comments and triggers.
pub fn create_plan(t: &TableSchema, at: Place) -> Result<Plan> {
    validate(t)?;
    let d = at.dialect;
    let name = at.table(&t.name);
    let mut p = Plan::default();
    p.statements.push(ddl::create_table(t, &name, d, d, &|f| at.ref_name(f), &mut p.warnings));
    p.statements.extend(ddl::comments(t, &name, d));
    for i in &t.indexes {
        p.statements.push(match &i.native_ddl {
            Some(native) if i.columns.is_empty() => native.clone(),
            _ => ddl::index_sql(i, &name, d, d),
        });
    }
    p.statements.extend(ddl::index_comments(t, at.schema, d));
    p.statements.extend(ddl::add_foreign_keys(t, &name, d, &|f| at.ref_name(f)));
    p.statements.extend(ddl::create_triggers(t, &name, d, d, &mut p.warnings));
    Ok(p)
}

/// Everything about a column except its name.
fn column_changed(a: &ColumnInfo, b: &ColumnInfo) -> bool {
    a.native_type.trim() != b.native_type.trim()
        || a.nullable != b.nullable
        || a.default != b.default
        || a.auto_increment != b.auto_increment
        || a.comment != b.comment
        || a.collation != b.collation
        || a.on_update != b.on_update
}

fn pk_cols(t: &TableSchema) -> Vec<String> {
    t.primary_key().iter().map(|c| c.name.clone()).collect()
}

/// Items of `old` that are gone or changed in `new` (to drop), and of `new` that are new or
/// changed (to create).
fn diff<'a, T: PartialEq>(old: &'a [T], new: &'a [T]) -> (Vec<&'a T>, Vec<&'a T>) {
    let drop = old.iter().filter(|o| !new.iter().any(|n| n == *o)).collect();
    let add = new.iter().filter(|n| !old.iter().any(|o| o == *n)).collect();
    (drop, add)
}

/// The statements that turn `old` into `new`. `origins[i]` is the saved name of `new.columns[i]`
/// (`None`: a new field).
pub fn alter_plan(old: &TableSchema, new: &TableSchema, origins: &[Option<String>], at: Place) -> Result<Plan> {
    validate(new)?;
    if origins.len() != new.columns.len() {
        return Err(Error::InvalidParam(format!("`origins` has {} entries for {} fields", origins.len(), new.columns.len())));
    }
    let old_names: HashSet<&str> = old.columns.iter().map(|c| c.name.as_str()).collect();
    let mut used = HashSet::new();
    for o in origins.iter().flatten() {
        if !old_names.contains(o.as_str()) {
            return Err(Error::InvalidParam(format!("field `{o}` is not in the saved table")));
        }
        if !used.insert(o.as_str()) {
            return Err(Error::InvalidParam(format!("field `{o}` is used twice")));
        }
    }
    match at.dialect {
        Dialect::Postgres => Ok(postgres(old, new, origins, at)),
        Dialect::Mysql => Ok(mysql(old, new, origins, at)),
        Dialect::Sqlite => Ok(sqlite(old, new, origins, at)),
        Dialect::Mssql => Err(Error::Unsupported("SQL Server tables can't be designed yet".into())),
    }
}

fn old_col<'a>(old: &'a TableSchema, origin: &Option<String>) -> Option<&'a ColumnInfo> {
    origin.as_ref().and_then(|o| old.columns.iter().find(|c| &c.name == o))
}

/// Whether kept fields stay in their saved order and new ones are only appended.
fn order_kept(old: &TableSchema, origins: &[Option<String>]) -> bool {
    let positions: Vec<Option<usize>> = origins.iter().map(|o| o.as_ref().and_then(|o| old.columns.iter().position(|c| &c.name == o))).collect();
    let kept: Vec<usize> = positions.iter().flatten().copied().collect();
    let sorted = kept.windows(2).all(|w| w.first() < w.get(1));
    let first_new = positions.iter().position(Option::is_none).unwrap_or(positions.len());
    let appended = positions.iter().skip(first_new).all(Option::is_none);
    sorted && appended
}

fn postgres(old: &TableSchema, new: &TableSchema, origins: &[Option<String>], at: Place) -> Plan {
    let d = Dialect::Postgres;
    let mut p = Plan::default();
    let s = &mut p.statements;
    let mut cur = at.table(&old.name);
    if new.name != old.name {
        s.push(format!("ALTER TABLE {cur} RENAME TO {}", quote_ident(d, &new.name)));
        cur = at.table(&new.name);
    }
    let (drop_trg, add_trg) = diff(&old.triggers, &new.triggers);
    for t in drop_trg {
        s.push(format!("DROP TRIGGER {} ON {cur}", quote_ident(d, &t.name)));
    }
    let (drop_fk, add_fk) = diff(&old.foreign_keys, &new.foreign_keys);
    let (drop_uq, add_uq) = diff(&old.uniques, &new.uniques);
    let (drop_ck, add_ck) = diff(&old.checks, &new.checks);
    for name in drop_fk.iter().map(|f| &f.name).chain(drop_uq.iter().map(|u| &u.name)).chain(drop_ck.iter().map(|c| &c.name)) {
        s.push(format!("ALTER TABLE {cur} DROP CONSTRAINT {}", quote_ident(d, name)));
    }
    let (drop_idx, add_idx) = diff(&old.indexes, &new.indexes);
    for i in drop_idx {
        s.push(format!("DROP INDEX {}", at.object(&i.name)));
    }
    let pk_changed = pk_cols_mapped(old, new, origins) != pk_cols(new);
    if pk_changed && !old.primary_key().is_empty() {
        let name = old.primary_key_name.clone().unwrap_or_else(|| format!("{}_pkey", old.name));
        s.push(format!("ALTER TABLE {cur} DROP CONSTRAINT {}", quote_ident(d, &name)));
    }
    // Columns: renames, drops, changes, additions.
    let kept: HashSet<&str> = origins.iter().flatten().map(String::as_str).collect();
    for (c, o) in new.columns.iter().zip(origins) {
        if let Some(o) = o
            && o != &c.name
        {
            s.push(format!("ALTER TABLE {cur} RENAME COLUMN {} TO {}", quote_ident(d, o), quote_ident(d, &c.name)));
        }
    }
    for c in old.columns.iter().filter(|c| !kept.contains(c.name.as_str())) {
        s.push(format!("ALTER TABLE {cur} DROP COLUMN {}", quote_ident(d, &c.name)));
    }
    let new_pk: HashSet<String> = pk_cols(new).into_iter().collect();
    for (c, o) in new.columns.iter().zip(origins) {
        let col = quote_ident(d, &c.name);
        let Some(was) = old_col(old, o) else {
            s.push(format!("ALTER TABLE {cur} ADD COLUMN {}", ddl::column_def(c, new_pk.contains(&c.name), false, new_pk.len() == 1, d, d, &mut p.warnings)));
            if let Some(cm) = &c.comment {
                s.push(format!("COMMENT ON COLUMN {cur}.{col} IS {}", quote_literal(d, cm)));
            }
            continue;
        };
        if was.native_type.trim() != c.native_type.trim() || was.collation != c.collation {
            let coll = c.collation.as_ref().map(|x| format!(" COLLATE {}", quote_ident(d, x))).unwrap_or_default();
            s.push(format!("ALTER TABLE {cur} ALTER COLUMN {col} TYPE {}{coll} USING {col}::{}", c.native_type, c.native_type));
        }
        if was.auto_increment != c.auto_increment {
            if c.auto_increment {
                s.push(format!("ALTER TABLE {cur} ALTER COLUMN {col} ADD GENERATED BY DEFAULT AS IDENTITY"));
            } else {
                s.push(format!("ALTER TABLE {cur} ALTER COLUMN {col} DROP IDENTITY IF EXISTS"));
            }
        }
        if was.nullable != c.nullable && !(c.nullable && new_pk.contains(&c.name)) {
            s.push(format!("ALTER TABLE {cur} ALTER COLUMN {col} {} NOT NULL", if c.nullable { "DROP" } else { "SET" }));
        }
        if was.default != c.default && !c.auto_increment {
            match &c.default {
                Some(v) => s.push(format!("ALTER TABLE {cur} ALTER COLUMN {col} SET DEFAULT {v}")),
                None => s.push(format!("ALTER TABLE {cur} ALTER COLUMN {col} DROP DEFAULT")),
            }
        }
        if was.comment != c.comment {
            let v = c.comment.as_ref().map_or("NULL".to_string(), |cm| quote_literal(d, cm));
            s.push(format!("COMMENT ON COLUMN {cur}.{col} IS {v}"));
        }
    }
    if !order_kept(old, origins) {
        p.warnings.push("PostgreSQL can't reorder columns: kept fields stay in their saved order and new fields are added at the end".into());
    }
    if pk_changed && !new_pk.is_empty() {
        let cols = pk_cols(new).iter().map(|c| quote_ident(d, c)).collect::<Vec<_>>().join(", ");
        match &new.primary_key_name {
            Some(n) => s.push(format!("ALTER TABLE {cur} ADD CONSTRAINT {} PRIMARY KEY ({cols})", quote_ident(d, n))),
            None => s.push(format!("ALTER TABLE {cur} ADD PRIMARY KEY ({cols})")),
        }
    }
    for u in add_uq {
        s.push(format!("ALTER TABLE {cur} ADD {}", ddl::unique_clause(u, d)));
    }
    for c in add_ck {
        s.push(format!("ALTER TABLE {cur} ADD {}", ddl::check_clause(c, d)));
    }
    for i in &add_idx {
        s.push(index_or_native(i, &cur, d));
    }
    for i in add_idx.iter().filter(|i| i.comment.is_some()) {
        s.push(format!("COMMENT ON INDEX {} IS {}", at.object(&i.name), quote_literal(d, i.comment.as_deref().unwrap_or(""))));
    }
    for f in add_fk {
        s.push(format!("ALTER TABLE {cur} ADD {}", fk(f, at)));
    }
    for t in add_trg {
        s.push(ddl::trigger_sql(t, &cur, d));
    }
    if old.comment != new.comment {
        let v = new.comment.as_ref().map_or("NULL".to_string(), |c| quote_literal(d, c));
        s.push(format!("COMMENT ON TABLE {cur} IS {v}"));
    }
    if old.options.get("tablespace") != new.options.get("tablespace")
        && let Some(ts) = new.options.get("tablespace").filter(|v| !v.is_empty())
    {
        s.push(format!("ALTER TABLE {cur} SET TABLESPACE {}", quote_ident(d, ts)));
    }
    let unlogged = |t: &TableSchema| t.options.get("unlogged").is_some_and(|v| v == "true");
    if unlogged(old) != unlogged(new) {
        s.push(format!("ALTER TABLE {cur} SET {}", if unlogged(new) { "UNLOGGED" } else { "LOGGED" }));
    }
    p
}

/// The saved primary key, written with the fields' new names.
fn pk_cols_mapped(old: &TableSchema, new: &TableSchema, origins: &[Option<String>]) -> Vec<String> {
    pk_cols(old)
        .into_iter()
        .map(|k| origins.iter().zip(&new.columns).find(|(o, _)| o.as_deref() == Some(k.as_str())).map_or(k, |(_, c)| c.name.clone()))
        .collect()
}

fn index_or_native(i: &IndexInfo, table: &str, d: Dialect) -> String {
    match &i.native_ddl {
        Some(native) if i.columns.is_empty() => native.clone(),
        _ => ddl::index_sql(i, table, d, d),
    }
}

fn fk(f: &ForeignKeyInfo, at: Place) -> String {
    let d = at.dialect;
    let cols = |v: &[String]| v.iter().map(|c| quote_ident(d, c)).collect::<Vec<_>>().join(", ");
    let mut s = format!("CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})", quote_ident(d, &f.name), cols(&f.columns), at.ref_name(f), cols(&f.ref_columns));
    if let Some(a) = &f.on_delete {
        s.push_str(&format!(" ON DELETE {a}"));
    }
    if let Some(a) = &f.on_update {
        s.push_str(&format!(" ON UPDATE {a}"));
    }
    s
}

fn mysql(old: &TableSchema, new: &TableSchema, origins: &[Option<String>], at: Place) -> Plan {
    let d = Dialect::Mysql;
    let mut p = Plan::default();
    let mut cur = at.table(&old.name);
    if new.name != old.name {
        p.statements.push(format!("ALTER TABLE {cur} RENAME TO {}", at.table(&new.name)));
        cur = at.table(&new.name);
    }
    let (drop_trg, add_trg) = diff(&old.triggers, &new.triggers);
    for t in drop_trg {
        p.statements.push(format!("DROP TRIGGER {}", at.object(&t.name)));
    }
    let (drop_fk, add_fk) = diff(&old.foreign_keys, &new.foreign_keys);
    if !drop_fk.is_empty() {
        let drops: Vec<String> = drop_fk.iter().map(|f| format!("DROP FOREIGN KEY {}", quote_ident(d, &f.name))).collect();
        p.statements.push(format!("ALTER TABLE {cur}\n  {}", drops.join(",\n  ")));
    }
    let mut clauses = Vec::new();
    let (drop_idx, add_idx) = diff(&old.indexes, &new.indexes);
    for i in &drop_idx {
        clauses.push(format!("DROP INDEX {}", quote_ident(d, &i.name)));
    }
    let pk_changed = pk_cols(old) != pk_cols(new) || origins.iter().zip(&new.columns).any(|(o, c)| o.as_ref().is_some_and(|o| o != &c.name) && c.pk.is_some());
    if pk_changed && !old.primary_key().is_empty() {
        clauses.push("DROP PRIMARY KEY".into());
    }
    let (drop_ck, add_ck) = diff(&old.checks, &new.checks);
    for c in &drop_ck {
        clauses.push(format!("DROP CONSTRAINT {}", quote_ident(d, &c.name)));
    }
    let kept: HashSet<&str> = origins.iter().flatten().map(String::as_str).collect();
    for c in old.columns.iter().filter(|c| !kept.contains(c.name.as_str())) {
        clauses.push(format!("DROP COLUMN {}", quote_ident(d, &c.name)));
    }
    // The saved field before each kept field, skipping dropped ones.
    let kept_old: Vec<&str> = old.columns.iter().map(|c| c.name.as_str()).filter(|n| kept.contains(n)).collect();
    let old_prev: HashMap<&str, Option<&str>> =
        kept_old.iter().enumerate().map(|(i, n)| (*n, i.checked_sub(1).and_then(|j| kept_old.get(j)).copied())).collect();
    let new_pk: HashSet<String> = pk_cols(new).into_iter().collect();
    for (i, (c, o)) in new.columns.iter().zip(origins).enumerate() {
        let position = match i.checked_sub(1).and_then(|j| new.columns.get(j)) {
            None => " FIRST".to_string(),
            Some(prev) => format!(" AFTER {}", quote_ident(d, &prev.name)),
        };
        let def = ddl::column_def(c, new_pk.contains(&c.name), in_key(new, &c.name), new_pk.len() == 1, d, d, &mut p.warnings);
        match old_col(old, o) {
            None => clauses.push(format!("ADD COLUMN {def}{position}")),
            Some(was) => {
                let prev_origin = i.checked_sub(1).and_then(|j| origins.get(j)).cloned().flatten();
                let moved = old_prev.get(was.name.as_str()).copied().flatten().map(str::to_string) != prev_origin;
                if was.name != c.name || column_changed(was, c) || moved {
                    clauses.push(format!("CHANGE COLUMN {} {def}{position}", quote_ident(d, &was.name)));
                }
            }
        }
    }
    if pk_changed && !new_pk.is_empty() {
        clauses.push(format!("ADD PRIMARY KEY ({})", pk_cols(new).iter().map(|c| quote_ident(d, c)).collect::<Vec<_>>().join(", ")));
    }
    for i in &add_idx {
        // `CREATE … INDEX n ON t (…)` becomes `ADD … INDEX n (…)`.
        let create = ddl::index_sql(i, &cur, d, d);
        let add = create.replacen("CREATE ", "ADD ", 1).replacen(&format!(" ON {cur}"), "", 1);
        clauses.push(add);
    }
    for c in &add_ck {
        clauses.push(format!("ADD {}", ddl::check_clause(c, d)));
    }
    let opt = |t: &TableSchema, k: &str| t.options.get(k).cloned().filter(|v| !v.is_empty());
    for (key, sql) in
        [("engine", "ENGINE"), ("charset", "DEFAULT CHARSET"), ("collation", "COLLATE"), ("row_format", "ROW_FORMAT"), ("auto_increment", "AUTO_INCREMENT")]
    {
        if opt(old, key) != opt(new, key)
            && let Some(v) = opt(new, key)
        {
            clauses.push(format!("{sql}={v}"));
        }
    }
    if old.comment != new.comment {
        clauses.push(format!("COMMENT={}", quote_literal(d, new.comment.as_deref().unwrap_or(""))));
    }
    if !clauses.is_empty() {
        p.statements.push(format!("ALTER TABLE {cur}\n  {}", clauses.join(",\n  ")));
    }
    if !add_fk.is_empty() {
        let adds: Vec<String> = add_fk.iter().map(|f| format!("ADD {}", fk(f, at))).collect();
        p.statements.push(format!("ALTER TABLE {cur}\n  {}", adds.join(",\n  ")));
    }
    for t in add_trg {
        p.statements.push(ddl::trigger_sql(t, &cur, d));
    }
    p
}

fn in_key(t: &TableSchema, col: &str) -> bool {
    let c = col.to_string();
    t.indexes.iter().any(|i| i.columns.contains(&c))
        || t.foreign_keys.iter().any(|f| f.columns.contains(&c))
        || t.uniques.iter().any(|u| u.columns.contains(&c))
}

fn sqlite(old: &TableSchema, new: &TableSchema, origins: &[Option<String>], at: Place) -> Plan {
    let d = Dialect::Sqlite;
    let kept: HashSet<&str> = origins.iter().flatten().map(String::as_str).collect();
    let dropped: Vec<&ColumnInfo> = old.columns.iter().filter(|c| !kept.contains(c.name.as_str())).collect();
    let renamed = |name: &str| -> String {
        // A saved field's new name (for comparing constraints across renames).
        origins.iter().zip(&new.columns).find(|(o, _)| o.as_deref() == Some(name)).map_or(name.to_string(), |(_, c)| c.name.clone())
    };
    let same_list = |a: &[String], b: &[String]| a.iter().map(|x| renamed(x)).collect::<Vec<_>>() == b;
    let constraints_same = same_list(&pk_cols(old), &pk_cols(new))
        && old.uniques.len() == new.uniques.len()
        && old.uniques.iter().zip(&new.uniques).all(|(a, b)| same_list(&a.columns, &b.columns))
        && old.foreign_keys.len() == new.foreign_keys.len()
        && old.foreign_keys.iter().zip(&new.foreign_keys).all(|(a, b)| {
            same_list(&a.columns, &b.columns)
                && a.ref_table == b.ref_table
                && a.ref_columns == b.ref_columns
                && a.on_delete == b.on_delete
                && a.on_update == b.on_update
        })
        && old.checks == new.checks
        && old.options == new.options;
    let columns_simple = new.columns.iter().zip(origins).all(|(c, o)| match old_col(old, o) {
        Some(was) => !column_changed(was, c),
        // ALTER TABLE ADD COLUMN: no key, no non-constant default, NOT NULL only with a default.
        None => c.pk.is_none() && !c.auto_increment && (c.nullable || c.default.is_some()) && c.default.as_deref().is_none_or(|v| !v.contains('(')),
    });
    let dropped_simple = dropped.iter().all(|c| {
        c.pk.is_none()
            && !old.uniques.iter().any(|u| u.columns.contains(&c.name))
            && !old.foreign_keys.iter().any(|f| f.columns.contains(&c.name))
            && !old.checks.iter().any(|k| k.expression.contains(&c.name))
    });
    let mut p = Plan::default();
    if constraints_same && columns_simple && dropped_simple && order_kept(old, origins) {
        let mut cur = at.table(&old.name);
        if new.name != old.name {
            p.statements.push(format!("ALTER TABLE {cur} RENAME TO {}", quote_ident(d, &new.name)));
            cur = at.table(&new.name);
        }
        let (drop_trg, add_trg) = diff(&old.triggers, &new.triggers);
        for t in drop_trg {
            p.statements.push(format!("DROP TRIGGER {}", quote_ident(d, &t.name)));
        }
        let (drop_idx, add_idx) = diff(&old.indexes, &new.indexes);
        for i in drop_idx {
            p.statements.push(format!("DROP INDEX {}", quote_ident(d, &i.name)));
        }
        for (c, o) in new.columns.iter().zip(origins) {
            if let Some(o) = o
                && o != &c.name
            {
                p.statements.push(format!("ALTER TABLE {cur} RENAME COLUMN {} TO {}", quote_ident(d, o), quote_ident(d, &c.name)));
            }
        }
        for c in &dropped {
            p.statements.push(format!("ALTER TABLE {cur} DROP COLUMN {}", quote_ident(d, &c.name)));
        }
        for (c, o) in new.columns.iter().zip(origins) {
            if o.is_none() {
                p.statements.push(format!("ALTER TABLE {cur} ADD COLUMN {}", ddl::column_def(c, false, false, false, d, d, &mut p.warnings)));
            }
        }
        for i in add_idx {
            p.statements.push(index_or_native(i, &cur, d));
        }
        for t in add_trg {
            p.statements.push(ddl::trigger_sql(t, &cur, d));
        }
        return p;
    }
    // Rebuild (https://www.sqlite.org/lang_altertable.html#otheralter).
    p.rebuild = true;
    let tmp_name = format!("_dbcraft_new_{}", new.name);
    let tmp = at.table(&tmp_name);
    let mut create = ddl::create_table(new, &tmp, d, d, &|f| at.ref_name(f), &mut p.warnings);
    // Self-references must point at the final name.
    create = create.replace(&format!("REFERENCES {tmp}"), &format!("REFERENCES {}", at.table(&new.name)));
    p.statements.push(create);
    let pairs: Vec<(&ColumnInfo, &String)> = new.columns.iter().zip(origins).filter_map(|(c, o)| o.as_ref().map(|o| (c, o))).collect();
    if !pairs.is_empty() {
        let to = pairs.iter().map(|(c, _)| quote_ident(d, &c.name)).collect::<Vec<_>>().join(", ");
        let from = pairs.iter().map(|(_, o)| quote_ident(d, o)).collect::<Vec<_>>().join(", ");
        p.statements.push(format!("INSERT INTO {tmp} ({to}) SELECT {from} FROM {}", at.table(&old.name)));
    }
    p.statements.push(format!("DROP TABLE {}", at.table(&old.name)));
    p.statements.push(format!("ALTER TABLE {tmp} RENAME TO {}", quote_ident(d, &new.name)));
    let cur = at.table(&new.name);
    for i in &new.indexes {
        p.statements.push(index_or_native(i, &cur, d));
    }
    for t in &new.triggers {
        p.statements.push(ddl::trigger_sql(t, &cur, d));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcraft_core::{CheckInfo, GenericType as G, TriggerInfo, UniqueInfo};

    fn col(name: &str, ty: &str, nullable: bool, pk: Option<u32>) -> ColumnInfo {
        ColumnInfo::new(name, ty, G::Text, nullable, pk)
    }

    fn base() -> TableSchema {
        TableSchema {
            name: "film".into(),
            columns: vec![col("id", "integer", false, Some(1)), col("title", "text", false, None), col("year", "integer", true, None)],
            indexes: vec![IndexInfo { name: "film_title".into(), columns: vec!["title".into()], ..IndexInfo::default() }],
            ..TableSchema::default()
        }
    }

    fn origins(t: &TableSchema) -> Vec<Option<String>> {
        t.columns.iter().map(|c| Some(c.name.clone())).collect()
    }

    fn pg() -> Place<'static> {
        Place { dialect: Dialect::Postgres, database: None, schema: Some("public") }
    }

    #[test]
    fn nothing_changed_means_nothing_to_run() {
        for at in
            [pg(), Place { dialect: Dialect::Mysql, database: Some("db"), schema: None }, Place { dialect: Dialect::Sqlite, database: None, schema: None }]
        {
            let p = alter_plan(&base(), &base(), &origins(&base()), at).unwrap();
            assert!(p.statements.is_empty(), "{:?}: {:?}", at.dialect, p.statements);
        }
    }

    #[test]
    fn postgres_rename_retype_add_drop() {
        let old = base();
        let mut new = base();
        new.columns[1].name = "name".into();
        new.columns[2].native_type = "bigint".into();
        new.columns[2].nullable = false;
        new.columns[2].default = Some("0".into());
        new.columns.push(col("note", "text", true, None));
        new.indexes[0].columns = vec!["name".into()];
        new.checks.push(CheckInfo { name: "year_ok".into(), expression: "year > 1880".into() });
        new.comment = Some("Films".into());
        let mut o = origins(&old);
        o.push(None);
        let p = alter_plan(&old, &new, &o, pg()).unwrap();
        assert_eq!(
            p.statements,
            [
                "DROP INDEX \"public\".\"film_title\"",
                "ALTER TABLE \"public\".\"film\" RENAME COLUMN \"title\" TO \"name\"",
                "ALTER TABLE \"public\".\"film\" ALTER COLUMN \"year\" TYPE bigint USING \"year\"::bigint",
                "ALTER TABLE \"public\".\"film\" ALTER COLUMN \"year\" SET NOT NULL",
                "ALTER TABLE \"public\".\"film\" ALTER COLUMN \"year\" SET DEFAULT 0",
                "ALTER TABLE \"public\".\"film\" ADD COLUMN \"note\" text",
                "ALTER TABLE \"public\".\"film\" ADD CONSTRAINT \"year_ok\" CHECK (year > 1880)",
                "CREATE INDEX \"film_title\" ON \"public\".\"film\" (\"name\")",
                "COMMENT ON TABLE \"public\".\"film\" IS 'Films'",
            ]
        );
        assert!(!p.rebuild);
    }

    #[test]
    fn mysql_one_alter_with_positions() {
        let at = Place { dialect: Dialect::Mysql, database: Some("db"), schema: None };
        let old = base();
        let mut new = base();
        // Move `year` before `title`, drop nothing, add a field first, change the table engine.
        new.columns.swap(1, 2);
        new.columns.insert(0, col("code", "varchar(10)", true, None));
        new.options.insert("engine".into(), "InnoDB".into());
        let o = vec![None, Some("id".into()), Some("year".into()), Some("title".into())];
        let p = alter_plan(&old, &new, &o, at).unwrap();
        assert_eq!(
            p.statements,
            [
                "ALTER TABLE `db`.`film`\n  ADD COLUMN `code` varchar(10) FIRST,\n  CHANGE COLUMN `year` `year` integer AFTER `id`,\n  CHANGE COLUMN `title` `title` text NOT NULL AFTER `year`,\n  ENGINE=InnoDB"
            ]
        );
    }

    #[test]
    fn mysql_keys_and_triggers() {
        let at = Place { dialect: Dialect::Mysql, database: Some("db"), schema: None };
        let old = base();
        let mut new = base();
        new.columns[2].pk = Some(2);
        new.foreign_keys.push(ForeignKeyInfo {
            name: "fk_y".into(),
            columns: vec!["year".into()],
            ref_table: "years".into(),
            ref_columns: vec!["y".into()],
            ..ForeignKeyInfo::default()
        });
        new.triggers.push(TriggerInfo {
            name: "t1".into(),
            timing: "BEFORE".into(),
            events: vec!["INSERT".into()],
            for_each_row: true,
            condition: None,
            body: "SET NEW.title = TRIM(NEW.title)".into(),
        });
        let p = alter_plan(&old, &new, &origins(&old), at).unwrap();
        assert_eq!(p.statements.len(), 3, "{:?}", p.statements);
        assert!(p.statements[0].contains("DROP PRIMARY KEY") && p.statements[0].contains("ADD PRIMARY KEY (`id`, `year`)"), "{}", p.statements[0]);
        assert_eq!(p.statements[1], "ALTER TABLE `db`.`film`\n  ADD CONSTRAINT `fk_y` FOREIGN KEY (`year`) REFERENCES `db`.`years` (`y`)");
        assert_eq!(p.statements[2], "CREATE TRIGGER `t1` BEFORE INSERT ON `db`.`film` FOR EACH ROW\nSET NEW.title = TRIM(NEW.title)");
    }

    #[test]
    fn sqlite_simple_changes_stay_native() {
        let at = Place { dialect: Dialect::Sqlite, database: None, schema: None };
        let old = base();
        let mut new = base();
        new.name = "films".into();
        new.columns[1].name = "name".into();
        new.indexes[0].columns = vec!["name".into()];
        new.columns.remove(2);
        new.columns.push(col("note", "text", true, None));
        let p = alter_plan(&old, &new, &[Some("id".into()), Some("title".into()), None], at).unwrap();
        assert!(!p.rebuild);
        assert_eq!(
            p.statements,
            [
                "ALTER TABLE \"film\" RENAME TO \"films\"",
                "DROP INDEX \"film_title\"",
                "ALTER TABLE \"films\" RENAME COLUMN \"title\" TO \"name\"",
                "ALTER TABLE \"films\" DROP COLUMN \"year\"",
                "ALTER TABLE \"films\" ADD COLUMN \"note\" text",
                "CREATE INDEX \"film_title\" ON \"films\" (\"name\")",
            ]
        );
    }

    #[test]
    fn sqlite_other_changes_rebuild() {
        let at = Place { dialect: Dialect::Sqlite, database: None, schema: None };
        let old = base();
        let mut new = base();
        new.columns[2].nullable = false;
        new.columns[2].default = Some("0".into());
        new.uniques.push(UniqueInfo { name: "uq_title".into(), columns: vec!["title".into()] });
        let p = alter_plan(&old, &new, &origins(&old), at).unwrap();
        assert!(p.rebuild);
        assert!(p.statements[0].starts_with("CREATE TABLE \"_dbcraft_new_film\""), "{}", p.statements[0]);
        assert!(p.statements[0].contains("CONSTRAINT \"uq_title\" UNIQUE (\"title\")"));
        assert_eq!(p.statements[1], "INSERT INTO \"_dbcraft_new_film\" (\"id\", \"title\", \"year\") SELECT \"id\", \"title\", \"year\" FROM \"film\"");
        assert_eq!(p.statements[2], "DROP TABLE \"film\"");
        assert_eq!(p.statements[3], "ALTER TABLE \"_dbcraft_new_film\" RENAME TO \"film\"");
        assert_eq!(p.statements[4], "CREATE INDEX \"film_title\" ON \"film\" (\"title\")");
    }

    #[test]
    fn bad_definitions_are_refused() {
        let at = pg();
        let mut t = base();
        t.columns[1].name = "id".into();
        assert!(create_plan(&t, at).is_err(), "duplicate field");
        let mut t = base();
        t.indexes[0].columns = vec!["nope".into()];
        assert!(create_plan(&t, at).is_err(), "index on a missing field");
        let mut t = base();
        t.columns[0].native_type = " ".into();
        assert!(create_plan(&t, at).is_err(), "no type");
        assert!(alter_plan(&base(), &base(), &[None], at).is_err(), "origins length");
        assert!(alter_plan(&base(), &base(), &[Some("x".into()), None, None], at).is_err(), "unknown origin");
        let mut t = base();
        t.name = String::new();
        assert!(create_plan(&t, at).is_err());
    }

    #[test]
    fn create_plan_has_everything() {
        let mut t = base();
        t.triggers.push(TriggerInfo {
            name: "t".into(),
            timing: "AFTER".into(),
            events: vec!["INSERT".into(), "UPDATE".into()],
            for_each_row: true,
            condition: None,
            body: "EXECUTE FUNCTION log_film()".into(),
        });
        let p = create_plan(&t, pg()).unwrap();
        assert!(p.statements[0].starts_with("CREATE TABLE \"public\".\"film\""));
        assert_eq!(p.statements[1], "CREATE INDEX \"film_title\" ON \"public\".\"film\" (\"title\")");
        assert_eq!(p.statements[2], "CREATE TRIGGER \"t\" AFTER INSERT OR UPDATE ON \"public\".\"film\" FOR EACH ROW\nEXECUTE FUNCTION log_film()");
    }
}
