use std::time::Instant;

use dbcraft_core::{
    CheckInfo, ColumnInfo, ConnectionConfig, DbKind, Error, ForeignKeyInfo, GenericType, IndexInfo, Result, ResultSet, TableInfo, TableSchema, TriggerInfo,
    Value, ViewInfo,
};
use dbcraft_model::{DbObject, ObjectKind};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::types::{Decimal, JsonValue};
use sqlx::{AssertSqlSafe, Executor, Row};

use crate::common::{
    CONNECT_TIMEOUT, block_on, cell_str, cell_u64, columns_of, current_cancel, db_err, decode_row, on_cancel, probe, returns_rows, run_atomic, spawn,
};

/// MySQL and MariaDB (same wire protocol).
pub struct MySqlConn {
    kind: DbKind,
    pool: MySqlPool,
    /// For the side connection that sends `KILL QUERY` when the user presses Stop.
    opts: MySqlConnectOptions,
    /// MariaDB reports column defaults as SQL expressions, MySQL as bare values.
    mariadb: bool,
}

impl MySqlConn {
    pub fn open(cfg: &ConnectionConfig) -> Result<Self> {
        let mut opts = MySqlConnectOptions::new()
            .host(if cfg.host.is_empty() { "localhost" } else { &cfg.host })
            .port(cfg.port.unwrap_or(3306))
            .username(if cfg.user.is_empty() { "root" } else { &cfg.user });
        if let Some(pw) = &cfg.password {
            opts = opts.password(pw);
        }
        if !cfg.database.is_empty() {
            opts = opts.database(&cfg.database);
        }
        probe::<sqlx::MySqlConnection>(&opts)?;
        Self::with_pool(cfg.kind, opts, 4)
    }

    fn with_pool(kind: DbKind, opts: MySqlConnectOptions, size: u32) -> Result<Self> {
        let pool = block_on(MySqlPoolOptions::new().max_connections(size).acquire_timeout(CONNECT_TIMEOUT).connect_with(opts.clone()))?.map_err(db_err)?;
        let version: String = block_on(sqlx::query_scalar("select version()").fetch_one(&pool))?.map_err(db_err)?;
        let mariadb = kind == DbKind::Mariadb || version.to_ascii_lowercase().contains("mariadb");
        Ok(MySqlConn { kind, pool, opts, mariadb })
    }
}

impl crate::Connection for MySqlConn {
    fn kind(&self) -> DbKind {
        self.kind
    }

    fn dialect(&self) -> Dialect {
        Dialect::Mysql
    }

    fn server_version(&self) -> Result<String> {
        let row = block_on(sqlx::query("select version()").fetch_one(&self.pool))?.map_err(db_err)?;
        row.try_get::<String, _>(0).map_err(db_err)
    }

