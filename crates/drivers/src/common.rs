use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use dbcraft_core::{Error, Result, ResultSet, Value};
use tokio::runtime::Runtime;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();

/// Run a future to completion on the shared runtime. Must not be called from inside an async
/// context (the engine is synchronous, so it never is).
pub fn block_on<F: Future>(f: F) -> Result<F::Output> {
    let rt = RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string()));
    match rt {
        Ok(rt) => Ok(rt.block_on(f)),
        Err(e) => Err(Error::Database(format!("cannot start the async runtime: {e}"))),
    }
}

pub fn db_err(e: sqlx::Error) -> Error {
    Error::Database(e.to_string())
}

/// Make one direct connection before building a pool. Pools retry refused connections until their
/// acquire timeout; a single connect fails at once with the real cause (refused, bad password…).
pub fn probe<C: sqlx::Connection>(opts: &C::Options) -> Result<()> {
    block_on(async {
        match tokio::time::timeout(CONNECT_TIMEOUT, C::connect_with(opts)).await {
            Err(_) => Err(Error::Database(format!("timed out after {}s", CONNECT_TIMEOUT.as_secs()))),
            Ok(Err(e)) => Err(db_err(e)),
            Ok(Ok(conn)) => {
                let _ = conn.close().await;
                Ok(())
            }
        }
    })?
}

/// A cell as text (`None` for NULL or out of range). Catalog columns come back as text, numbers or
/// binary depending on engine and version, so metadata code reads them through these helpers.
pub fn cell_str(rs: &ResultSet, row: usize, col: usize) -> Option<String> {
    match rs.rows.get(row)?.get(col)? {
        Value::Null => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        v => Some(v.display()),
    }
}

pub fn cell_u64(rs: &ResultSet, row: usize, col: usize) -> Option<u64> {
    match rs.rows.get(row)?.get(col)? {
        Value::Int(i) => u64::try_from(*i).ok(),
        Value::UInt(u) => Some(*u),
        Value::Float(f) if f.is_finite() && *f >= 0.0 => Some(*f as u64),
        Value::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn cell_bool(rs: &ResultSet, row: usize, col: usize) -> bool {
    match rs.rows.get(row).and_then(|r| r.get(col)) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Int(i)) => *i != 0,
        Some(Value::Text(s)) => matches!(s.as_str(), "t" | "true" | "1" | "YES"),
        _ => false,
    }
}

/// Whether a statement yields rows (prepared + fetch) or only a count (raw execute).
pub fn returns_rows(sql: &str) -> bool {
    let mut s = sql.trim_start();
    // Skip leading comments and parentheses.
    loop {
        if let Some(rest) = s.strip_prefix("--").or_else(|| s.strip_prefix('#')) {
            s = rest.split_once('\n').map(|(_, r)| r).unwrap_or("").trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map(|(_, r)| r).unwrap_or("").trim_start();
        } else if let Some(rest) = s.strip_prefix('(') {
            s = rest.trim_start();
        } else {
            break;
        }
    }
    let first: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
    matches!(first.as_str(), "select" | "with" | "show" | "pragma" | "explain" | "values" | "describe" | "desc" | "table" | "call")
        || sql.to_ascii_lowercase().contains("returning")
}

/// Decode every cell of a row by trying candidate Rust types in order (sqlx checks type
/// compatibility, so the first that fits wins). Unknown types show as `<TYPE>`.
macro_rules! decode_row {
    ($row:expr, $( $t:ty => $f:expr ),* $(,)?) => {{
        use sqlx::{Column as _, Row as _, TypeInfo as _};
        let row = $row;
        (0..row.columns().len())
            .map(|i| -> dbcraft_core::Value {
                $(
                    if let Ok(v) = row.try_get::<Option<$t>, usize>(i) {
                        return match v { Some(x) => ($f)(x), None => dbcraft_core::Value::Null };
                    }
                )*
                dbcraft_core::Value::Text(format!("<{}>", row.columns().get(i).map(|c| c.type_info().name().to_string()).unwrap_or_default()))
            })
            .collect::<Vec<_>>()
    }};
}
pub(crate) use decode_row;

/// Column headers of a row.
macro_rules! columns_of {
    ($row:expr) => {{
        use sqlx::{Column as _, Row as _, TypeInfo as _};
        $row.columns().iter().map(|c| dbcraft_core::Column { name: c.name().to_string(), type_name: c.type_info().name().to_string() }).collect::<Vec<_>>()
    }};
}
pub(crate) use columns_of;

/// Error for a statement in an atomic batch that hit the wrong number of rows.
pub fn count_mismatch(index: usize, sql: &str, got: u64, want: u64) -> Error {
    Error::Database(format!(
        "statement {} affected {got} rows instead of {want}; nothing was saved.
{sql}",
        index + 1
    ))
}

/// The body of `execute_atomic` for any sqlx transaction: run, check counts, commit.
macro_rules! run_atomic {
    ($tx:expr, $stmts:expr) => {{
        let mut tx = $tx;
        let mut counts = Vec::with_capacity($stmts.len());
        for (i, (sql, want)) in $stmts.iter().enumerate() {
            let got = sqlx::raw_sql(sqlx::AssertSqlSafe(sql.clone())).execute(&mut *tx).await.map_err(crate::common::db_err)?.rows_affected();
            if let Some(want) = want
                && got != *want
            {
                return Err(crate::common::count_mismatch(i, sql, got, *want)); // tx drops: rollback
            }
            counts.push(got);
        }
        tx.commit().await.map_err(crate::common::db_err)?;
        Ok::<_, dbcraft_core::Error>(counts)
    }};
}
pub(crate) use run_atomic;

#[cfg(test)]
mod tests {
    use super::returns_rows;

    #[test]
    fn classifies_statements() {
        assert!(returns_rows("SELECT 1"));
        assert!(returns_rows("  -- hi\n with x as (select 1) select * from x"));
        assert!(returns_rows("/* c */ (select 1)"));
        assert!(returns_rows("insert into t values (1) returning id"));
        assert!(!returns_rows("insert into t values (1)"));
        assert!(!returns_rows("create table t (a int)"));
        assert!(!returns_rows(""));
        assert!(!returns_rows("/* unterminated"));
    }
}
