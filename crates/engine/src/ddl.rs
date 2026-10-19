//! Portable DDL from [`TableSchema`] (Data Transfer, Backup restore, Structure Synchronization).
//! On the same kind of engine the native types and defaults are kept exactly; across engines
//! types go through [`GenericType`](dbcraft_core::GenericType) and defaults through a small set
//! of portable forms (literals, `CURRENT_TIMESTAMP`…). What can't be carried over is reported as a
//! warning, never silently changed into something else.

use std::collections::{HashMap, HashSet};

use dbcraft_core::{ColumnInfo, ForeignKeyInfo, GenericType as G, IndexInfo, TableSchema, ViewInfo};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};

use crate::transfer::render;

/// A default value in a form every engine understands.
#[derive(Debug, Clone, PartialEq)]
enum Default {
    Num(String),
    Str(String),
    Bool(bool),
    CurrentTimestamp,
    CurrentDate,
    CurrentTime,
    /// Engine-specific expression: only valid on the same kind of engine.
    Raw(String),
}

/// Remove `::type` casts that sit outside string literals (`'a'::character varying`).
fn strip_pg_casts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_str = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            in_str = !in_str;
            out.push(c);
        } else if !in_str && c == ':' && chars.peek() == Some(&':') {
            chars.next();
            // Skip the type name (words, brackets, its own `(n)`); a `)` it didn't open ends it.
            let mut depth = 0u32;
            while let Some(&n) = chars.peek() {
                match n {
                    '(' => depth += 1,
                    ')' if depth == 0 => break,
                    ')' => depth -= 1,
                    _ if n.is_alphanumeric() || n == '_' || n == ' ' || n == '[' || n == ']' || n == '"' => {}
                    _ => break,
                }
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// The text of a single quoted literal, or `None` when `s` is something else.
fn unquote(s: &str, src: Dialect) -> Option<String> {
    let inner = s.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
            }
            '\'' => return None, // two literals, not one
            '\\' if src == Dialect::Mysql => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('0') => {}
                Some(o) => out.push(o),
                None => return None,
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

fn classify(raw: &str, src: Dialect) -> Option<Default> {
    let mut s = raw.trim().to_string();
    if src == Dialect::Postgres {
        s = strip_pg_casts(&s);
    }
    // `(0)`, `('x')`: parentheses around a plain value.
    while s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
        let inner = s.get(1..s.len() - 1).unwrap_or("").trim().to_string();
        if inner.contains('(') != inner.contains(')') {
            break;
        }
        s = inner;
    }
    let upper = s.to_ascii_uppercase();
    let fn_name = upper.split('(').next().unwrap_or("").trim().to_string();
    Some(match upper.as_str() {
        "NULL" => return None,
        "TRUE" => Default::Bool(true),
        "FALSE" => Default::Bool(false),
        _ if matches!(fn_name.as_str(), "CURRENT_TIMESTAMP" | "NOW" | "LOCALTIMESTAMP" | "LOCALTIME" | "TRANSACTION_TIMESTAMP")
            && (upper == fn_name || upper.ends_with(')')) =>
        {
            Default::CurrentTimestamp
        }
        _ if matches!(fn_name.as_str(), "CURRENT_DATE" | "CURDATE") => Default::CurrentDate,
        _ if matches!(fn_name.as_str(), "CURRENT_TIME" | "CURTIME") => Default::CurrentTime,
        _ if s.parse::<f64>().is_ok() && !s.is_empty() => Default::Num(s),
        _ => match unquote(&s, src) {
            Some(text) => Default::Str(text),
            None => Default::Raw(raw.trim().to_string()),
        },
    })
}

/// A column default rendered for `dst`, or `None` (with a warning) when it can't be carried over.
pub fn default_sql(c: &ColumnInfo, src: Dialect, dst: Dialect, warnings: &mut Vec<String>) -> Option<String> {
    let raw = c.default.as_deref()?;
    if src == dst {
        return Some(raw.to_string());
    }
    let numeric = matches!(c.generic, G::SmallInt | G::Integer | G::BigInt | G::Decimal { .. } | G::Float | G::Double);
    let boolean = |b: bool| match dst {
        Dialect::Postgres => if b { "TRUE" } else { "FALSE" }.to_string(),
        _ => if b { "1" } else { "0" }.to_string(),
    };
    Some(match classify(raw, src)? {
        Default::Num(n) if c.generic == G::Bool => boolean(n != "0"),
        Default::Num(n) if numeric || c.generic == G::Bool => n,
        Default::Num(n) => quote_literal(dst, &n),
        Default::Str(s) if c.generic == G::Bool => boolean(matches!(s.to_ascii_lowercase().as_str(), "1" | "t" | "true" | "y" | "yes" | "on")),
        Default::Str(s) if numeric && s.trim().parse::<f64>().is_ok() => s.trim().to_string(),
        Default::Str(s) => quote_literal(dst, &s),
        Default::Bool(b) => boolean(b),
        Default::CurrentTimestamp => "CURRENT_TIMESTAMP".into(),
        Default::CurrentDate if dst == Dialect::Mysql => "(CURRENT_DATE)".into(),
        Default::CurrentDate => "CURRENT_DATE".into(),
        Default::CurrentTime if dst == Dialect::Mysql => "(CURRENT_TIME)".into(),
        Default::CurrentTime => "CURRENT_TIME".into(),
        Default::Raw(expr) => {
            warnings.push(format!("column `{}`: default `{expr}` is specific to the source engine and was not carried over", c.name));
            return None;
        }
    })
}

/// Text types can't carry a literal default on MySQL (only an expression).
fn mysql_text(c: &ColumnInfo, same: bool) -> bool {
    let t = c.native_type.to_ascii_lowercase();
    if same { t.contains("text") || t.contains("blob") || t == "json" } else { matches!(c.generic, G::Text | G::Blob | G::Json) }
}

/// Column type on `dst`: the native type on the same engine, the generic mapping otherwise.
fn column_type(c: &ColumnInfo, src: Dialect, dst: Dialect, in_key: bool) -> String {
    if src == dst && !c.native_type.is_empty() { c.native_type.clone() } else { render(dst, c.generic, in_key) }
}

/// A column definition (`name type [IDENTITY] [NOT NULL] [DEFAULT …] [COLLATE …] …`) for
/// `CREATE TABLE`, `ADD COLUMN` and MySQL `CHANGE COLUMN`. `in_pk`: part of the primary key;
/// `in_key`: part of any key (MySQL can't index unbounded text).
pub fn column_def(c: &ColumnInfo, in_pk: bool, in_key: bool, single_pk: bool, src: Dialect, dst: Dialect, warnings: &mut Vec<String>) -> String {
    let same = src == dst;
    let mut line = format!("{} {}", quote_ident(dst, &c.name), column_type(c, src, dst, in_key || in_pk));
    if dst == Dialect::Mysql
        && same
        && let Some(coll) = &c.collation
    {
        line.push_str(&format!(" COLLATE {coll}"));
    }
    if (dst == Dialect::Postgres || dst == Dialect::Sqlite)
        && same
        && let Some(coll) = &c.collation
    {
        line.push_str(&format!(" COLLATE {}", quote_ident(dst, coll)));
    }
    if c.auto_increment && (src != Dialect::Sqlite || single_pk) {
        match dst {
            Dialect::Postgres => line.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
            Dialect::Mysql => line.push_str(" NOT NULL AUTO_INCREMENT"),
            _ => {}
        }
    }
    if (!c.nullable || in_pk) && !line.ends_with("AUTO_INCREMENT") {
        line.push_str(" NOT NULL");
    }
    if !c.auto_increment
        && let Some(mut d) = default_sql(c, src, dst, warnings)
    {
        // MySQL: `DATETIME(6) DEFAULT CURRENT_TIMESTAMP` must name the same precision.
        if dst == Dialect::Mysql
            && d == "CURRENT_TIMESTAMP"
            && c.generic == G::DateTime
            && let Some(p) = line.rsplit_once('(').and_then(|(_, rest)| rest.split_once(')')).map(|(n, _)| n.to_string())
            && p.chars().all(|ch| ch.is_ascii_digit())
            && !p.is_empty()
        {
            d = format!("CURRENT_TIMESTAMP({p})");
        }
        // MySQL text columns and SQLite expressions take a parenthesised default.
        let wrap = (dst == Dialect::Mysql && mysql_text(c, same) && !d.starts_with('(')) || (dst == Dialect::Sqlite && !is_plain_literal(&d));
        if wrap {
            line.push_str(&format!(" DEFAULT ({d})"));
        } else {
            line.push_str(&format!(" DEFAULT {d}"));
        }
    }
    if dst == Dialect::Mysql
        && let Some(u) = &c.on_update
    {
        line.push_str(&format!(" ON UPDATE {u}"));
    }
    if dst == Dialect::Mysql
        && let Some(comment) = &c.comment
    {
        line.push_str(&format!(" COMMENT {}", quote_literal(dst, comment)));
    }
    line
}

/// Whether a column is part of any key of `t`.
fn in_any_key(t: &TableSchema, col: &str) -> bool {
    let c = col.to_string();
    t.indexes.iter().any(|i| i.columns.contains(&c))
        || t.foreign_keys.iter().any(|f| f.columns.contains(&c))
        || t.uniques.iter().any(|u| u.columns.contains(&c))
}

/// SQLite numbers a lone `INTEGER PRIMARY KEY` itself; it must be declared inline.
fn sqlite_rowid(t: &TableSchema, dst: Dialect) -> bool {
    let pk = t.primary_key();
    dst == Dialect::Sqlite
        && pk.len() == 1
        && pk
            .first()
            .is_some_and(|c| c.auto_increment || matches!(c.generic, G::Integer | G::BigInt | G::SmallInt) && c.native_type.eq_ignore_ascii_case("integer"))
}

/// `CONSTRAINT name UNIQUE (cols)`.
pub fn unique_clause(u: &dbcraft_core::UniqueInfo, dst: Dialect) -> String {
    format!("CONSTRAINT {} UNIQUE ({})", quote_ident(dst, &u.name), u.columns.iter().map(|c| quote_ident(dst, c)).collect::<Vec<_>>().join(", "))
}

/// `CONSTRAINT name CHECK (expr)`.
pub fn check_clause(c: &dbcraft_core::CheckInfo, dst: Dialect) -> String {
    format!("CONSTRAINT {} CHECK ({})", quote_ident(dst, &c.name), c.expression)
}

/// `CREATE TABLE` with columns, defaults, auto-increment, primary key, uniques, checks, options
/// and (SQLite only, where they can't be added later) foreign keys. `name` is the already-quoted
/// target name; `ref_name` quotes a referenced table for the target.
pub fn create_table(
    t: &TableSchema,
    name: &str,
    src: Dialect,
    dst: Dialect,
    ref_name: &dyn Fn(&ForeignKeyInfo) -> String,
    warnings: &mut Vec<String>,
) -> String {
    let same = src == dst;
    let pk = t.primary_key();
    let pk_names: HashSet<&str> = pk.iter().map(|c| c.name.as_str()).collect();
    let rowid = sqlite_rowid(t, dst);
    let mut lines = Vec::new();
    for c in &t.columns {
        let in_pk = pk_names.contains(c.name.as_str());
        if rowid && in_pk {
            lines.push(format!("  {} INTEGER PRIMARY KEY", quote_ident(dst, &c.name)));
            continue;
        }
        lines.push(format!("  {}", column_def(c, in_pk, in_any_key(t, &c.name), pk.len() == 1, src, dst, warnings)));
    }
    if !pk.is_empty() && !rowid {
        let cols = pk.iter().map(|c| quote_ident(dst, &c.name)).collect::<Vec<_>>().join(", ");
        match (&t.primary_key_name, dst) {
            (Some(n), Dialect::Postgres) if same => lines.push(format!("  CONSTRAINT {} PRIMARY KEY ({cols})", quote_ident(dst, n))),
            _ => lines.push(format!("  PRIMARY KEY ({cols})")),
        }
    }
    for u in &t.uniques {
        lines.push(format!("  {}", unique_clause(u, dst)));
    }
    for c in &t.checks {
        if same {
            lines.push(format!("  {}", check_clause(c, dst)));
        } else {
            warnings.push(format!("check `{}` ({}) is written in the source engine's SQL and was not carried over", c.name, c.expression));
        }
    }
    if dst == Dialect::Sqlite {
        for f in &t.foreign_keys {
            lines.push(format!("  {}", fk_clause(f, ref_name(f), dst)));
        }
    }
    let opt = |k: &str| t.options.get(k).filter(|v| !v.is_empty() && same);
    let unlogged = if dst == Dialect::Postgres && opt("unlogged").is_some_and(|v| v == "true") { "UNLOGGED " } else { "" };
    let mut sql = format!("CREATE {unlogged}TABLE {name} (\n{}\n)", lines.join(",\n"));
    match dst {
        Dialect::Mysql => {
            if let Some(e) = opt("engine") {
                sql.push_str(&format!(" ENGINE={e}"));
            }
            if let Some(cs) = opt("charset") {
                sql.push_str(&format!(" DEFAULT CHARSET={cs}"));
            }
            if let Some(co) = opt("collation") {
                sql.push_str(&format!(" COLLATE={co}"));
            }
            if let Some(ai) = opt("auto_increment").filter(|v| v.parse::<u64>().is_ok()) {
                sql.push_str(&format!(" AUTO_INCREMENT={ai}"));
            }
            if let Some(rf) = opt("row_format") {
                sql.push_str(&format!(" ROW_FORMAT={rf}"));
            }
            if let Some(c) = &t.comment {
                sql.push_str(&format!(" COMMENT={}", quote_literal(dst, c)));
            }
        }
        Dialect::Postgres => {
            if let Some(ts) = opt("tablespace") {
                sql.push_str(&format!(" TABLESPACE {}", quote_ident(dst, ts)));
            }
        }
        Dialect::Sqlite => {
            let mut tail = Vec::new();
            if opt("without_rowid").is_some_and(|v| v == "true") {
                tail.push("WITHOUT ROWID");
            }
            if opt("strict").is_some_and(|v| v == "true") {
                tail.push("STRICT");
            }
            if !tail.is_empty() {
                sql.push(' ');
                sql.push_str(&tail.join(", "));
            }
        }
        Dialect::Mssql => {}
    }
    sql
}

fn is_plain_literal(d: &str) -> bool {
    d.parse::<f64>().is_ok()
        || (d.starts_with('\'') && d.ends_with('\''))
        || matches!(d, "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "CURRENT_TIME" | "TRUE" | "FALSE" | "NULL")
}

fn fk_clause(f: &ForeignKeyInfo, ref_table: String, dst: Dialect) -> String {
    let cols = |v: &[String]| v.iter().map(|c| quote_ident(dst, c)).collect::<Vec<_>>().join(", ");
    let mut s = format!("CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {ref_table} ({})", quote_ident(dst, &f.name), cols(&f.columns), cols(&f.ref_columns));
    if let Some(a) = &f.on_delete {
        s.push_str(&format!(" ON DELETE {a}"));
    }
    if let Some(a) = &f.on_update {
        s.push_str(&format!(" ON UPDATE {a}"));
    }
    s
}

/// Comments that can't go inline (PostgreSQL `COMMENT ON`).
pub fn comments(t: &TableSchema, name: &str, dst: Dialect) -> Vec<String> {
    if dst != Dialect::Postgres {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(c) = &t.comment {
        out.push(format!("COMMENT ON TABLE {name} IS {}", quote_literal(dst, c)));
    }
    for col in &t.columns {
        if let Some(c) = &col.comment {
            out.push(format!("COMMENT ON COLUMN {name}.{} IS {}", quote_ident(dst, &col.name), quote_literal(dst, c)));
        }
    }
    out
}

/// `CREATE [UNIQUE] INDEX` statements (after the data is loaded: faster).
pub fn create_indexes(t: &TableSchema, name: &str, src: Dialect, dst: Dialect, warnings: &mut Vec<String>) -> Vec<String> {
    let mut used = HashSet::new();
    t.indexes
        .iter()
        .filter_map(|i: &IndexInfo| {
            if i.columns.is_empty() {
                return match (&i.native_ddl, src == dst) {
                    (Some(ddl), true) => Some(ddl.clone()),
                    _ => {
                        warnings.push(format!("index `{}` uses expressions or a condition and can't be recreated on another engine", i.name));
                        None
                    }
                };
            }
            // PostgreSQL and SQLite index names are per schema: keep them unique.
            let mut idx_name = i.name.clone();
            let mut n = 2;
            while !used.insert(idx_name.clone()) {
                idx_name = format!("{}_{n}", i.name);
                n += 1;
            }
            Some(index_sql(&IndexInfo { name: idx_name, ..i.clone() }, name, src, dst))
        })
        .collect()
}

/// One `CREATE [UNIQUE|FULLTEXT|SPATIAL] INDEX` (method and comment kept on the same engine).
pub fn index_sql(i: &IndexInfo, table: &str, src: Dialect, dst: Dialect) -> String {
    let same = src == dst;
    let cols = i.columns.iter().map(|c| quote_ident(dst, c)).collect::<Vec<_>>().join(", ");
    let kind = match (&i.kind, dst) {
        (Some(k), Dialect::Mysql) if same => format!("{k} "),
        _ if i.unique => "UNIQUE ".to_string(),
        _ => String::new(),
    };
    let name = quote_ident(dst, &i.name);
    let method = i.method.as_ref().filter(|_| same);
    let mut sql = match (dst, method) {
        (Dialect::Postgres, Some(m)) => format!("CREATE {kind}INDEX {name} ON {table} USING {} ({cols})", m.to_ascii_lowercase()),
        (Dialect::Mysql, Some(m)) if i.kind.is_none() => format!("CREATE {kind}INDEX {name} ON {table} ({cols}) USING {m}"),
        _ => format!("CREATE {kind}INDEX {name} ON {table} ({cols})"),
    };
    if dst == Dialect::Mysql
        && let Some(c) = &i.comment
    {
        sql.push_str(&format!(" COMMENT {}", quote_literal(dst, c)));
    }
    sql
}

/// PostgreSQL index comments (`COMMENT ON INDEX`).
pub fn index_comments(t: &TableSchema, schema: Option<&str>, dst: Dialect) -> Vec<String> {
    if dst != Dialect::Postgres {
        return Vec::new();
    }
    t.indexes
        .iter()
        .filter_map(|i| {
            let c = i.comment.as_ref()?;
            let name = match schema {
                Some(s) => format!("{}.{}", quote_ident(dst, s), quote_ident(dst, &i.name)),
                None => quote_ident(dst, &i.name),
            };
            Some(format!("COMMENT ON INDEX {name} IS {}", quote_literal(dst, c)))
        })
        .collect()
}

/// One `CREATE TRIGGER` for `table` (MySQL and SQLite: one event per trigger).
pub fn trigger_sql(t: &dbcraft_core::TriggerInfo, table: &str, dst: Dialect) -> String {
    let events = match dst {
        Dialect::Postgres => t.events.join(" OR "),
        _ => t.events.first().cloned().unwrap_or_default(),
    };
    let mut sql = format!("CREATE TRIGGER {} {} {events} ON {table}", quote_ident(dst, &t.name), t.timing);
    match (dst, t.for_each_row) {
        (Dialect::Mysql, _) | (_, true) => sql.push_str(" FOR EACH ROW"),
        (Dialect::Postgres, false) => sql.push_str(" FOR EACH STATEMENT"),
        _ => {}
    }
    if dst != Dialect::Mysql
        && let Some(c) = t.condition.as_ref().filter(|c| !c.trim().is_empty())
    {
        sql.push_str(&format!(" WHEN ({c})"));
    }
    sql.push('\n');
    sql.push_str(t.body.trim());
    sql
}

/// The table's triggers (only on the same kind of engine: their bodies are engine SQL).
pub fn create_triggers(t: &TableSchema, name: &str, src: Dialect, dst: Dialect, warnings: &mut Vec<String>) -> Vec<String> {
    if src != dst {
        if !t.triggers.is_empty() {
            warnings.push(format!("{}: {} trigger(s) are written in the source engine's SQL and were not carried over", t.name, t.triggers.len()));
        }
        return Vec::new();
    }
    t.triggers.iter().map(|tr| trigger_sql(tr, name, dst)).collect()
}

/// `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY` (not SQLite: there they are part of the table).
pub fn add_foreign_keys(t: &TableSchema, name: &str, dst: Dialect, ref_name: &dyn Fn(&ForeignKeyInfo) -> String) -> Vec<String> {
    if dst == Dialect::Sqlite {
        return Vec::new();
    }
    t.foreign_keys.iter().map(|f| format!("ALTER TABLE {name} ADD {}", fk_clause(f, ref_name(f), dst))).collect()
}

/// After loading explicit ids into PostgreSQL identity columns, move their sequences past them.
pub fn reset_sequences(t: &TableSchema, name: &str, dst: Dialect) -> Vec<String> {
    if dst != Dialect::Postgres {
        return Vec::new();
    }
    t.columns
        .iter()
        .filter(|c| c.auto_increment)
        .map(|c| {
            let col = quote_ident(dst, &c.name);
            format!(
                "SELECT setval(pg_get_serial_sequence({}, {}), COALESCE((SELECT MAX({col}) FROM {name}), 0) + 1, false)",
                quote_literal(dst, name),
                quote_literal(dst, &c.name)
            )
        })
        .collect()
}

/// `CREATE VIEW` (the definition is the source engine's SQL; across engines it may fail, which the
/// caller reports).
pub fn create_view(v: &ViewInfo, name: &str) -> String {
    format!("CREATE VIEW {name} AS\n{}", v.definition)
}

/// Order tables so that referenced tables come before the tables that point to them (cycles keep
/// their original order).
pub fn dependency_order(tables: &[TableSchema]) -> Vec<usize> {
    let index: HashMap<&str, usize> = tables.iter().enumerate().map(|(i, t)| (t.name.as_str(), i)).collect();
    let mut done = vec![false; tables.len()];
    let mut visiting = vec![false; tables.len()];
    let mut out = Vec::with_capacity(tables.len());
    fn visit(i: usize, tables: &[TableSchema], index: &HashMap<&str, usize>, done: &mut [bool], visiting: &mut [bool], out: &mut Vec<usize>) {
        if done.get(i).copied().unwrap_or(true) || visiting.get(i).copied().unwrap_or(true) {
            return;
        }
        if let Some(v) = visiting.get_mut(i) {
            *v = true;
        }
        if let Some(t) = tables.get(i) {
            for f in &t.foreign_keys {
                if let Some(&j) = index.get(f.ref_table.as_str())
                    && j != i
                {
                    visit(j, tables, index, done, visiting, out);
                }
            }
        }
        if let Some(d) = done.get_mut(i) {
            *d = true;
        }
        out.push(i);
    }
    for i in 0..tables.len() {
        visit(i, tables, &index, &mut done, &mut visiting, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbcraft_core::GenericType as G;

    fn col(name: &str, native: &str, g: G, nullable: bool, pk: Option<u32>) -> ColumnInfo {
        ColumnInfo::new(name, native, g, nullable, pk)
    }

    fn table() -> TableSchema {
        let mut id = col("id", "integer", G::Integer, false, Some(1));
        id.auto_increment = true;
        let mut title = col("title", "character varying(100)", G::Varchar { len: 100 }, false, None);
        title.default = Some("'untitled'::character varying".into());
        let mut added = col("added", "timestamp without time zone", G::DateTime, true, None);
        added.default = Some("now()".into());
        let mut odd = col("code", "text", G::Text, true, None);
        odd.default = Some("gen_random_uuid()".into());
        let mut flag = col("active", "boolean", G::Bool, false, None);
        flag.default = Some("true".into());
        TableSchema {
            schema: Some("public".into()),
            name: "film".into(),
            columns: vec![id, title, added, odd, flag, col("studio_id", "integer", G::Integer, true, None)],
            primary_key_name: Some("film_pkey".into()),
            indexes: vec![IndexInfo { name: "film_title".into(), columns: vec!["title".into()], unique: true, ..IndexInfo::default() }],
            foreign_keys: vec![ForeignKeyInfo {
                name: "film_studio_fk".into(),
                columns: vec!["studio_id".into()],
                ref_schema: None,
                ref_table: "studio".into(),
                ref_columns: vec!["id".into()],
                on_delete: Some("SET NULL".into()),
                on_update: None,
            }],
            comment: Some("Films".into()),
            ..TableSchema::default()
        }
    }

    #[test]
    fn postgres_to_mysql() {
        let mut w = Vec::new();
        let t = table();
        let sql = create_table(&t, "`film`", Dialect::Postgres, Dialect::Mysql, &|f| quote_ident(Dialect::Mysql, &f.ref_table), &mut w);
        assert_eq!(
            sql,
            "CREATE TABLE `film` (\n  `id` INT NOT NULL AUTO_INCREMENT,\n  `title` VARCHAR(100) NOT NULL DEFAULT 'untitled',\n  `added` DATETIME(6) DEFAULT CURRENT_TIMESTAMP(6),\n  `code` LONGTEXT,\n  `active` TINYINT(1) NOT NULL DEFAULT 1,\n  `studio_id` INT,\n  PRIMARY KEY (`id`)\n) COMMENT='Films'"
        );
        assert_eq!(w.len(), 1, "gen_random_uuid() is reported: {w:?}");
        assert_eq!(create_indexes(&t, "`film`", Dialect::Postgres, Dialect::Mysql, &mut w), ["CREATE UNIQUE INDEX `film_title` ON `film` (`title`)"]);
        assert_eq!(
            add_foreign_keys(&t, "`film`", Dialect::Mysql, &|f| quote_ident(Dialect::Mysql, &f.ref_table)),
            ["ALTER TABLE `film` ADD CONSTRAINT `film_studio_fk` FOREIGN KEY (`studio_id`) REFERENCES `studio` (`id`) ON DELETE SET NULL"]
        );
    }

    #[test]
    fn postgres_to_postgres_keeps_native_text() {
        let mut w = Vec::new();
        let sql = create_table(&table(), "\"film\"", Dialect::Postgres, Dialect::Postgres, &|f| f.ref_table.clone(), &mut w);
        assert!(sql.contains("\"id\" integer GENERATED BY DEFAULT AS IDENTITY NOT NULL"), "{sql}");
        assert!(sql.contains("DEFAULT 'untitled'::character varying"));
        assert!(sql.contains("\"code\" text DEFAULT gen_random_uuid()"));
        assert!(sql.contains("CONSTRAINT \"film_pkey\" PRIMARY KEY (\"id\")"));
        assert!(w.is_empty());
        assert_eq!(
            reset_sequences(&table(), "\"film\"", Dialect::Postgres),
            ["SELECT setval(pg_get_serial_sequence('\"film\"', 'id'), COALESCE((SELECT MAX(\"id\") FROM \"film\"), 0) + 1, false)"]
        );
        assert_eq!(comments(&table(), "\"film\"", Dialect::Postgres), ["COMMENT ON TABLE \"film\" IS 'Films'"]);
    }

    #[test]
    fn to_sqlite_inlines_rowid_and_foreign_keys() {
        let mut w = Vec::new();
        let sql = create_table(&table(), "\"film\"", Dialect::Postgres, Dialect::Sqlite, &|f| quote_ident(Dialect::Sqlite, &f.ref_table), &mut w);
        assert!(sql.contains("\"id\" INTEGER PRIMARY KEY,"), "{sql}");
        assert!(!sql.contains("PRIMARY KEY (\"id\")"));
        assert!(sql.contains("\"added\" DATETIME DEFAULT CURRENT_TIMESTAMP"), "{sql}");
        assert!(sql.contains("CONSTRAINT \"film_studio_fk\" FOREIGN KEY (\"studio_id\") REFERENCES \"studio\" (\"id\") ON DELETE SET NULL"));
        assert!(add_foreign_keys(&table(), "\"film\"", Dialect::Sqlite, &|f| f.ref_table.clone()).is_empty());
    }

    #[test]
    fn defaults_are_classified() {
        let c = |d: &str, g: G| {
            let mut c = col("c", "x", g, true, None);
            c.default = Some(d.into());
            c
        };
        let mut w = Vec::new();
        assert_eq!(default_sql(&c("('it''s'::text)", G::Text), Dialect::Postgres, Dialect::Sqlite, &mut w).as_deref(), Some("'it''s'"));
        assert_eq!(default_sql(&c("'0'", G::Integer), Dialect::Mysql, Dialect::Postgres, &mut w).as_deref(), Some("0"));
        assert_eq!(default_sql(&c("'a\\'b'", G::Text), Dialect::Mysql, Dialect::Postgres, &mut w).as_deref(), Some("'a''b'"));
        assert_eq!(default_sql(&c("current_timestamp()", G::DateTime), Dialect::Mysql, Dialect::Postgres, &mut w).as_deref(), Some("CURRENT_TIMESTAMP"));
        assert_eq!(default_sql(&c("CURRENT_DATE", G::Date), Dialect::Postgres, Dialect::Mysql, &mut w).as_deref(), Some("(CURRENT_DATE)"));
        assert_eq!(default_sql(&c("1", G::Bool), Dialect::Sqlite, Dialect::Postgres, &mut w).as_deref(), Some("TRUE"));
        assert_eq!(default_sql(&c("NULL", G::Text), Dialect::Mysql, Dialect::Postgres, &mut w), None);
        assert_eq!(default_sql(&c("-1.5", G::Double), Dialect::Postgres, Dialect::Mysql, &mut w).as_deref(), Some("-1.5"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(default_sql(&c("nextval('x')", G::Integer), Dialect::Postgres, Dialect::Mysql, &mut w), None);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn parents_come_first() {
        let mk = |name: &str, refs: &[&str]| TableSchema {
            name: name.into(),
            foreign_keys: refs.iter().map(|r| ForeignKeyInfo { ref_table: (*r).into(), ..ForeignKeyInfo::default() }).collect(),
            ..TableSchema::default()
        };
        let tables = vec![mk("rental", &["film", "customer"]), mk("film", &["studio"]), mk("customer", &[]), mk("studio", &[]), mk("self", &["self"])];
        let order: Vec<&str> = dependency_order(&tables).into_iter().map(|i| tables[i].name.as_str()).collect();
        let pos = |n: &str| order.iter().position(|x| *x == n).unwrap();
        assert!(pos("studio") < pos("film") && pos("film") < pos("rental") && pos("customer") < pos("rental"), "{order:?}");
        assert_eq!(order.len(), 5);
        // A cycle doesn't loop forever.
        let cyc = vec![mk("a", &["b"]), mk("b", &["a"])];
        assert_eq!(dependency_order(&cyc).len(), 2);
    }
}
