//! Rule 9: every command returns `Err`, never panics, for any params or session state.
//! Fuzzes each command with adversarial params, with and without an open connection.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::{Session, commands};
use serde_json::{Value, json};

fn adversarial() -> Vec<Value> {
    vec![
        Value::Null,
        json!({}),
        json!([]),
        json!("str"),
        json!(42),
        json!({"id": 1, "connection": [], "database": {}, "sql": 7, "table": null}),
        json!({"id": "", "connection": "", "database": "", "sql": "", "table": ""}),
        json!({"id": "missing", "connection": "missing", "sql": "select 1", "table": "t", "kind": "sqlite", "name": " "}),
        json!({"kind": "nope", "name": "x", "port": 999999}),
        json!({"kind": "oracle", "name": "x", "port": -1}),
        json!({"connection": "c", "sql": ";;;--", "limit": u64::MAX, "offset": u64::MAX, "table": "\"]`'", "path": ""}),
        json!({"connection": "c", "sql": "selec ' unterminated", "database": "no_such_db"}),
    ]
}

#[test]
fn no_command_panics_on_bad_params() {
    let db = std::env::temp_dir().join(format!("dbcraft-panic-hunt-{}.db", std::process::id()));
    let mut s = Session::new();
    s.dispatch("connection.save", &json!({"id": "c", "kind": "sqlite", "name": "c", "database": db.to_string_lossy()})).unwrap();

    for state in ["closed", "open"] {
        if state == "open" {
            s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
        }
        for cmd in commands() {
            for p in adversarial() {
                // Destructive commands are exercised too; recreate the fixture after.
                let _ = s.dispatch(cmd.id, &p);
                if s.workspace.connection("c").is_err() {
                    s.dispatch("connection.save", &json!({"id": "c", "kind": "sqlite", "name": "c", "database": db.to_string_lossy()})).unwrap();
                }
                if state == "open" && !s.is_open("c") {
                    s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
                }
            }
        }
    }
}

#[test]
fn unknown_command_is_an_error() {
    assert!(Session::new().dispatch("no.such.command", &json!({})).is_err());
}

#[test]
fn sqlite_round_trip_through_commands() {
    let db = std::env::temp_dir().join(format!("dbcraft-engine-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let mut s = Session::new();
    let id = s.dispatch("connection.save", &json!({"kind": "sqlite", "name": "Local", "database": db.to_string_lossy()})).unwrap();
    let id = id["id"].as_str().unwrap().to_string();
    let opened = s.dispatch("connection.open", &json!({"id": id})).unwrap();
    assert_eq!(opened["databases"], json!(["main"]));

    let r = s
        .dispatch(
            "query.run",
            &json!({"connection": id, "sql": "create table t(a int, b text); insert into t values (1,'x'),(2,'y;z'); select * from t order by a"}),
        )
        .unwrap();
    assert_eq!(r["statements"], 3);
    assert_eq!(r["result"]["rows"].as_array().unwrap().len(), 2);

    let objs = s.dispatch("database.objects", &json!({"connection": id, "database": "main"})).unwrap();
    assert_eq!(objs[0]["name"], "t");

    let page = s.dispatch("table.open", &json!({"connection": id, "table": "t", "limit": 1})).unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 1);

    let csv = std::env::temp_dir().join(format!("dbcraft-engine-{}.csv", std::process::id()));
    s.dispatch("export.resultCsv", &json!({"path": csv.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), "a,b\r\n1,x\r\n");
}
