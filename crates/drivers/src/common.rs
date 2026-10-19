use std::cell::RefCell;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use dbcraft_core::{Error, Result, ResultSet, Value};
use tokio::runtime::Runtime;
use tokio::sync::Notify;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a cancelled statement gets to stop on its own before its connection is dropped.
const CANCEL_GRACE: Duration = Duration::from_secs(3);

static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();

fn runtime() -> Result<&'static Runtime> {
    let rt = RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string()));
    rt.as_ref().map_err(|e| Error::Database(format!("cannot start the async runtime: {e}")))
}

/// Server-side "stop that query" action, registered by a driver while a statement runs.
type KillFn = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct CancelInner {
    cancelled: AtomicBool,
    /// A server-side stop was requested (worth waiting a moment for the statement to end).
    killed: AtomicBool,
    notify: Notify,
    kill: Mutex<Option<KillFn>>,
}

/// Stop button for driver calls made on one thread (see [`with_cancel`]). Cancelling makes the
/// running call return [`Error::Cancelled`] at once and asks the server to stop the statement
/// (PostgreSQL `pg_cancel_backend`, MySQL `KILL QUERY`); later calls fail before they start.
#[derive(Clone, Default)]
pub struct Cancel(Arc<CancelInner>);

impl Cancel {
    pub fn new() -> Self {
        Cancel::default()
    }

    /// Safe to call from any thread, any number of times; never blocks.
    pub fn cancel(&self) {
        if self.0.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        // Mark the server-side stop before waking the call: woken first, it would see no stop
        // coming, abandon the statement at once and leave it running on the server.
        let kill = self.0.kill.lock().unwrap_or_else(PoisonError::into_inner).take();
        if kill.is_some() {
            self.0.killed.store(true, Ordering::SeqCst);
        }
        self.0.notify.notify_waiters();
        if let Some(kill) = kill {
            kill();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    async fn cancelled(&self) {
        // Register before checking the flag so a cancel between the two isn't missed.
        let notified = self.0.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}

/// Run `f` with `cancel` as the Stop button for every driver call `f` makes on this thread.
pub fn with_cancel<R>(cancel: &Cancel, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<Cancel>);
    impl Drop for Reset {
        fn drop(&mut self) {
            let prev = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = prev);
        }
    }
    let _reset = Reset(CURRENT.with(|c| c.borrow_mut().replace(cancel.clone())));
    f()
}

/// The Stop button for this thread's driver calls, if any.
pub fn current_cancel() -> Option<Cancel> {
    CURRENT.with(|c| c.borrow().clone())
}

/// While the returned guard lives, cancelling this thread's calls also runs `kill` (which must
/// not block: spawn the server round trip with [`spawn`]).
pub fn on_cancel(kill: impl FnOnce() + Send + 'static) -> KillGuard {
    let cancel = current_cancel();
    if let Some(c) = &cancel {
        *c.0.kill.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(kill));
    }
    KillGuard(cancel)
}

pub struct KillGuard(Option<Cancel>);

impl Drop for KillGuard {
    fn drop(&mut self) {
        if let Some(c) = &self.0 {
            c.0.kill.lock().unwrap_or_else(PoisonError::into_inner).take();
        }
    }
}

/// Marks a statement as finished when dropped (it completed, failed or was abandoned).
pub struct DoneGuard(pub Arc<AtomicBool>);

