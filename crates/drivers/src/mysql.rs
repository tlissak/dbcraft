use std::time::Instant;

use dbcraft_core::{ColumnInfo, ConnectionConfig, DbKind, Error, Result, ResultSet, TableInfo, Value};
use dbcraft_model::{DbObject, ObjectKind};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use sqlx::types::{Decimal, JsonValue};
use sqlx::{AssertSqlSafe, Executor, Row};

use crate::common::{CONNECT_TIMEOUT, block_on, cell_str, cell_u64, columns_of, db_err, decode_row, probe, returns_rows, run_atomic};

/// MySQL and MariaDB (same wire protocol).
pub struct MySqlConn {
    kind: DbKind,
    pool: MySqlPool,
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
        let pool = block_on(MySqlPoolOptions::new().max_connections(2).acquire_timeout(CONNECT_TIMEOUT).connect_with(opts))?.map_err(db_err)?;
        Ok(MySqlConn { kind: cfg.kind, pool })
    }
}

impl crate::Connection for MySqlConn {
    fn kind(&self) -> DbKind {
        self.kind
    }

    fn dialect(&self) -> Dialect {
        Dialect::Mysql
    }

    fn server_version(&mut self) -> Result<String> {
        let row = block_on(sqlx::query("select version()").fetch_one(&self.pool))?.map_err(db_err)?;
        row.try_get::<String, _>(0).map_err(db_err)
    }

    fn list_databases(&mut self) -> Result<Vec<String>> {
        let rows = block_on(sqlx::query("select schema_name from information_schema.schemata order by 1").fetch_all(&self.pool))?.map_err(db_err)?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(db_err)).collect()
    }

    fn list_objects(&mut self, database: &str) -> Result<Vec<DbObject>> {
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

    fn table_info(&mut self, database: &str) -> Result<Vec<TableInfo>> {
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

    fn table_ddl(&mut self, database: &str, _schema: Option<&str>, table: &str) -> Result<String> {
        let sql = format!("show create table {}.{}", quote_ident(Dialect::Mysql, database), quote_ident(Dialect::Mysql, table));
        let rs = self.query(None, &sql)?;
        cell_str(&rs, 0, 1).map(|s| format!("{s};")).ok_or_else(|| Error::NotFound(format!("table `{table}`")))
    }

    fn execute_atomic(&mut self, database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
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

    fn columns(&mut self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let db = database.map(|d| quote_literal(Dialect::Mysql, d)).unwrap_or_else(|| "database()".into());
        let rs = self.query(
            None,
            &format!(
                "select cast(column_name as char), cast(column_type as char), cast(is_nullable as char) \
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
                Some(ColumnInfo {
                    pk: pk.iter().position(|k| *k == name).and_then(|p| u32::try_from(p + 1).ok()),
                    name,
                    generic: crate::types::from_mysql(&native),
                    native_type: native,
                    nullable: cell_str(&rs, i, 2).as_deref() == Some("YES"),
                })
            })
            .collect())
    }

    fn primary_key(&mut self, database: Option<&str>, _schema: Option<&str>, table: &str) -> Result<Vec<String>> {
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

    fn query(&mut self, database: Option<&str>, sql: &str) -> Result<ResultSet> {
        let start = Instant::now();
        let mut rs = ResultSet::default();
        let sql = sql.to_string();
        let use_db = database.map(|d| format!("USE {}", quote_ident(Dialect::Mysql, d)));
        let wants_rows = returns_rows(&sql);
        let pool = self.pool.clone();
        let out = block_on(async move {
            let mut conn = pool.acquire().await?;
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
