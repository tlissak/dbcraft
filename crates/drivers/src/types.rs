//! Native type text ⇄ [`GenericType`], so a table can be recreated on another engine
//! (Data Transfer, backups). Pure functions; unknown types fall back to text.

use dbcraft_core::GenericType as G;
use dbcraft_sql::Dialect;

/// Numbers in `name(a, b)`.
fn args(t: &str) -> Vec<u32> {
    let Some(start) = t.find('(') else { return Vec::new() };
    let end = t[start..].find(')').map_or(t.len(), |e| start + e);
    t.get(start + 1..end).unwrap_or("").split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

fn decimal(t: &str) -> G {
    let a = args(t);
    G::Decimal { precision: a.first().copied().unwrap_or(18).clamp(1, 65), scale: a.get(1).copied().unwrap_or(0).min(30) }
}

fn varchar(t: &str) -> G {
    match args(t).first() {
        Some(&len) if len > 0 && len <= 65_535 => G::Varchar { len },
        _ => G::Text,
    }
}

/// SQLite declared type (affinity rules from the SQLite docs, plus common names).
pub fn from_sqlite(decl: &str) -> G {
    let t = decl.to_ascii_lowercase();
    if t.is_empty() || t.contains("blob") {
        G::Blob
    } else if t.contains("bool") {
        G::Bool
    } else if t.contains("int") {
        G::BigInt
    } else if t.contains("char") || t.contains("clob") || t.contains("text") {
        if t.contains('(') { varchar(&t) } else { G::Text }
    } else if t.contains("real") || t.contains("floa") || t.contains("doub") {
        G::Double
    } else if t.contains("datetime") || t.contains("timestamp") {
        G::DateTime
    } else if t.contains("date") {
        G::Date
    } else if t.contains("time") {
        G::Time
    } else if t.contains("json") {
        G::Json
    } else if t.contains("uuid") {
        G::Uuid
    } else if t.contains("dec") || t.contains("num") {
        decimal(&t)
    } else {
        G::Text
    }
}

/// PostgreSQL `format_type()` text.
pub fn from_postgres(t: &str) -> G {
    let t = t.to_ascii_lowercase();
    let base = t.split('(').next().unwrap_or("").trim();
    if t.ends_with("[]") {
        return G::Text;
    }
    match base {
        "smallint" => G::SmallInt,
        "integer" => G::Integer,
        "bigint" => G::BigInt,
        "numeric" | "decimal" => decimal(&t),
        "real" => G::Float,
        "double precision" => G::Double,
        "boolean" => G::Bool,
        "character varying" | "character" => varchar(&t),
        "text" | "name" | "citext" => G::Text,
        "bytea" => G::Blob,
        "date" => G::Date,
        "json" | "jsonb" => G::Json,
        "uuid" => G::Uuid,
        _ if base.starts_with("timestamp") => G::DateTime,
        _ if base.starts_with("time") => G::Time,
        _ => G::Text,
    }
}

/// MySQL / MariaDB `COLUMN_TYPE`.
pub fn from_mysql(t: &str) -> G {
    let t = t.to_ascii_lowercase();
    let base = t.split(['(', ' ']).next().unwrap_or("");
    match base {
        "tinyint" if t.starts_with("tinyint(1)") => G::Bool,
        "bool" | "boolean" => G::Bool,
        "tinyint" | "smallint" => G::SmallInt,
        "mediumint" | "int" | "integer" | "year" => {
            if t.contains("unsigned") && base != "year" {
                G::BigInt
            } else {
                G::Integer
            }
        }
        "bigint" => G::BigInt,
        "decimal" | "numeric" => decimal(&t),
        "float" => G::Float,
        "double" | "real" => G::Double,
        "char" | "varchar" => varchar(&t),
        "enum" | "set" => G::Varchar { len: 255 },
        "tinytext" | "text" | "mediumtext" | "longtext" => G::Text,
        "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" | "bit" => G::Blob,
        "date" => G::Date,
        "time" => G::Time,
        "datetime" | "timestamp" => G::DateTime,
        "json" => G::Json,
        _ => G::Text,
    }
}

/// The type to declare on `dialect`. `in_key`: MySQL can't index unbounded text, so key columns
/// get `VARCHAR(255)` there.
pub fn render(dialect: Dialect, g: G, in_key: bool) -> String {
    match dialect {
        Dialect::Sqlite => match g {
            G::SmallInt | G::Integer | G::BigInt | G::Bool => "INTEGER".into(),
            G::Decimal { precision, scale } => format!("NUMERIC({precision},{scale})"),
            G::Float | G::Double => "REAL".into(),
            G::Varchar { len } => format!("VARCHAR({len})"),
            G::Text | G::Json | G::Uuid => "TEXT".into(),
            G::Blob => "BLOB".into(),
            G::Date => "DATE".into(),
            G::Time => "TIME".into(),
            G::DateTime => "DATETIME".into(),
        },
        Dialect::Postgres => match g {
            G::SmallInt => "SMALLINT".into(),
            G::Integer => "INTEGER".into(),
            G::BigInt => "BIGINT".into(),
            G::Decimal { precision, scale } => format!("NUMERIC({},{scale})", precision.min(1000)),
            G::Float => "REAL".into(),
            G::Double => "DOUBLE PRECISION".into(),
            G::Bool => "BOOLEAN".into(),
            G::Varchar { len } => format!("VARCHAR({len})"),
            G::Text => "TEXT".into(),
            G::Blob => "BYTEA".into(),
            G::Date => "DATE".into(),
            G::Time => "TIME".into(),
            G::DateTime => "TIMESTAMP".into(),
            G::Json => "JSONB".into(),
            G::Uuid => "UUID".into(),
        },
        Dialect::Mysql => match g {
            G::SmallInt => "SMALLINT".into(),
            G::Integer => "INT".into(),
            G::BigInt => "BIGINT".into(),
            G::Decimal { precision, scale } => format!("DECIMAL({},{})", precision.min(65), scale.min(30)),
            G::Float => "FLOAT".into(),
            G::Double => "DOUBLE".into(),
            G::Bool => "TINYINT(1)".into(),
            G::Varchar { len } => format!("VARCHAR({})", len.min(16_383)),
            G::Text if in_key => "VARCHAR(255)".into(),
            G::Text => "LONGTEXT".into(),
            G::Blob if in_key => "VARBINARY(255)".into(),
            G::Blob => "LONGBLOB".into(),
            G::Date => "DATE".into(),
            G::Time => "TIME".into(),
            G::DateTime => "DATETIME(6)".into(),
            G::Json => "JSON".into(),
            G::Uuid => "CHAR(36)".into(),
        },
        Dialect::Mssql => match g {
            G::SmallInt => "SMALLINT".into(),
            G::Integer => "INT".into(),
            G::BigInt => "BIGINT".into(),
            G::Decimal { precision, scale } => format!("DECIMAL({},{})", precision.min(38), scale.min(38)),
            G::Float => "REAL".into(),
            G::Double => "FLOAT".into(),
            G::Bool => "BIT".into(),
            G::Varchar { len } if len <= 4000 => format!("NVARCHAR({len})"),
            G::Varchar { .. } | G::Text | G::Json => if in_key { "NVARCHAR(450)" } else { "NVARCHAR(MAX)" }.into(),
            G::Blob => "VARBINARY(MAX)".into(),
            G::Date => "DATE".into(),
            G::Time => "TIME".into(),
            G::DateTime => "DATETIME2".into(),
            G::Uuid => "UNIQUEIDENTIFIER".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_native_types() {
        assert_eq!(from_mysql("tinyint(1)"), G::Bool);
        assert_eq!(from_mysql("int unsigned"), G::BigInt);
        assert_eq!(from_mysql("decimal(3,1)"), G::Decimal { precision: 3, scale: 1 });
        assert_eq!(from_mysql("varchar(45)"), G::Varchar { len: 45 });
        assert_eq!(from_mysql("enum('a','b')"), G::Varchar { len: 255 });
        assert_eq!(from_postgres("character varying(45)"), G::Varchar { len: 45 });
        assert_eq!(from_postgres("timestamp with time zone"), G::DateTime);
        assert_eq!(from_postgres("time without time zone"), G::Time);
        assert_eq!(from_postgres("numeric"), G::Decimal { precision: 18, scale: 0 });
        assert_eq!(from_postgres("integer[]"), G::Text);
        assert_eq!(from_sqlite("INTEGER"), G::BigInt);
        assert_eq!(from_sqlite("VARCHAR(20)"), G::Varchar { len: 20 });
        assert_eq!(from_sqlite(""), G::Blob);
        assert_eq!(from_sqlite("DATETIME"), G::DateTime);
        assert_eq!(from_sqlite("weird"), G::Text);
    }

    #[test]
    fn renders_per_dialect_and_survives_junk() {
        assert_eq!(render(Dialect::Mysql, G::Text, true), "VARCHAR(255)");
        assert_eq!(render(Dialect::Postgres, G::Json, false), "JSONB");
        assert_eq!(render(Dialect::Sqlite, G::Bool, false), "INTEGER");
        for junk in ["decimal(", "varchar(99999999999)", "numeric(,)", "(", ")("] {
            let _ = (from_mysql(junk), from_postgres(junk), from_sqlite(junk));
        }
    }
}