impl Drop for DoneGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Send a server-side stop, again every 300 ms, until the statement has ended (Stop may land
/// before the statement reaches the server, when a cancel finds the session idle and does
/// nothing). Gives up after ~3 s.
pub fn repeat_kill<F, Fut>(done: Arc<AtomicBool>, attempt: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    spawn(async move {
        for _ in 0..10 {
            if done.load(Ordering::SeqCst) {
                return;
            }
            attempt().await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
}

/// Fire-and-forget a future on the shared runtime (server-side cancel requests).
pub fn spawn(f: impl Future<Output = ()> + Send + 'static) {
    if let Ok(rt) = runtime() {
        rt.spawn(f);
    }
}

/// Run a future to completion on the shared runtime. Must not be called from inside an async
/// context (the engine is synchronous, so it never is). Honours this thread's [`Cancel`].
pub fn block_on<F: Future>(f: F) -> Result<F::Output> {
    let rt = runtime()?;
    match current_cancel() {
        None => Ok(rt.block_on(f)),
        Some(c) if c.is_cancelled() => Err(Error::Cancelled),
        Some(c) => rt.block_on(async {
            tokio::pin!(f);
            tokio::select! {
                biased;
                // A statement the server stopped fails with its own error; report the Stop instead.
                out = &mut f => return if c.is_cancelled() { Err(Error::Cancelled) } else { Ok(out) },
                () = c.cancelled() => {}
            }
            // If the server was asked to stop the statement, let it fail cleanly for a moment so
            // the connection goes back to the pool in a known state; otherwise drop it now.
            if c.0.killed.load(Ordering::SeqCst) {
                let _ = tokio::time::timeout(CANCEL_GRACE, &mut f).await;
            }
            Err(Error::Cancelled)
        }),
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

/// The `SELECT` of a `CREATE VIEW name [(cols)] AS select` statement (SQLite keeps only the
/// whole statement).
pub fn view_select(create: &str) -> Option<String> {
    let lower = create.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut from = lower.find("view")?;
    while let Some(rel) = lower.get(from..)?.find("as") {
        let at = from + rel;
        let before = at.checked_sub(1).and_then(|i| bytes.get(i)).copied().unwrap_or(b' ');
        let after = bytes.get(at + 2).copied().unwrap_or(b' ');
        let edge = |b: u8| b.is_ascii_whitespace() || b == b'(' || b == b')' || b == b'"' || b == b'`' || b == b']';
        if edge(before) && (after.is_ascii_whitespace() || after == b'(') {
            return create.get(at + 2..).map(|s| s.trim().trim_end_matches(';').trim().to_string());
        }
        from = at + 2;
    }
    None
}

/// Split a `chr(31)`-joined list from a catalog query.
pub fn split_list(s: Option<String>) -> Vec<String> {
    s.map(|s| s.split('\u{1f}').filter(|p| !p.is_empty()).map(str::to_string).collect()).unwrap_or_default()
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
    use super::*;

    #[test]
    fn cancel_interrupts_block_on() {
        let c = Cancel::new();
        let c2 = c.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            c2.cancel();
        });
        let start = std::time::Instant::now();
        let r = with_cancel(&c, || block_on(async { tokio::time::sleep(Duration::from_secs(30)).await }));
        t.join().unwrap();
        assert!(matches!(r, Err(Error::Cancelled)));
        assert!(start.elapsed() < Duration::from_secs(1), "no server-side stop pending: no grace wait");
        // Later calls fail before they start; other threads are unaffected.
        assert!(matches!(with_cancel(&c, || block_on(async { 1 })), Err(Error::Cancelled)));
        assert_eq!(block_on(async { 2 }).unwrap(), 2);
    }

    #[test]
    fn kill_runs_once_while_registered() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = Cancel::new();
        with_cancel(&c, || {
            let h = hits.clone();
            let _g = on_cancel(move || {
                h.fetch_add(1, Ordering::SeqCst);
            });
            c.cancel();
            c.cancel();
        });
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // After the guard drops, nothing is registered.
        let c = Cancel::new();
        with_cancel(&c, || drop(on_cancel(|| panic!("must not run"))));
        c.cancel();
        assert!(current_cancel().is_none());
    }

    #[test]
    fn finds_the_select_of_a_view() {
        assert_eq!(view_select("CREATE VIEW v AS select 1").as_deref(), Some("select 1"));
        assert_eq!(view_select("create view \"has as\" (a, b) as\n  select x as a, y from t;").as_deref(), Some("select x as a, y from t"));
        assert_eq!(view_select("CREATE VIEW [v]AS(select 1)").as_deref(), Some("(select 1)"));
        assert_eq!(view_select("CREATE VIEW basket AS SELECT 1").as_deref(), Some("SELECT 1"));
        assert_eq!(view_select("nonsense"), None);
    }

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
