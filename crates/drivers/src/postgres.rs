use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use dbcraft_core::{
    CheckInfo, ColumnInfo, ConnectionConfig, DbKind, Error, ForeignKeyInfo, IndexInfo, Result, ResultSet, TableInfo, TableSchema, TriggerInfo, UniqueInfo,
    Value, ViewInfo,
};
use dbcraft_model::{DbObject, ObjectKind};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::types::{Decimal, JsonValue, Uuid};
use sqlx::{AssertSqlSafe, Row};

use crate::common::{
    CONNECT_TIMEOUT, block_on, cell_bool, cell_str, cell_u64, columns_of, current_cancel, db_err, decode_row, on_cancel, probe, returns_rows, run_atomic,
    spawn, split_list,
};

/// Connections per database pool: a query tab, a table tab and the tree can work at once.
const POOL_SIZE: u32 = 4;

/// PostgreSQL scopes a connection to one database, so we keep one small pool per database the
/// user has expanded in the tree.
pub struct PgConn {
    base: PgConnectOptions,
    default_db: String,
    pools: Mutex<HashMap<String, PgPool>>,
    pool_size: u32,
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
        let conn = PgConn { base, default_db: default_db.clone(), pools: Mutex::new(HashMap::new()), pool_size: POOL_SIZE };
        conn.pool(&default_db)?;
        Ok(conn)
    }

    fn pool(&self, db: &str) -> Result<PgPool> {
        if let Some(p) = self.pools.lock().unwrap_or_else(PoisonError::into_inner).get(db) {
            return Ok(p.clone());
        }
        // Connect without holding the lock (it can take seconds); a concurrent caller may build a
        // second pool for the same database, and the first one stored wins.
        let opts = self.base.clone().database(db);
        probe::<sqlx::PgConnection>(&opts)?;
        let pool = block_on(PgPoolOptions::new().max_connections(self.pool_size).acquire_timeout(CONNECT_TIMEOUT).connect_with(opts))?.map_err(db_err)?;
        Ok(self.pools.lock().unwrap_or_else(PoisonError::into_inner).entry(db.to_string()).or_insert(pool).clone())
    }
}

impl crate::Connection for PgConn {
    fn kind(&self) -> DbKind {
        DbKind::Postgres
    }

    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn server_version(&self) -> Result<String> {
        let pool = self.pool(&self.default_db.clone())?;
        let row = block_on(sqlx::query("select version()").fetch_one(&pool))?.map_err(db_err)?;
        row.try_get::<String, _>(0).map_err(db_err)
    }

