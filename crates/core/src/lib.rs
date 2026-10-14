//! L0 foundation shared by every DbCraft crate: cell values, result sets, connection profiles and
//! the common error type. Pure data, no I/O, compiles to wasm.

use serde::{Deserialize, Serialize};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid parameter: {0}")]
    InvalidParam(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("i/o error: {0}")]
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// The database engines DbCraft targets (Navicat Premium's list). Not all have a driver yet:
/// see `ROADMAP.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Sqlite,
    Postgres,
    Mysql,
    Mariadb,
    Sqlserver,
    Oracle,
    Mongodb,
    Redis,
}

impl DbKind {
    pub const ALL: [DbKind; 8] =
        [DbKind::Mysql, DbKind::Mariadb, DbKind::Postgres, DbKind::Oracle, DbKind::Sqlite, DbKind::Sqlserver, DbKind::Mongodb, DbKind::Redis];

    pub fn id(self) -> &'static str {
        match self {
            DbKind::Sqlite => "sqlite",
            DbKind::Postgres => "postgres",
            DbKind::Mysql => "mysql",
            DbKind::Mariadb => "mariadb",
            DbKind::Sqlserver => "sqlserver",
            DbKind::Oracle => "oracle",
            DbKind::Mongodb => "mongodb",
            DbKind::Redis => "redis",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DbKind::Sqlite => "SQLite",
            DbKind::Postgres => "PostgreSQL",
            DbKind::Mysql => "MySQL",
            DbKind::Mariadb => "MariaDB",
            DbKind::Sqlserver => "SQL Server",
            DbKind::Oracle => "Oracle",
            DbKind::Mongodb => "MongoDB",
            DbKind::Redis => "Redis",
        }
    }

    pub fn default_port(self) -> Option<u16> {
        match self {
            DbKind::Sqlite => None,
            DbKind::Postgres => Some(5432),
            DbKind::Mysql | DbKind::Mariadb => Some(3306),
            DbKind::Sqlserver => Some(1433),
            DbKind::Oracle => Some(1521),
            DbKind::Mongodb => Some(27017),
            DbKind::Redis => Some(6379),
        }
    }

    /// Accepts the machine id (`postgres`) or the label (`PostgreSQL`), case-insensitively.
    pub fn parse(s: &str) -> Option<DbKind> {
        DbKind::ALL.into_iter().find(|k| k.id().eq_ignore_ascii_case(s) || k.label().eq_ignore_ascii_case(s))
    }
}

/// A saved connection (Navicat: File > New Connection).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    /// Plain text for now; moving secrets to the OS keychain is a tracked gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Initial database (server engines) or the file path (SQLite).
    #[serde(default)]
    pub database: String,
    /// Reach the server through an SSH tunnel (Navicat: New Connection › SSH).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshConfig>,
}

impl ConnectionConfig {
    pub fn sqlite(id: impl Into<String>, name: impl Into<String>, path: impl Into<String>) -> Self {
        ConnectionConfig {
            id: id.into(),
            name: name.into(),
            kind: DbKind::Sqlite,
            host: String::new(),
            port: None,
            user: String::new(),
            password: None,
            database: path.into(),
            ssh: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SshAuth {
    #[default]
    Password,
    PublicKey,
}

/// SSH tunnel settings. The database host/port are resolved from the SSH server's side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub auth: SshAuth,
    /// Password (password auth) or key passphrase (public key auth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Private key file (public key auth).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

/// One row of the Objects tab table list (Navicat shows these columns for tables).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// `table` or `view`.
    pub kind: String,
    /// Exact for SQLite, the engine's estimate for PostgreSQL and MySQL (as Navicat shows).
    pub rows: Option<u64>,
    /// Bytes of table data (MySQL/MariaDB data length, PostgreSQL total relation size).
    pub data_length: Option<u64>,
    pub engine: Option<String>,
    pub owner: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub collation: Option<String>,
    pub comment: Option<String>,
}

/// One cell of a result grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

impl Value {
    /// How the grid shows the value (Navicat shows `(NULL)` and `(BLOB) n bytes`).
    pub fn display(&self) -> String {
        match self {
            Value::Null => "(NULL)".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Text(s) => s.clone(),
            Value::Bytes(b) => format!("(BLOB) {} bytes", b.len()),
        }
    }

    /// The value as a CSV field (RFC 4180 quoting). NULL becomes an empty field.
    pub fn to_csv(&self) -> String {
        let s = match self {
            Value::Null => return String::new(),
            Value::Bytes(b) => b.iter().map(|x| format!("{x:02x}")).collect(),
            other => other.display(),
        };
        if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
    }
}

/// Engine-neutral column type, used to carry a table to another engine (Data Transfer, backups).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum GenericType {
    SmallInt,
    Integer,
    BigInt,
    Decimal {
        precision: u32,
        scale: u32,
    },
    Float,
    Double,
    Bool,
    /// Bounded string (`VARCHAR(n)`).
    Varchar {
        len: u32,
    },
    Text,
    Blob,
    Date,
    Time,
    DateTime,
    Json,
    Uuid,
}

/// A table column as the drivers describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    /// The engine's own type text (`varchar(45)`, `timestamp with time zone`, …).
    pub native_type: String,
    pub generic: GenericType,
    pub nullable: bool,
    /// Position in the primary key (1-based), if part of it.
    pub pk: Option<u32>,
}