    fn list_databases(&self) -> Result<Vec<String>> {
        let rows = block_on(sqlx::query("select schema_name from information_schema.schemata order by 1").fetch_all(&self.pool))?.map_err(db_err)?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(db_err)).collect()
    }

    fn list_objects(&self, database: &str) -> Result<Vec<DbObject>> {
        let sql = "select case table_type when 'VIEW' then 'view' else 'table' end as k, table_name as n \
                   from information_schema.tables where table_schema = ? \
                   union all \
                   select lower(routine_type), routine_name from information_schema.routines where routine_schema = ? \
                   union all \
                   select 'trigger', trigger_name from information_schema.triggers where trigger_schema = ? \
                   union all \
                   select 'event', event_name from information_schema.events where event_schema = ? \
                   order by 1, 2";
        let q = sqlx::query(sql).bind(database).bind(database).bind(database).bind(database);
        let rows = block_on(q.fetch_all(&self.pool))?.map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let kind = match r.try_get::<String, _>(0).map_err(db_err)?.as_str() {
                "view" => ObjectKind::View,
                "function" => ObjectKind::Function,
                "procedure" => ObjectKind::Procedure,
                "trigger" => ObjectKind::Trigger,
                "event" => ObjectKind::Event,
                _ => ObjectKind::Table,
            };
            out.push(DbObject { kind, name: r.try_get(1).map_err(db_err)?, schema: Some(database.to_string()) });
        }
        Ok(out)
    }

    fn table_info(&self, database: &str) -> Result<Vec<TableInfo>> {
        // Casts keep the result types the same across MySQL and MariaDB versions.
        let sql = format!(
            "select cast(table_name as char), cast(table_type as char), cast(table_rows as unsigned), cast(data_length as unsigned), \
             cast(engine as char), cast(create_time as char), cast(update_time as char), cast(table_collation as char), cast(table_comment as char) \
             from information_schema.tables where table_schema = {} order by table_name",
            quote_literal(Dialect::Mysql, database)
        );
        let rs = self.query(None, &sql)?;
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                let view = cell_str(&rs, i, 1).is_some_and(|t| t.contains("VIEW"));
                Some(TableInfo {
                    name: cell_str(&rs, i, 0)?,
                    schema: None,
                    kind: if view { "view" } else { "table" }.into(),
                    rows: cell_u64(&rs, i, 2),
                    data_length: cell_u64(&rs, i, 3),
                    engine: cell_str(&rs, i, 4),
                    owner: None,
                    created: cell_str(&rs, i, 5),
                    modified: cell_str(&rs, i, 6),
                    collation: cell_str(&rs, i, 7),
                    // MySQL puts "VIEW" in views' comment column; Navicat shows it empty.
                    comment: cell_str(&rs, i, 8).filter(|c| !c.is_empty() && !(view && c == "VIEW")),
                })
            })
            .collect())
    }

    fn table_ddl(&self, database: &str, _schema: Option<&str>, table: &str) -> Result<String> {
        let sql = format!("show create table {}.{}", quote_ident(Dialect::Mysql, database), quote_ident(Dialect::Mysql, table));
        let rs = self.query(None, &sql)?;
        cell_str(&rs, 0, 1).map(|s| format!("{s};")).ok_or_else(|| Error::NotFound(format!("table `{table}`")))
    }

    fn execute_atomic(&self, database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
        let pool = self.pool.clone();
        let mut all = Vec::with_capacity(stmts.len() + 1);
        if let Some(db) = database {
            all.push((format!("USE {}", quote_ident(Dialect::Mysql, db)), None));
        }
        all.extend_from_slice(stmts);
        let skip = all.len() - stmts.len();
        let counts = block_on(async move { run_atomic!(pool.begin().await.map_err(db_err)?, all) })??;
        Ok(counts.into_iter().skip(skip).collect())
    }

    fn columns(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let db = database.map(|d| quote_literal(Dialect::Mysql, d)).unwrap_or_else(|| "database()".into());
        let rs = self.query(
            None,
            &format!(
                "select cast(column_name as char), cast(column_type as char), cast(is_nullable as char), cast(column_default as char), \
                 cast(extra as char), cast(column_comment as char), cast(collation_name as char) \
                 from information_schema.columns where table_schema = {db} and table_name = {} order by ordinal_position",
                quote_literal(Dialect::Mysql, table)
            ),
        )?;
        if rs.rows.is_empty() {
            return Err(Error::NotFound(format!("table `{table}`")));
        }
        let pk = self.primary_key(database, schema, table)?;
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                let name = cell_str(&rs, i, 0)?;
                let native = cell_str(&rs, i, 1).unwrap_or_default();
                let generic = crate::types::from_mysql(&native);
                let extra = cell_str(&rs, i, 4).unwrap_or_default().to_ascii_lowercase();
                let mut c = ColumnInfo::new(
                    name.clone(),
                    native,
                    generic,
                    cell_str(&rs, i, 2).as_deref() == Some("YES"),
                    pk.iter().position(|k| *k == name).and_then(|p| u32::try_from(p + 1).ok()),
                );
                c.default = column_default(cell_str(&rs, i, 3), &extra, generic, self.mariadb);
                c.auto_increment = extra.contains("auto_increment");
                c.comment = cell_str(&rs, i, 5).filter(|s| !s.is_empty());
                c.collation = cell_str(&rs, i, 6);
                // `on update CURRENT_TIMESTAMP(6)` (MariaDB: `current_timestamp()`).
                let raw_extra = cell_str(&rs, i, 4).unwrap_or_default();
                c.on_update = extra
                    .find("on update ")
                    .and_then(|p| raw_extra.get(p + 10..))
                    .map(|v| v.trim().to_ascii_uppercase().replace("CURRENT_TIMESTAMP()", "CURRENT_TIMESTAMP"));
                Some(c)
            })
            .collect())
    }

    fn table_schema(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<TableSchema> {
        let db = database.map(|d| quote_literal(Dialect::Mysql, d)).unwrap_or_else(|| "database()".into());
        let t = quote_literal(Dialect::Mysql, table);
        let columns = self.columns(database, schema, table)?;
        let idx = self.query(
            None,
            &format!(
                "select cast(index_name as char), non_unique, cast(column_name as char), cast(index_type as char), cast(index_comment as char) \
                 from information_schema.statistics \
                 where table_schema = {db} and table_name = {t} and index_name <> 'PRIMARY' order by index_name, seq_in_index"
            ),
        )?;
        let mut indexes: Vec<IndexInfo> = Vec::new();
        for i in 0..idx.rows.len() {
            let Some(name) = cell_str(&idx, i, 0) else { continue };
            if indexes.last().is_none_or(|x| x.name != name) {
                let ty = cell_str(&idx, i, 3).unwrap_or_default().to_ascii_uppercase();
                let (kind, method) = match ty.as_str() {
                    "FULLTEXT" | "SPATIAL" => (Some(ty.clone()), None),
                    "" => (None, None),
                    _ => (None, Some(ty.clone())),
                };
                indexes.push(IndexInfo {
                    name,
                    unique: cell_u64(&idx, i, 1) == Some(0),
                    kind,
                    method,
                    comment: cell_str(&idx, i, 4).filter(|c| !c.is_empty()),
                    ..IndexInfo::default()
                });
            }
            if let Some(x) = indexes.last_mut() {
                match cell_str(&idx, i, 2) {
                    Some(c) => x.columns.push(c),
                    // A functional index part (MySQL 8): not portable, so not carried over.
                    None => x.columns.clear(),
                }
            }
        }
        indexes.retain(|x| !x.columns.is_empty());
        let fk = self.query(
            None,
            &format!(
                "select cast(k.constraint_name as char), cast(k.column_name as char), cast(k.referenced_table_schema as char), \
                 cast(k.referenced_table_name as char), cast(k.referenced_column_name as char), cast(r.update_rule as char), cast(r.delete_rule as char) \
                 from information_schema.key_column_usage k join information_schema.referential_constraints r \
                 on r.constraint_schema = k.constraint_schema and r.constraint_name = k.constraint_name and r.table_name = k.table_name \
                 where k.table_schema = {db} and k.table_name = {t} and k.referenced_table_name is not null \
                 order by k.constraint_name, k.ordinal_position"
            ),
        )?;
        let own_db = match database {
            Some(d) => Some(d.to_string()),
            None => cell_str(&self.query(None, "select database()")?, 0, 0),
        };
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        for i in 0..fk.rows.len() {
            let Some(name) = cell_str(&fk, i, 0) else { continue };
            if foreign_keys.last().is_none_or(|f| f.name != name) {
                let action = |c| cell_str(&fk, i, c).filter(|a| a != "NO ACTION");
                foreign_keys.push(ForeignKeyInfo {
                    name,
                    ref_schema: cell_str(&fk, i, 2).filter(|s| Some(s) != own_db.as_ref()),
                    ref_table: cell_str(&fk, i, 3).unwrap_or_default(),
                    on_update: action(5),
                    on_delete: action(6),
                    ..ForeignKeyInfo::default()
                });
            }
            if let Some(f) = foreign_keys.last_mut() {
                f.columns.push(cell_str(&fk, i, 1).unwrap_or_default());
                f.ref_columns.push(cell_str(&fk, i, 4).unwrap_or_default());
            }
        }
        let info = self.query(
            None,
            &format!(
                "select cast(table_comment as char), cast(engine as char), cast(table_collation as char), cast(auto_increment as char), cast(row_format as char) \
                 from information_schema.tables where table_schema = {db} and table_name = {t}"
            ),
        )?;
        let mut options = std::collections::BTreeMap::new();
        let table_collation = cell_str(&info, 0, 2);
        for (key, col) in [("engine", 1), ("collation", 2), ("auto_increment", 3), ("row_format", 4)] {
            if let Some(v) = cell_str(&info, 0, col).filter(|v| !v.is_empty()) {
                options.insert(key.to_string(), v);
            }
        }
        if let Some(charset) = table_collation.as_deref().and_then(|c| c.split('_').next()) {
            options.insert("charset".to_string(), charset.to_string());
        }
        let mut columns = columns;
        for c in &mut columns {
            if c.collation == table_collation {
                c.collation = None;
            }
        }
        // CHECK constraints (MySQL 8.0.16+, MariaDB 10.2+); older servers simply have none.
        let checks = self
            .query(
                None,
                &format!(
                    "select cast(cc.constraint_name as char), cast(cc.check_clause as char) from information_schema.table_constraints tc \
                     join information_schema.check_constraints cc on cc.constraint_schema = tc.constraint_schema and cc.constraint_name = tc.constraint_name \
                     where tc.table_schema = {db} and tc.table_name = {t} and tc.constraint_type = 'CHECK' order by 1"
                ),
            )
            .map(|rs| (0..rs.rows.len()).filter_map(|i| Some(CheckInfo { name: cell_str(&rs, i, 0)?, expression: cell_str(&rs, i, 1)? })).collect())
            .unwrap_or_default();
        let trg = self.query(
            None,
            &format!(
                "select cast(trigger_name as char), cast(action_timing as char), cast(event_manipulation as char), cast(action_statement as char), \
                 cast(action_orientation as char) from information_schema.triggers where event_object_schema = {db} and event_object_table = {t} order by 1"
            ),
        )?;
        let triggers = (0..trg.rows.len())
            .filter_map(|i| {
                Some(TriggerInfo {
                    name: cell_str(&trg, i, 0)?,
                    timing: cell_str(&trg, i, 1)?,
                    events: vec![cell_str(&trg, i, 2)?],
                    for_each_row: cell_str(&trg, i, 4).is_none_or(|o| o == "ROW"),
                    condition: None,
                    body: cell_str(&trg, i, 3)?,
                })
            })
            .collect();
        Ok(TableSchema {
            schema: None,
            name: table.to_string(),
            columns,
            primary_key_name: None,
            indexes,
            foreign_keys,
            uniques: Vec::new(),
            checks,
            triggers,
            options,
            comment: cell_str(&info, 0, 0).filter(|c| !c.is_empty()),
        })
    }

    fn dedicated(&self) -> Result<Box<dyn crate::Connection>> {
        Ok(Box::new(Self::with_pool(self.kind, self.opts.clone(), 1)?))
    }

    fn views(&self, database: &str) -> Result<Vec<ViewInfo>> {
        let rs = self.query(
            None,
            &format!(
                "select cast(table_name as char), cast(view_definition as char) from information_schema.views where table_schema = {} order by table_name",
                quote_literal(Dialect::Mysql, database)
            ),
        )?;
        // MySQL qualifies every column with the database; drop it so the view works elsewhere.
        let prefix = format!("{}.", quote_ident(Dialect::Mysql, database));
        Ok((0..rs.rows.len())
            .filter_map(|i| Some(ViewInfo { schema: None, name: cell_str(&rs, i, 0)?, definition: cell_str(&rs, i, 1)?.replace(&prefix, "") }))
            .collect())
    }

    fn primary_key(&self, database: Option<&str>, _schema: Option<&str>, table: &str) -> Result<Vec<String>> {
        let schema = database.map(|d| quote_literal(Dialect::Mysql, d)).unwrap_or_else(|| "database()".into());
        let sql = format!(
            "select cast(column_name as char) from information_schema.key_column_usage \
             where table_schema = {} and table_name = {} and constraint_name = 'PRIMARY' order by ordinal_position",
            schema,
            quote_literal(Dialect::Mysql, table)
        );
        let rs = self.query(None, &sql)?;
        Ok((0..rs.rows.len()).filter_map(|i| cell_str(&rs, i, 0)).collect())
    }

    fn query(&self, database: Option<&str>, sql: &str) -> Result<ResultSet> {
        let start = Instant::now();
        let mut rs = ResultSet::default();
        let sql = sql.to_string();
        let use_db = database.map(|d| format!("USE {}", quote_ident(Dialect::Mysql, d)));
        let wants_rows = returns_rows(&sql);
        let pool = self.pool.clone();
        let cancellable = current_cancel().is_some();
        let kill_opts = self.opts.clone();
        let out = block_on(async move {
            let mut conn = pool.acquire().await?;
            // With a Stop button, remember the thread id so Stop can kill the statement server-side.
            let _kill = if cancellable {
                let id: u64 = sqlx::query_scalar("select connection_id()").fetch_one(&mut *conn).await?;
                Some(on_cancel(move || {
                    spawn(async move {
                        if let Ok(mut c) = <sqlx::MySqlConnection as sqlx::Connection>::connect_with(&kill_opts).await {
                            let _ = c.execute(sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {id}")))).await;
                            let _ = sqlx::Connection::close(c).await;
                        }
                    })
                }))
            } else {
                None
            };
            if let Some(u) = use_db {
                conn.execute(sqlx::raw_sql(AssertSqlSafe(u))).await?;
            }
            if wants_rows {
                Ok::<_, sqlx::Error>((Some(sqlx::query(AssertSqlSafe(sql.clone())).fetch_all(&mut *conn).await?), 0))
            } else {
                Ok((None, conn.execute(sqlx::raw_sql(AssertSqlSafe(sql))).await?.rows_affected()))
            }
        })?
        .map_err(db_err)?;
        match out {
            (Some(rows), _) => {
                if let Some(first) = rows.first() {
                    rs.columns = columns_of!(first);
                }
                rs.rows = rows
                    .iter()
                    .map(|row| {
                        decode_row!(row,
                            i64 => Value::Int,
                            u64 => Value::UInt,
                            f64 => Value::Float,
                            f32 => |x: f32| Value::Float(x.into()),
                            String => Value::Text,
                            Decimal => |x: Decimal| Value::Text(x.to_string()),
                            NaiveDateTime => |x: NaiveDateTime| Value::Text(x.to_string()),
                            DateTime<Utc> => |x: DateTime<Utc>| Value::Text(x.to_rfc3339()),
                            NaiveDate => |x: NaiveDate| Value::Text(x.to_string()),
                            NaiveTime => |x: NaiveTime| Value::Text(x.to_string()),
                            JsonValue => |x: JsonValue| Value::Text(x.to_string()),
                            bool => Value::Bool,
                            Vec<u8> => Value::Bytes,
                        )
                    })
                    .collect();
            }
            (None, n) => rs.rows_affected = n,
        }
        rs.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(rs)
    }
}

/// A column default as SQL. MariaDB already reports SQL (`'abc'`, `current_timestamp()`, `NULL`);
/// MySQL reports bare values and flags expressions with `DEFAULT_GENERATED`.
fn column_default(raw: Option<String>, extra: &str, generic: GenericType, mariadb: bool) -> Option<String> {
    let raw = raw?;
    if mariadb {
        return (raw != "NULL").then_some(raw);
    }
    let numeric = matches!(
        generic,
        GenericType::SmallInt
            | GenericType::Integer
            | GenericType::BigInt
            | GenericType::Decimal { .. }
            | GenericType::Float
            | GenericType::Double
            | GenericType::Bool
    );
    if extra.contains("default_generated") {
        let upper = raw.to_ascii_uppercase();
        // Keywords stand alone; other expressions need parentheses in a DEFAULT clause.
        Some(if upper.starts_with("CURRENT_") || upper.starts_with("NOW") || upper.starts_with("LOCALTIME") { raw } else { format!("({raw})") })
    } else if numeric && raw.parse::<f64>().is_ok() {
        Some(raw)
    } else {
        Some(quote_literal(Dialect::Mysql, &raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_become_sql() {
        let g = GenericType::Varchar { len: 10 };
        assert_eq!(column_default(Some("abc".into()), "", g, false).as_deref(), Some("'abc'"));
        assert_eq!(column_default(Some("it's".into()), "", g, false).as_deref(), Some("'it''s'"));
        assert_eq!(column_default(Some("0".into()), "", GenericType::Integer, false).as_deref(), Some("0"));
        assert_eq!(column_default(Some("CURRENT_TIMESTAMP".into()), "default_generated", GenericType::DateTime, false).as_deref(), Some("CURRENT_TIMESTAMP"));
        assert_eq!(column_default(Some("uuid()".into()), "default_generated", g, false).as_deref(), Some("(uuid())"));
        assert_eq!(column_default(None, "", g, false), None);
        assert_eq!(column_default(Some("'abc'".into()), "", g, true).as_deref(), Some("'abc'"));
        assert_eq!(column_default(Some("NULL".into()), "", g, true), None);
    }
}
