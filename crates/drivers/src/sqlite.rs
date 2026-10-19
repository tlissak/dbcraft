use std::time::Instant;

use dbcraft_core::{ColumnInfo, ConnectionConfig, DbKind, Error, ForeignKeyInfo, IndexInfo, Result, ResultSet, TableInfo, TableSchema, Value, ViewInfo};
use dbcraft_model::{DbObject, ObjectKind};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::{AssertSqlSafe, Row};

use crate::common::{CONNECT_TIMEOUT, block_on, cell_str, cell_u64, columns_of, current_cancel, db_err, decode_row, returns_rows, run_atomic};

pub struct SqliteConn {
    pool: SqlitePool,
    opts: SqliteConnectOptions,
}

impl SqliteConn {
    pub fn open(cfg: &ConnectionConfig) -> Result<Self> {
        if cfg.database.trim().is_empty() {
            return Err(Error::InvalidParam("SQLite connection needs a database file path".into()));
        }
        let opts = SqliteConnectOptions::new().filename(&cfg.database).create_if_missing(true);
        // Several connections let the tree read while a long query runs; an in-memory database
        // exists per connection, so it gets exactly one.
        let memory = cfg.database.trim() == ":memory:" || cfg.database.contains("mode=memory");
        Self::with_pool(opts, if memory { 1 } else { 4 })
    }

    fn with_pool(opts: SqliteConnectOptions, size: u32) -> Result<Self> {
        let pool = block_on(
            SqlitePoolOptions::new()
                .max_connections(size)
                // A Stop handler from an earlier query must not interrupt the next one.
                .before_acquire(|conn, _| {
                    Box::pin(async move {
                        conn.lock_handle().await?.remove_progress_handler();
                        Ok(true)
                    })
                })
                .acquire_timeout(CONNECT_TIMEOUT)
                .connect_with(opts.clone()),
        )?
        .map_err(db_err)?;
        Ok(SqliteConn { pool, opts })
    }
}

impl crate::Connection for SqliteConn {
    fn kind(&self) -> DbKind {
        DbKind::Sqlite
    }

    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    fn server_version(&self) -> Result<String> {
        let row = block_on(sqlx::query("select sqlite_version()").fetch_one(&self.pool))?.map_err(db_err)?;
        Ok(format!("SQLite {}", row.try_get::<String, _>(0).map_err(db_err)?))
    }

    fn list_databases(&self) -> Result<Vec<String>> {
        let rows = block_on(sqlx::query("select name from pragma_database_list order by seq").fetch_all(&self.pool))?.map_err(db_err)?;
        rows.iter().map(|r| r.try_get::<String, _>(0).map_err(db_err)).collect()
    }

