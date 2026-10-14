use std::collections::HashMap;
use std::time::Instant;

use dbcraft_core::{ColumnInfo, ConnectionConfig, DbKind, Error, Result, ResultSet, TableInfo, Value};
use dbcraft_model::{DbObject, ObjectKind};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::types::{Decimal, JsonValue, Uuid};
use sqlx::{AssertSqlSafe, Row};

use crate::common::{CONNECT_TIMEOUT, block_on, cell_bool, cell_str, cell_u64, columns_of, db_err, decode_row, probe, returns_rows, run_atomic};

/// PostgreSQL scopes a connection to one database, so we keep one small pool per database the
/// user has expanded in the tree.
pub struct PgConn {
    base: PgConnectOptions,
    default_db: String,
    pools: HashMap<String, PgPool>,
}

impl PgConn {
    pub fn open(cfg: &ConnectionConfig) -> Result<Self> {
        let mut base = PgConnectOptions::new()
            .host(if cfg.host.is_empty() { "localhost" } else { &cfg.host })
            .port(cfg.port.unwrap_or(5432))
            .username(if cfg.user.is_empty() { "postgres" } else { &cfg.user });
        if let Some(pw) = &cfg.password {
            base = base.password(pw);
        }
        let default_db = if cfg.database.is_empty() { "postgres".to_string() } else { cfg.database.clone() };
        let mut conn = PgConn { base, default_db: default_db.clone(), pools: HashMap::new() };
        conn.pool(&default_db)?;
        Ok(conn)
    }

    fn pool(&mut self, db: &str) -> Result<PgPool> {
        if let Some(p) = self.pools.get(db) {
            return Ok(p.clone());
        }
        let opts = self.base.clone().database(db);
        probe::<sqlx::PgConnection>(&opts)?;
        let pool = block_on(PgPoolOptions::new().max_connections(2).acquire_timeout(CONNECT_TIMEOUT).connect_with(opts))?.map_err(db_err)?;
        self.pools.insert(db.to_string(), pool.clone());
        Ok(pool)
    }
}

impl crate::Connection for PgConn {
    fn kind(&self) -> DbKind {
        DbKind::Postgres
    }

    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn server_version(&mut self) -> Result<String> {
        let pool = self.pool(&self.default_db.clone())?;
        let row = block_on(sqlx::query("select version()").fetch_one(&pool))?.map_err(db_err)?;
        row.try_get::<String, _>(0).map_err(db_err)
    }

