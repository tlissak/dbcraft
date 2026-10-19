//! Background tasks (ROADMAP 1.1): a worker session shares the live connections, and its results
//! flow back into the app's session without resurrecting connections the user closed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use serde_json::json;

fn session(name: &str) -> Session {
    let d = std::env::temp_dir().join(format!("dbcraft-bg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    for id in ["a", "b"] {
        s.dispatch("connection.save", &json!({"id": id, "kind": "sqlite", "name": id, "database": d.join(format!("{id}.db")).to_string_lossy()})).unwrap();
    }
    s
}

#[test]
fn worker_shares_connections_and_hands_results_back() {
    let mut s = session("share");
    s.dispatch("connection.open", &json!({"id": "a"})).unwrap();
    s.dispatch("query.run", &json!({"connection": "a", "sql": "create table t (x int); insert into t values (1), (2)"})).unwrap();

    // The worker uses the open connection (no reconnect) and opens `b` itself.
    let mut w = s.fork_shared();
    let handle = std::thread::spawn(move || {
        w.dispatch("query.run", &json!({"connection": "a", "sql": "select count(*) as n from t"})).unwrap();
        w.dispatch("connection.open", &json!({"id": "b"})).unwrap();
        w.dispatch("database.objects", &json!({"connection": "a", "database": "main"})).unwrap();
        w
    });
    let w = handle.join().unwrap();
    assert!(!s.is_open("b"));
    s.absorb(w);
    assert!(s.is_open("b"), "connection opened by the worker is adopted");
    assert!(s.objects.contains_key(&("a".to_string(), "main".to_string())));
    assert_eq!(s.last_result.as_ref().unwrap().rows[0][0], dbcraft_core::Value::Int(2));
    assert!(s.messages.iter().any(|m| m.contains("select count(*)")));
}

#[test]
fn closed_meanwhile_stays_closed() {
    let mut s = session("closed");
    s.dispatch("connection.open", &json!({"id": "a"})).unwrap();
    let mut w = s.fork_shared();
    w.dispatch("database.objects", &json!({"connection": "a", "database": "main"})).unwrap();
    // The user closes `a` while the worker runs.
    s.dispatch("connection.close", &json!({"id": "a"})).unwrap();
    s.absorb(w);
    assert!(!s.is_open("a"));
    assert!(s.objects.is_empty(), "no cache for a closed connection");
}

#[test]
fn stop_cancels_a_running_command() {
    let mut s = session("stop");
    s.dispatch("connection.open", &json!({"id": "a"})).unwrap();
    let mut w = s.fork_shared();
    let cancel = dbcraft_engine::Cancel::new();
    let k = cancel.clone();
    let handle = std::thread::spawn(move || {
        dbcraft_engine::with_cancel(&k, || {
            w.dispatch(
                "query.run",
                &json!({"connection": "a", "sql": "with recursive n(x) as (select 1 union all select x + 1 from n) select count(*) from n"}),
            )
        })
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    cancel.cancel();
    let r = handle.join().unwrap();
    assert!(matches!(r, Err(dbcraft_core::Error::Cancelled)), "{r:?}");
    // The app's session can keep using the connection.
    assert!(s.dispatch("query.run", &json!({"connection": "a", "sql": "select 1"})).is_ok());
}