    fn list_objects(&self, database: &str) -> Result<Vec<DbObject>> {
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

    fn table_info(&self, database: &str) -> Result<Vec<TableInfo>> {
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

    fn table_ddl(&self, database: &str, _schema: Option<&str>, table: &str) -> Result<String> {
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

    fn execute_atomic(&self, _database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
        let pool = self.pool.clone();
        let stmts = stmts.to_vec();
        block_on(async move { run_atomic!(pool.begin().await.map_err(db_err)?, stmts) })?
    }

    fn columns(&self, database: Option<&str>, _schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        let sql = format!(
            "select name, type, \"notnull\", pk, dflt_value from pragma_table_info({}, {}) order by cid",
            quote_literal(Dialect::Sqlite, table),
            quote_literal(Dialect::Sqlite, database.unwrap_or("main"))
        );
        let rs = self.query(None, &sql)?;
        if rs.rows.is_empty() {
            return Err(Error::NotFound(format!("table `{table}`")));
        }
        let mut cols: Vec<ColumnInfo> = (0..rs.rows.len())
            .filter_map(|i| {
                let native = cell_str(&rs, i, 1).unwrap_or_default();
                let mut c = ColumnInfo::new(
                    cell_str(&rs, i, 0)?,
                    native.clone(),
                    crate::types::from_sqlite(&native),
                    cell_u64(&rs, i, 2) != Some(1),
                    cell_u64(&rs, i, 3).filter(|p| *p > 0).and_then(|p| u32::try_from(p).ok()),
                );
                c.default = cell_str(&rs, i, 4);
                Some(c)
            })
            .collect();
        // A single-column `INTEGER PRIMARY KEY` is the rowid: SQLite numbers it by itself.
        let pk: Vec<usize> = cols.iter().enumerate().filter(|(_, c)| c.pk.is_some()).map(|(i, _)| i).collect();
        if let [only] = pk.as_slice()
            && let Some(c) = cols.get_mut(*only)
            && c.native_type.eq_ignore_ascii_case("integer")
        {
            c.auto_increment = true;
        }
        Ok(cols)
    }

    fn table_schema(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<TableSchema> {
        let db = database.unwrap_or("main");
        let (t, d) = (quote_literal(Dialect::Sqlite, table), quote_literal(Dialect::Sqlite, db));
        let columns = self.columns(database, schema, table)?;
        let mut indexes = Vec::new();
        let list =
            self.query(None, &format!("select name, \"unique\", origin, partial from pragma_index_list({t}, {d}) where origin <> 'pk' order by name"))?;
        for i in 0..list.rows.len() {
            let Some(name) = cell_str(&list, i, 0) else { continue };
            let cols = self.query(None, &format!("select name from pragma_index_info({}, {d}) order by seqno", quote_literal(Dialect::Sqlite, &name)))?;
            let names: Vec<Option<String>> = (0..cols.rows.len()).map(|r| cell_str(&cols, r, 0)).collect();
            let expression = names.iter().any(Option::is_none) || cell_u64(&list, i, 3) == Some(1);
            let native_ddl = if expression {
                let master = format!("{}.sqlite_master", quote_ident(Dialect::Sqlite, db));
                let ddl = self.query(None, &format!("select sql from {master} where type = 'index' and name = {}", quote_literal(Dialect::Sqlite, &name)))?;
                cell_str(&ddl, 0, 0)
            } else {
                None
            };
            // UNIQUE constraints show up as `sqlite_autoindex_*`, a name nobody may create.
            let name = if name.starts_with("sqlite_") { format!("uq_{table}_{}", names.iter().flatten().cloned().collect::<Vec<_>>().join("_")) } else { name };
            indexes.push(IndexInfo { name, columns: names.into_iter().flatten().collect(), unique: cell_u64(&list, i, 1) == Some(1), native_ddl });
        }
        let fk =
            self.query(None, &format!("select id, \"table\", \"from\", \"to\", on_update, on_delete from pragma_foreign_key_list({t}, {d}) order by id, seq"))?;
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        let mut last_id = None;
        for i in 0..fk.rows.len() {
            let id = cell_u64(&fk, i, 0);
            if id != last_id || foreign_keys.is_empty() {
                last_id = id;
                foreign_keys.push(ForeignKeyInfo {
                    name: format!("fk_{table}_{}", id.unwrap_or(0)),
                    ref_table: cell_str(&fk, i, 1).unwrap_or_default(),
                    on_update: cell_str(&fk, i, 4).filter(|a| a != "NO ACTION"),
                    on_delete: cell_str(&fk, i, 5).filter(|a| a != "NO ACTION"),
                    ..ForeignKeyInfo::default()
                });
            }
            if let Some(f) = foreign_keys.last_mut() {
                f.columns.push(cell_str(&fk, i, 2).unwrap_or_default());
                // `to` is NULL when the FK names the parent's primary key implicitly.
                f.ref_columns.extend(cell_str(&fk, i, 3));
            }
        }
        for f in &mut foreign_keys {
            if f.ref_columns.is_empty() {
                f.ref_columns = self.primary_key(database, None, &f.ref_table)?;
            }
        }
        Ok(TableSchema { schema: None, name: table.to_string(), columns, primary_key_name: None, indexes, foreign_keys, comment: None })
    }

    fn dedicated(&self) -> Result<Box<dyn crate::Connection>> {
        Ok(Box::new(Self::with_pool(self.opts.clone(), 1)?))
    }

    fn views(&self, database: &str) -> Result<Vec<ViewInfo>> {
        let master = format!("{}.sqlite_master", quote_ident(Dialect::Sqlite, database));
        let rs = self.query(None, &format!("select name, sql from {master} where type = 'view' order by name"))?;
        Ok((0..rs.rows.len())
            .filter_map(|i| Some(ViewInfo { schema: None, name: cell_str(&rs, i, 0)?, definition: crate::common::view_select(&cell_str(&rs, i, 1)?)? }))
            .collect())
    }

    fn primary_key(&self, database: Option<&str>, _schema: Option<&str>, table: &str) -> Result<Vec<String>> {
        let database = database.unwrap_or("main");
        let sql = format!(
            "select name from pragma_table_info({}, {}) where pk > 0 order by pk",
            quote_literal(Dialect::Sqlite, table),
            quote_literal(Dialect::Sqlite, database)
        );
        let rs = self.query(None, &sql)?;
        Ok((0..rs.rows.len()).filter_map(|i| cell_str(&rs, i, 0)).collect())
    }

    fn query(&self, _database: Option<&str>, sql: &str) -> Result<ResultSet> {
        let start = Instant::now();
        let mut rs = ResultSet::default();
        let wants_rows = returns_rows(sql);
        let (pool, sql, cancel) = (self.pool.clone(), sql.to_string(), current_cancel());
        let (rows, affected) = block_on(async move {
            let mut conn = pool.acquire().await?;
            // With a Stop button, SQLite checks it every few thousand VM steps and interrupts.
            if let Some(c) = cancel {
                conn.lock_handle().await?.set_progress_handler(2000, move || !c.is_cancelled());
            }
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
                        i64 => Value::Int,
                        f64 => Value::Float,
                        String => Value::Text,
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