    fn list_databases(&mut self) -> Result<Vec<String>> {
        let pool = self.pool(&self.default_db.clone())?;
        let rows = block_on(sqlx::query("select datname from pg_database where not datistemplate order by 1").fetch_all(&pool))?.map_err(db_err)?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(db_err)).collect()
    }

    fn list_objects(&mut self, database: &str) -> Result<Vec<DbObject>> {
        let pool = self.pool(database)?;
        let sql = "select case table_type when 'VIEW' then 'view' else 'table' end, table_name, table_schema \
                   from information_schema.tables where table_schema not in ('pg_catalog','information_schema') \
                   union all \
                   select case routine_type when 'PROCEDURE' then 'procedure' else 'function' end, routine_name, routine_schema \
                   from information_schema.routines where routine_schema not in ('pg_catalog','information_schema') \
                   union all \
                   select 'sequence', sequence_name, sequence_schema from information_schema.sequences \
                   order by 3, 1, 2";
        let rows = block_on(sqlx::query(AssertSqlSafe(sql.to_string())).fetch_all(&pool))?.map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let kind = match r.try_get::<String, _>(0).map_err(db_err)?.as_str() {
                "view" => ObjectKind::View,
                "function" => ObjectKind::Function,
                "procedure" => ObjectKind::Procedure,
                "sequence" => ObjectKind::Sequence,
                _ => ObjectKind::Table,
            };
            out.push(DbObject { kind, name: r.try_get(1).map_err(db_err)?, schema: r.try_get(2).ok() });
        }
        Ok(out)
    }

    fn table_info(&mut self, database: &str) -> Result<Vec<TableInfo>> {
        let sql = "select c.relname::text, n.nspname::text, case when c.relkind in ('v','m') then 'view' else 'table' end, \
                   coalesce(case when c.reltuples >= 0 then c.reltuples::bigint end, s.n_live_tup), \
                   case when c.relkind in ('r','p','m') then pg_total_relation_size(c.oid) end, \
                   pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class') \
                   from pg_class c join pg_namespace n on n.oid = c.relnamespace \
                   left join pg_stat_user_tables s on s.relid = c.oid \
                   where c.relkind in ('r','p','v','m') and n.nspname not in ('pg_catalog','information_schema') \
                   and n.nspname not like 'pg_toast%' order by 2, 1";
        let rs = self.query(Some(database), sql)?;
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                Some(TableInfo {
                    name: cell_str(&rs, i, 0)?,
                    schema: cell_str(&rs, i, 1),
                    kind: cell_str(&rs, i, 2).unwrap_or_default(),
                    rows: cell_u64(&rs, i, 3),
                    data_length: cell_u64(&rs, i, 4),
                    owner: cell_str(&rs, i, 5),
                    comment: cell_str(&rs, i, 6),
                    ..TableInfo::default()
                })
            })
            .collect())
    }

    fn table_ddl(&mut self, database: &str, schema: Option<&str>, table: &str) -> Result<String> {
        let qualified = qualified(schema, table);
        let reg = format!("to_regclass({})", quote_literal(Dialect::Postgres, &qualified));
        let kind = self.query(Some(database), &format!("select relkind::text from pg_class where oid = {reg}"))?;
        let Some(kind) = cell_str(&kind, 0, 0) else { return Err(Error::NotFound(format!("table `{qualified}`"))) };
        if kind == "v" || kind == "m" {
            let def = self.query(Some(database), &format!("select pg_get_viewdef({reg}, true)"))?;
            let word = if kind == "m" { "MATERIALIZED VIEW" } else { "VIEW" };
            return Ok(format!("CREATE {word} {qualified} AS\n{}", cell_str(&def, 0, 0).unwrap_or_default()));
        }
        let cols = self.query(
            Some(database),
            &format!(
                "select a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(d.adbin, d.adrelid) \
                 from pg_attribute a left join pg_attrdef d on d.adrelid = a.attrelid and d.adnum = a.attnum \
                 where a.attrelid = {reg} and a.attnum > 0 and not a.attisdropped order by a.attnum"
            ),
        )?;
        let mut lines: Vec<String> = (0..cols.rows.len())
            .map(|i| {
                let mut l =
                    format!("  {} {}", quote_ident(Dialect::Postgres, &cell_str(&cols, i, 0).unwrap_or_default()), cell_str(&cols, i, 1).unwrap_or_default());
                if let Some(d) = cell_str(&cols, i, 3) {
                    l.push_str(&format!(" DEFAULT {d}"));
                }
                if cell_bool(&cols, i, 2) {
                    l.push_str(" NOT NULL");
                }
                l
            })
            .collect();
        let cons = self.query(
            Some(database),
            &format!("select conname::text, pg_get_constraintdef(oid) from pg_constraint where conrelid = {reg} order by contype desc, conname"),
        )?;
        for i in 0..cons.rows.len() {
            lines.push(format!(
                "  CONSTRAINT {} {}",
                quote_ident(Dialect::Postgres, &cell_str(&cons, i, 0).unwrap_or_default()),
                cell_str(&cons, i, 1).unwrap_or_default()
            ));
        }
        let mut ddl = format!("CREATE TABLE {qualified} (\n{}\n);", lines.join(",\n"));
        let idx = self.query(
            Some(database),
            &format!("select pg_get_indexdef(i.indexrelid) from pg_index i where i.indrelid = {reg} and not exists (select 1 from pg_constraint c where c.conindid = i.indexrelid) order by 1"),
        )?;
        for i in 0..idx.rows.len() {
            if let Some(s) = cell_str(&idx, i, 0) {
                ddl.push_str(&format!("\n\n{s};"));
            }
        }
        let comment = self.query(Some(database), &format!("select obj_description({reg}, 'pg_class')"))?;
        if let Some(c) = cell_str(&comment, 0, 0) {
            ddl.push_str(&format!("\n\nCOMMENT ON TABLE {qualified} IS {};", quote_literal(Dialect::Postgres, &c)));
        }
        Ok(ddl)
    }

    fn execute_atomic(&mut self, database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
        let pool = self.pool(&database.map(str::to_string).unwrap_or_else(|| self.default_db.clone()))?;
        let stmts = stmts.to_vec();
        block_on(async move { run_atomic!(pool.begin().await.map_err(db_err)?, stmts) })?
    }

    fn columns(&mut self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let reg = format!("to_regclass({})", quote_literal(Dialect::Postgres, &qualified(schema, table)));
        let rs = self.query(
            database,
            &format!(
                "select a.attname::text, format_type(a.atttypid, a.atttypmod), not a.attnotnull, \
                 (select array_position(i.indkey::int2[], a.attnum) + 1 from pg_index i where i.indrelid = a.attrelid and i.indisprimary) \
                 from pg_attribute a where a.attrelid = {reg} and a.attnum > 0 and not a.attisdropped order by a.attnum"
            ),
        )?;
        if rs.rows.is_empty() {
            return Err(Error::NotFound(format!("table `{}`", qualified(schema, table))));
        }
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                let native = cell_str(&rs, i, 1).unwrap_or_default();
                Some(ColumnInfo {
                    name: cell_str(&rs, i, 0)?,
                    generic: crate::types::from_postgres(&native),
                    native_type: native,
                    nullable: cell_bool(&rs, i, 2),
                    pk: cell_u64(&rs, i, 3).and_then(|p| u32::try_from(p).ok()),
                })
            })
            .collect())
    }

    fn primary_key(&mut self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<String>> {
        let reg = format!("to_regclass({})", quote_literal(Dialect::Postgres, &qualified(schema, table)));
        let rs = self.query(
            database,
            &format!(
                "select a.attname::text from pg_index i join pg_attribute a on a.attrelid = i.indrelid and a.attnum = any(i.indkey) \
                 where i.indrelid = {reg} and i.indisprimary order by array_position(i.indkey::int2[], a.attnum)"
            ),
        )?;
        Ok((0..rs.rows.len()).filter_map(|i| cell_str(&rs, i, 0)).collect())
    }

    fn query(&mut self, database: Option<&str>, sql: &str) -> Result<ResultSet> {
        let pool = self.pool(&database.map(str::to_string).unwrap_or_else(|| self.default_db.clone()))?;
        let start = Instant::now();
        let mut rs = ResultSet::default();
        if returns_rows(sql) {
            let rows = block_on(sqlx::query(AssertSqlSafe(sql.to_string())).fetch_all(&pool))?.map_err(db_err)?;
            if let Some(first) = rows.first() {
                rs.columns = columns_of!(first);
            }
            rs.rows = rows
                .iter()
                .map(|row| {
                    decode_row!(row,
                        bool => Value::Bool,
                        i64 => Value::Int,
                        i32 => |x: i32| Value::Int(x.into()),
                        i16 => |x: i16| Value::Int(x.into()),
                        f64 => Value::Float,
                        f32 => |x: f32| Value::Float(x.into()),
                        String => Value::Text,
                        Decimal => |x: Decimal| Value::Text(x.to_string()),
                        NaiveDateTime => |x: NaiveDateTime| Value::Text(x.to_string()),
                        DateTime<Utc> => |x: DateTime<Utc>| Value::Text(x.to_rfc3339()),
                        NaiveDate => |x: NaiveDate| Value::Text(x.to_string()),
                        NaiveTime => |x: NaiveTime| Value::Text(x.to_string()),
                        Uuid => |x: Uuid| Value::Text(x.to_string()),
                        JsonValue => |x: JsonValue| Value::Text(x.to_string()),
                        Vec<u8> => Value::Bytes,
                    )
                })
                .collect();
        } else {
            rs.rows_affected = block_on(sqlx::raw_sql(AssertSqlSafe(sql.to_string())).execute(&pool))?.map_err(db_err)?.rows_affected();
        }
        rs.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(rs)
    }
}

/// `"schema"."table"` (or just `"table"`), quoted for `to_regclass`.
fn qualified(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(s) => format!("{}.{}", quote_ident(Dialect::Postgres, s), quote_ident(Dialect::Postgres, table)),
        None => quote_ident(Dialect::Postgres, table),
    }
}