    fn list_databases(&self) -> Result<Vec<String>> {
        let pool = self.pool(&self.default_db.clone())?;
        let rows = block_on(sqlx::query("select datname from pg_database where not datistemplate order by 1").fetch_all(&pool))?.map_err(db_err)?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(db_err)).collect()
    }

    fn list_objects(&self, database: &str) -> Result<Vec<DbObject>> {
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

    fn table_info(&self, database: &str) -> Result<Vec<TableInfo>> {
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

    fn table_ddl(&self, database: &str, schema: Option<&str>, table: &str) -> Result<String> {
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

    fn execute_atomic(&self, database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
        let pool = self.pool(&database.map(str::to_string).unwrap_or_else(|| self.default_db.clone()))?;
        let stmts = stmts.to_vec();
        block_on(async move { run_atomic!(pool.begin().await.map_err(db_err)?, stmts) })?
    }

    fn columns(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let reg = format!("to_regclass({})", quote_literal(Dialect::Postgres, &qualified(schema, table)));
        let rs = self.query(
            database,
            &format!(
                "select a.attname::text, format_type(a.atttypid, a.atttypmod), not a.attnotnull, \
                 (select array_position(i.indkey::int2[], a.attnum) + 1 from pg_index i where i.indrelid = a.attrelid and i.indisprimary), \
                 pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, col_description(a.attrelid, a.attnum) \
                 from pg_attribute a left join pg_attrdef d on d.adrelid = a.attrelid and d.adnum = a.attnum \
                 where a.attrelid = {reg} and a.attnum > 0 and not a.attisdropped order by a.attnum"
            ),
        )?;
        if rs.rows.is_empty() {
            return Err(Error::NotFound(format!("table `{}`", qualified(schema, table))));
        }
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                let native = cell_str(&rs, i, 1).unwrap_or_default();
                let mut c = ColumnInfo::new(
                    cell_str(&rs, i, 0)?,
                    native.clone(),
                    crate::types::from_postgres(&native),
                    cell_bool(&rs, i, 2),
                    cell_u64(&rs, i, 3).and_then(|p| u32::try_from(p).ok()),
                );
                let default = cell_str(&rs, i, 4);
                let identity = cell_str(&rs, i, 5).is_some_and(|x| x == "a" || x == "d");
                // serial/bigserial: the default is the sequence; the column is simply auto-increment.
                let serial = default.as_deref().is_some_and(|d| d.starts_with("nextval("));
                c.auto_increment = identity || serial;
                c.default = default.filter(|_| !serial);
                c.comment = cell_str(&rs, i, 6);
                Some(c)
            })
            .collect())
    }

    fn table_schema(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<TableSchema> {
        let reg = format!("to_regclass({})", quote_literal(Dialect::Postgres, &qualified(schema, table)));
        let columns = self.columns(database, schema, table)?;
        let owner = self.query(
            database,
            &format!(
                "select n.nspname::text, obj_description(c.oid, 'pg_class') from pg_class c join pg_namespace n on n.oid = c.relnamespace where c.oid = {reg}"
            ),
        )?;
        let own_schema = cell_str(&owner, 0, 0);
        let pk_name = self.query(database, &format!("select conname::text from pg_constraint where conrelid = {reg} and contype = 'p'"))?;
        let idx = self.query(
            database,
            &format!(
                "select i.relname::text, ix.indisunique, pg_get_indexdef(ix.indexrelid), \
                 array_to_string(array(select a.attname from unnest(ix.indkey) with ordinality k(attnum, ord) \
                   join pg_attribute a on a.attrelid = ix.indrelid and a.attnum = k.attnum order by k.ord), chr(31)), \
                 (ix.indexprs is not null or ix.indpred is not null or 0 = any(ix.indkey::int2[])), am.amname::text, obj_description(i.oid, 'pg_class') \
                 from pg_index ix join pg_class i on i.oid = ix.indexrelid join pg_am am on am.oid = i.relam \
                 where ix.indrelid = {reg} and not ix.indisprimary \
                 and not exists (select 1 from pg_constraint c where c.conindid = ix.indexrelid and c.contype in ('u', 'x')) order by 1"
            ),
        )?;
        let indexes = (0..idx.rows.len())
            .filter_map(|i| {
                let expression = cell_bool(&idx, i, 4);
                Some(IndexInfo {
                    name: cell_str(&idx, i, 0)?,
                    unique: cell_bool(&idx, i, 1),
                    columns: if expression { Vec::new() } else { split_list(cell_str(&idx, i, 3)) },
                    native_ddl: if expression { cell_str(&idx, i, 2) } else { None },
                    method: cell_str(&idx, i, 5).map(|m| m.to_ascii_uppercase()).filter(|m| m != "BTREE"),
                    kind: None,
                    comment: cell_str(&idx, i, 6),
                })
            })
            .collect();
        let list = |key: &str, rel: &str| {
            format!(
                "array_to_string(array(select a.attname from unnest(c.{key}) with ordinality k(n, o) \
                 join pg_attribute a on a.attrelid = c.{rel} and a.attnum = k.n order by k.o), chr(31))"
            )
        };
        let fk = self.query(
            database,
            &format!(
                "select c.conname::text, {}, rn.nspname::text, r.relname::text, {}, c.confdeltype::text, c.confupdtype::text \
                 from pg_constraint c join pg_class r on r.oid = c.confrelid join pg_namespace rn on rn.oid = r.relnamespace \
                 where c.conrelid = {reg} and c.contype = 'f' order by 1",
                list("conkey", "conrelid"),
                list("confkey", "confrelid")
            ),
        )?;
        let action = |code: Option<String>| match code.as_deref() {
            Some("c") => Some("CASCADE".to_string()),
            Some("n") => Some("SET NULL".to_string()),
            Some("d") => Some("SET DEFAULT".to_string()),
            Some("r") => Some("RESTRICT".to_string()),
            _ => None,
        };
        let foreign_keys = (0..fk.rows.len())
            .filter_map(|i| {
                Some(ForeignKeyInfo {
                    name: cell_str(&fk, i, 0)?,
                    columns: split_list(cell_str(&fk, i, 1)),
                    ref_schema: cell_str(&fk, i, 2).filter(|s| Some(s) != own_schema.as_ref()),
                    ref_table: cell_str(&fk, i, 3)?,
                    ref_columns: split_list(cell_str(&fk, i, 4)),
                    on_delete: action(cell_str(&fk, i, 5)),
                    on_update: action(cell_str(&fk, i, 6)),
                })
            })
            .collect();
        let cons = self.query(
            database,
            &format!(
                "select c.conname::text, c.contype::text, pg_get_constraintdef(c.oid), {} \
                 from pg_constraint c where c.conrelid = {reg} and c.contype in ('u', 'c') order by 1",
                list("conkey", "conrelid")
            ),
        )?;
        let mut uniques = Vec::new();
        let mut checks = Vec::new();
        for i in 0..cons.rows.len() {
            let Some(name) = cell_str(&cons, i, 0) else { continue };
            if cell_str(&cons, i, 1).as_deref() == Some("u") {
                uniques.push(UniqueInfo { name, columns: split_list(cell_str(&cons, i, 3)) });
            } else if let Some(def) = cell_str(&cons, i, 2) {
                // `CHECK ((x > 0))` -> `(x > 0)`.
                let expression = crate::sqltext::parens_after(&def, 0).map(|(e, _)| e).unwrap_or(def);
                checks.push(CheckInfo { name, expression });
            }
        }
        let trg = self
            .query(database, &format!("select tgname::text, pg_get_triggerdef(oid) from pg_trigger where tgrelid = {reg} and not tgisinternal order by 1"))?;
        let triggers = (0..trg.rows.len())
            .filter_map(|i| {
                let (timing, events, for_each_row, condition, body) = crate::sqltext::trigger_parts(&cell_str(&trg, i, 1)?)?;
                Some(TriggerInfo { name: cell_str(&trg, i, 0)?, timing, events, for_each_row, condition, body })
            })
            .collect();
        let opt = self.query(
            database,
            &format!("select t.spcname::text, c.relpersistence::text from pg_class c left join pg_tablespace t on t.oid = c.reltablespace where c.oid = {reg}"),
        )?;
        let mut options = std::collections::BTreeMap::new();
        if let Some(ts) = cell_str(&opt, 0, 0) {
            options.insert("tablespace".to_string(), ts);
        }
        if cell_str(&opt, 0, 1).as_deref() == Some("u") {
            options.insert("unlogged".to_string(), "true".to_string());
        }
        Ok(TableSchema {
            schema: own_schema,
            name: table.to_string(),
            columns,
            primary_key_name: cell_str(&pk_name, 0, 0),
            indexes,
            foreign_keys,
            uniques,
            checks,
            triggers,
            options,
            comment: cell_str(&owner, 0, 1),
        })
    }

    fn dedicated(&self) -> Result<Box<dyn crate::Connection>> {
        // One connection per database it touches (PostgreSQL sessions belong to one database).
        let conn = PgConn { base: self.base.clone(), default_db: self.default_db.clone(), pools: Mutex::new(HashMap::new()), pool_size: 1 };
        conn.pool(&conn.default_db)?;
        Ok(Box::new(conn))
    }

    fn views(&self, database: &str) -> Result<Vec<ViewInfo>> {
        let rs = self.query(
            Some(database),
            "select n.nspname::text, c.relname::text, pg_get_viewdef(c.oid, true) from pg_class c join pg_namespace n on n.oid = c.relnamespace \
             where c.relkind = 'v' and n.nspname not in ('pg_catalog', 'information_schema') order by 1, 2",
        )?;
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                Some(ViewInfo {
                    schema: cell_str(&rs, i, 0),
                    name: cell_str(&rs, i, 1)?,
                    definition: cell_str(&rs, i, 2)?.trim().trim_end_matches(';').trim().to_string(),
                })
            })
            .collect())
    }

    fn primary_key(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<String>> {
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

    fn query(&self, database: Option<&str>, sql: &str) -> Result<ResultSet> {
        let db = database.map(str::to_string).unwrap_or_else(|| self.default_db.clone());
        let pool = self.pool(&db)?;
        let start = Instant::now();
        let mut rs = ResultSet::default();
        let cancellable = current_cancel().is_some();
        let wants_rows = returns_rows(sql);
        let sql = sql.to_string();
        let kill_opts = self.base.clone().database(&db);
        let (rows, affected) = block_on(async move {
            let mut conn = pool.acquire().await?;
            // With a Stop button, remember the backend so Stop can cancel the statement server-side.
            let _kill = if cancellable {
                let pid: i32 = sqlx::query_scalar("select pg_backend_pid()").fetch_one(&mut *conn).await?;
                Some(on_cancel(move || {
                    spawn(async move {
                        if let Ok(mut c) = <sqlx::PgConnection as sqlx::Connection>::connect_with(&kill_opts).await {
                            let _ = sqlx::query("select pg_cancel_backend($1)").bind(pid).execute(&mut c).await;
                            let _ = sqlx::Connection::close(c).await;
                        }
                    })
                }))
            } else {
                None
            };
            if wants_rows {
                Ok::<_, sqlx::Error>((sqlx::query(AssertSqlSafe(sql)).fetch_all(&mut *conn).await?, 0))
            } else {
                Ok((Vec::new(), sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut *conn).await?.rows_affected()))
            }
        })?
        .map_err(db_err)?;
        if wants_rows {
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
            rs.rows_affected = affected;
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
