//! Database drivers behind one blocking [`Connection`] trait. The engine and everything above it
//! are synchronous; each driver owns an async sqlx pool and blocks on a shared tokio runtime.
//!
//! Adding an engine (SQL Server, Oracle, MongoDB, Redis): implement [`Connection`] in a new module,
//! route it in [`connect`], add a docker service in `dev/docker-compose.yml` and an integration
//! test in `tests/`.

mod common;
mod mysql;
mod postgres;
mod sqlite;
mod tunnel;
pub mod types;
pub mod users;

pub use common::{Cancel, current_cancel, with_cancel};

use dbcraft_core::{ColumnInfo, ConnectionConfig, DbKind, Error, Result, ResultSet, TableInfo, TableSchema, ViewInfo};
use dbcraft_model::DbObject;
use dbcraft_sql::Dialect;

/// A live session with one server (or one SQLite file). Shared between the UI thread and
/// background workers (`Arc<dyn Connection>`), so implementations pool connections internally.
/// Calls honour the calling thread's [`Cancel`] (see [`with_cancel`]).
pub trait Connection: Send + Sync {
    fn kind(&self) -> DbKind;
    fn dialect(&self) -> Dialect;
    fn server_version(&self) -> Result<String>;
    /// Databases on the server (SQLite: attached databases, normally just `main`).
    fn list_databases(&self) -> Result<Vec<String>>;
    /// Tables, views, routines… in one database.
    fn list_objects(&self, database: &str) -> Result<Vec<DbObject>>;
    /// Tables and views with the details the Objects tab shows (rows, size, engine, dates…).
    fn table_info(&self, database: &str) -> Result<Vec<TableInfo>>;
    /// `CREATE TABLE` / `CREATE VIEW` text for the information pane.
    fn table_ddl(&self, database: &str, schema: Option<&str>, table: &str) -> Result<String>;
    /// Primary key columns in key order (empty when the table has none).
    fn primary_key(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<String>>;
    /// Columns with native and engine-neutral types, nullability, key position, default and
    /// auto-increment.
    fn columns(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>>;
    /// Columns, primary key, indexes and foreign keys: what Transfer, Backup and Structure
    /// Synchronization need to recreate or compare a table.
    fn table_schema(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<TableSchema>;
    /// Views with their `SELECT` text.
    fn views(&self, database: &str) -> Result<Vec<ViewInfo>>;
    /// A connection of its own: one server session, so `BEGIN`, `LOCK TABLES` and the queries after
    /// them share it (consistent backups). The shared connection uses a pool instead.
    fn dedicated(&self) -> Result<Box<dyn Connection>>;
    /// Run statements in one transaction on one connection. A statement paired with `Some(n)` must
    /// affect exactly `n` rows; otherwise, or on any error, everything is rolled back.
    fn execute_atomic(&self, database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>>;
    /// Run one statement, optionally in the context of `database`.
    fn query(&self, database: Option<&str>, sql: &str) -> Result<ResultSet>;
}

/// Open a connection, through an SSH tunnel when configured. Fails with an actionable error
/// (never panics) on bad config, no server or rejected credentials.
pub fn connect(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    match (&cfg.ssh, cfg.kind) {
        (Some(_), DbKind::Sqlite) => Err(Error::InvalidParam("SSH tunnels apply to server connections, not SQLite files".into())),
        (Some(ssh), kind) => {
            let db_host = if cfg.host.trim().is_empty() { "localhost" } else { cfg.host.trim() };
            let db_port = cfg.port.or(kind.default_port()).ok_or_else(|| Error::InvalidParam("port is required".into()))?;
            let tunnel = tunnel::open(ssh, db_host, db_port)?;
            let local = ConnectionConfig { host: "127.0.0.1".into(), port: Some(tunnel.local_port), ssh: None, ..cfg.clone() };
            Ok(Box::new(Tunneled { inner: connect_direct(&local)?, _tunnel: std::sync::Arc::new(tunnel) }))
        }
        (None, _) => connect_direct(cfg),
    }
}

fn connect_direct(cfg: &ConnectionConfig) -> Result<Box<dyn Connection>> {
    match cfg.kind {
        DbKind::Sqlite => Ok(Box::new(sqlite::SqliteConn::open(cfg)?)),
        DbKind::Postgres => Ok(Box::new(postgres::PgConn::open(cfg)?)),
        DbKind::Mysql | DbKind::Mariadb => Ok(Box::new(mysql::MySqlConn::open(cfg)?)),
        other => Err(Error::Unsupported(format!("{} driver is not implemented yet (see ROADMAP.md)", other.label()))),
    }
}

/// Engines that [`connect`] can open today.
pub fn supported_kinds() -> &'static [DbKind] {
    &[DbKind::Sqlite, DbKind::Postgres, DbKind::Mysql, DbKind::Mariadb]
}

/// A connection that owns its SSH tunnel. Field order matters: the pool drops before the tunnel.
struct Tunneled {
    inner: Box<dyn Connection>,
    _tunnel: std::sync::Arc<tunnel::Tunnel>,
}

impl Connection for Tunneled {
    fn kind(&self) -> DbKind {
        self.inner.kind()
    }
    fn dialect(&self) -> Dialect {
        self.inner.dialect()
    }
    fn server_version(&self) -> Result<String> {
        self.inner.server_version()
    }
    fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases()
    }
    fn list_objects(&self, database: &str) -> Result<Vec<DbObject>> {
        self.inner.list_objects(database)
    }
    fn table_info(&self, database: &str) -> Result<Vec<TableInfo>> {
        self.inner.table_info(database)
    }
    fn table_ddl(&self, database: &str, schema: Option<&str>, table: &str) -> Result<String> {
        self.inner.table_ddl(database, schema, table)
    }
    fn primary_key(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<String>> {
        self.inner.primary_key(database, schema, table)
    }
    fn columns(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<Vec<ColumnInfo>> {
        self.inner.columns(database, schema, table)
    }
    fn table_schema(&self, database: Option<&str>, schema: Option<&str>, table: &str) -> Result<TableSchema> {
        self.inner.table_schema(database, schema, table)
    }
    fn views(&self, database: &str) -> Result<Vec<ViewInfo>> {
        self.inner.views(database)
    }
    fn dedicated(&self) -> Result<Box<dyn Connection>> {
        // Same tunnel, its own session behind it.
        Ok(Box::new(Tunneled { inner: self.inner.dedicated()?, _tunnel: self._tunnel.clone() }))
    }
    fn execute_atomic(&self, database: Option<&str>, stmts: &[(String, Option<u64>)]) -> Result<Vec<u64>> {
        self.inner.execute_atomic(database, stmts)
    }
    fn query(&self, database: Option<&str>, sql: &str) -> Result<ResultSet> {
        self.inner.query(database, sql)
    }
}
