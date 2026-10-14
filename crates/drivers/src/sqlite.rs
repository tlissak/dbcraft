use std::time::Instant;

use dbcraft_core::{ColumnInfo, ConnectionConfig, DbKind, Error, Result, ResultSet, TableInfo, Value};
use dbcraft_model::{DbObject, ObjectKind};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::{AssertSqlSafe, Row};

use crate::common::{CONNECT_TIMEOUT, block_on, cell_str, cell_u64, columns_of, db_err, decode_row, returns_rows, run_atomic};

pub struct SqliteConn {
    pool: SqlitePool,
}

impl SqliteConn {
    pub fn open(cfg: &ConnectionConfig) -> Result<Self> {
        if cfg.database.trim().is_empty() {
            return Err(Error::InvalidParam("SQLite connection needs a database file path".into()));
        }
        let opts = SqliteConnectOptions::new().filename(&cfg.database).create_if_missing(true);
        let pool = block_on(SqlitePoolOptions::new().max_connections(1).acquire_timeout(CONNECT_TIMEOUT).connect_with(opts))?.map_err(db_err)?;
        Ok(SqliteConn { pool })
    }
}

impl crate::Connection for SqliteConn {
    fn kind(&self) -> DbKind {
        DbKind::Sqlite
    }

    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    fn server_version(&mut self) -> Result<String> {
        let row = block_on(sqlx::query("select sqlite_version()").fetch_one(&self.pool))?.map_err(db_err)?;
        Ok(format!("SQLite {}", row.try_get::<String, _>(0).map_err(db_err)?))
    }

    fn list_databases(&mut self) -> Result<Vec<String>> {
        let rows = block_on(sqlx::query("select name from pragma_database_list order by seq").fetch_all(&self.pool))?.map_err(db_err)?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(db_err)).collect()
    }

    fn list_objects(&mut self, database: &str) -> Result<Vec<DbObject>> {
        let master = format!("{}.sqlite_master", quote_ident(Dialect::Sqlite, database));
        let sql = format!("select type, name from {master} where type in ('table','view','trigger','index') and name not like 'sqlite_%' order by type, name");
        let rows = block_on(sqlx::query(AssertSqlSafe(sql.clone())).fetch_all(&self.pool))?.map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let kind = match r.try_get::<String, _>(0).map_err(db_err)?.as_str() {
                "table" => ObjectKind::Table,
                "view" => ObjectKind::View,
                "trigger" => ObjectKind::Trigger,
                _ => ObjectKind::Index,
            };
            out.push(DbObject { kind, name: r.try_get(1).map_err(db_err)?, schema: None });
        }
        Ok(out)
    }

    fn table_info(&mut self, database: &str) -> Result<Vec<TableInfo>> {
        let db = quote_ident(Dialect::Sqlite, database);
        let list =
            self.query(None, &format!("select name, type from {db}.sqlite_master where type in ('table','view') and name not like 'sqlite_%' order by name"))?;
        let mut out = Vec::with_capacity(list.rows.len());
        for i in 0..list.rows.len() {
            let Some(name) = cell_str(&list, i, 0) else { continue };
            let kind = cell_str(&list, i, 1).unwrap_or_default();
            // SQLite keeps no row estimate; count tables exactly (as Navicat does), skip views.
            let rows = if kind == "table" {
                let n = self.query(None, &format!("select count(*) from {db}.{}", quote_ident(Dialect::Sqlite, &name)))?;
                cell_u64(&n, 0, 0)
            } else {
                None
            };
            out.push(TableInfo { name, kind, rows, ..TableInfo::default() });
        }
        Ok(out)
    }

    fn table_ddl(&mut self, database: &str, _schema: Option<&str>, table: &str) -> Result<String> {
        let sql = format!(
            "select sql from {}.sqlite_master where name = {} and sql is not null order by type desc",
            quote_ident(Dialect::Sqlite, database),
            quote_literal(Dialect::Sqlite, table)
        );
        let rs = self.query(None, &sql)?;
        let parts: Vec<String> = (0..rs.rows.len()).filter_map(|i| cell_str(&rs, i, 0)).map(|s| format!("{s};")).collect();
        if parts.is_empty() {
            return Err(Error::NotFound(format!("table `{table}`")));
        }
        Ok(parts.join("\n\n"))
    }

    fn execute_atomic(&mut self, _database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
        let pool = self.pool.clone();
        let stmts = stmts.to_vec();
        block_on(async move { run_atomic!(pool.begin().await.map_err(db_err)?, stmts) })?
    }

    fn columns(&mut self, database: Option<&str>, _schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let sql = format!(
            "select name, type, \"notnull\", pk from pragma_table_info({}, {}) order by cid",
            quote_literal(Dialect::Sqlite, table),
            quote_literal(Dialect::Sqlite, database.unwrap_or("main"))
        );
        let rs = self.query(None, &sql)?;
        if rs.rows.is_empty() {
            return Err(Error::NotFound(format!("table `{table}`")));
        }
        Ok((0..rs.rows.len())
            .filter_map(|i| {
                let native = cell_str(&rs, i, 1).unwrap_or_default();
                Some(ColumnInfo {
                    name: cell_str(&rs, i, 0)?,
                    generic: crate::types::from_sqlite(&native),
                    native_type: native,
                    nullable: cell_u64(&rs, i, 2) != Some(1),
                    pk: cell_u64(&rs, i, 3).filter(|p| *p > 0).and_then(|p| u32::try_from(p).ok()),
                })
            })
            .collect())
    }

    fn primary_key(&mut self, database: Option<&str>, _schema: Option<&str>, table: &str) -> Result<Vec<String>> {
        let database = database.unwrap_or("main");
        let sql = format!(
            "select name from pragma_table_info({}, {}) where pk > 0 order by pk",
            quote_literal(Dialect::Sqlite, table),
            quote_literal(Dialect::Sqlite, database)
        );
        let rs = self.query(None, &sql)?;
        Ok((0..rs.rows.len()).filter_map(|i| cell_str(&rs, i, 0)).collect())
    }

    fn query(&mut self, _database: Option<&str>, sql: &str) -> Result<ResultSet> {
        let start = Instant::now();
        let mut rs = ResultSet::default();
        if returns_rows(sql) {
            let rows = block_on(sqlx::query(AssertSqlSafe(sql.to_string())).fetch_all(&self.pool))?.map_err(db_err)?;
            if let Some(first) = rows.first() {
                rs.columns = columns_of!(first);
            }
            rs.rows = rows
                .iter()
                .map(|row| {
                    decode_row!(row,
                        i64 => Value::Int,
                        f64 => Value::Float,
                        String => Value::Text,
                        Vec<u8> => Value::Bytes,
                    )
                })
                .collect();
        } else {
            rs.rows_affected = block_on(sqlx::raw_sql(AssertSqlSafe(sql.to_string())).execute(&self.pool))?.map_err(db_err)?.rows_affected();
        }
        rs.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(rs)
    }
}