/// A grant: privileges on one object (`*.*`, `db.*`, `db.table`; PostgreSQL `DATABASE x`, `TABLE s.t`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Grant {
    pub object: String,
    pub privileges: Vec<String>,
    #[serde(default)]
    pub grant_option: bool,
}

/// A user or role (Navicat: User / Role designers).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserInfo {
    pub name: String,
    /// MySQL/MariaDB account host (`%`, `localhost`…); `None` elsewhere.
    #[serde(default)]
    pub host: Option<String>,
    /// Roles have no login (PostgreSQL `NOLOGIN`, MySQL 8 roles).
    #[serde(default)]
    pub is_role: bool,
    /// Engine attributes shown as checkboxes (PostgreSQL `SUPERUSER`, `CREATEDB`…; MySQL `ACCOUNT LOCK`).
    #[serde(default)]
    pub attributes: std::collections::BTreeMap<String, bool>,
    /// Free-form settings (`plugin`, `connection_limit`, `valid_until`).
    #[serde(default)]
    pub settings: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub member_of: Vec<String>,
    #[serde(default)]
    pub grants: Vec<Grant>,
}

impl UserInfo {
    /// `'name'@'host'` (MySQL) or `name`, for messages and ids.
    pub fn display_name(&self) -> String {
        match &self.host {
            Some(h) => format!("{}@{h}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// The engine's own type name (e.g. `INT4`, `VARCHAR`).
    pub type_name: String,
}

/// The outcome of one statement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    /// Rows changed, for statements that don't return rows.
    pub rows_affected: u64,
    pub elapsed_ms: u64,
}

impl ResultSet {
    /// A boxed text table like the mysql/psql command-line clients print (Console).
    pub fn to_text_table(&self) -> String {
        if self.columns.is_empty() {
            return format!("Query OK, {} rows affected ({:.3} sec)", self.rows_affected, self.elapsed_ms as f64 / 1000.0);
        }
        let cells: Vec<Vec<String>> = self.rows.iter().map(|r| r.iter().map(|v| v.display().replace(['\n', '\r'], " ")).collect()).collect();
        let mut widths: Vec<usize> = self.columns.iter().map(|c| c.name.chars().count()).collect();
        for row in &cells {
            for (w, c) in widths.iter_mut().zip(row) {
                *w = (*w).max(c.chars().count()).min(80);
            }
        }
        let line = format!("+{}+", widths.iter().map(|w| "-".repeat(w + 2)).collect::<Vec<_>>().join("+"));
        let fmt_row = |vals: Vec<&str>| {
            let parts: Vec<String> = vals
                .iter()
                .zip(&widths)
                .map(|(v, w)| {
                    let v: String = v.chars().take(*w).collect();
                    format!(" {v}{} ", " ".repeat(w.saturating_sub(v.chars().count())))
                })
                .collect();
            format!("|{}|", parts.join("|"))
        };
        let mut out = vec![line.clone(), fmt_row(self.columns.iter().map(|c| c.name.as_str()).collect()), line.clone()];
        for row in &cells {
            out.push(fmt_row(row.iter().map(String::as_str).collect()));
        }
        out.push(line);
        let n = self.rows.len();
        out.push(format!("{n} row{} in set ({:.3} sec)", if n == 1 { "" } else { "s" }, self.elapsed_ms as f64 / 1000.0));
        out.join("\n")
    }

    pub fn to_csv(&self) -> String {
        let mut out = self.columns.iter().map(|c| Value::Text(c.name.clone()).to_csv()).collect::<Vec<_>>().join(",");
        out.push_str("\r\n");
        for row in &self.rows {
            out.push_str(&row.iter().map(Value::to_csv).collect::<Vec<_>>().join(","));
            out.push_str("\r\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(Value::Text("a,b".into()).to_csv(), "\"a,b\"");
        assert_eq!(Value::Text("say \"hi\"".into()).to_csv(), "\"say \"\"hi\"\"\"");
        assert_eq!(Value::Int(3).to_csv(), "3");
        assert_eq!(Value::Null.to_csv(), "");
    }

    #[test]
    fn text_table_like_the_cli() {
        let rs = ResultSet {
            columns: vec![Column { name: "id".into(), type_name: "INT".into() }, Column { name: "title".into(), type_name: "TEXT".into() }],
            rows: vec![vec![Value::Int(1), Value::Text("Metropolis".into())], vec![Value::Int(22), Value::Null]],
            rows_affected: 0,
            elapsed_ms: 12,
        };
        assert_eq!(
            rs.to_text_table(),
            "+----+------------+\n| id | title      |\n+----+------------+\n| 1  | Metropolis |\n| 22 | (NULL)     |\n+----+------------+\n2 rows in set (0.012 sec)"
        );
        let ok = ResultSet { rows_affected: 3, ..ResultSet::default() };
        assert_eq!(ok.to_text_table(), "Query OK, 3 rows affected (0.000 sec)");
    }

    #[test]
    fn kind_parse_accepts_ids_and_labels() {
        assert_eq!(DbKind::parse("postgres"), Some(DbKind::Postgres));
        assert_eq!(DbKind::parse("PostgreSQL"), Some(DbKind::Postgres));
        assert_eq!(DbKind::parse("nope"), None);
    }

    #[test]
    fn config_round_trips_through_json() {
        let c = ConnectionConfig::sqlite("c1", "Local", "test.db");
        let back: ConnectionConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
