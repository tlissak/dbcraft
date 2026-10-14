//! End-to-end SQLite tests (no server needed). Server engines are covered by `tests/servers.rs`,
//! which needs `docker compose -f dev/docker-compose.yml up -d`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_core::{ConnectionConfig, Value};
use dbcraft_model::ObjectKind;

fn temp_db(name: &str) -> String {
    let dir = std::env::temp_dir().join("dbcraft-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

#[test]
fn create_insert_select_and_list() {
    let cfg = ConnectionConfig::sqlite("t", "t", temp_db("crud"));
    let mut c = dbcraft_drivers::connect(&cfg).unwrap();
    assert!(c.server_version().unwrap().starts_with("SQLite"));

    c.query(None, "create table people (id integer primary key, name text, score real, pic blob)").unwrap();
    let ins = c.query(None, "insert into people (name, score, pic) values ('Ada', 9.5, x'0102'), ('Bob', null, null)").unwrap();
    assert_eq!(ins.rows_affected, 2);
    c.query(None, "create view v as select name from people").unwrap();

    let rs = c.query(None, "select id, name, score, pic from people order by id").unwrap();
    assert_eq!(rs.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "name", "score", "pic"]);
    assert_eq!(rs.rows[0], vec![Value::Int(1), Value::Text("Ada".into()), Value::Float(9.5), Value::Bytes(vec![1, 2])]);
    assert_eq!(rs.rows[1][2], Value::Null);

    assert_eq!(c.list_databases().unwrap(), ["main"]);
    let objs = c.list_objects("main").unwrap();
    assert!(objs.iter().any(|o| o.kind == ObjectKind::Table && o.name == "people"));
    assert!(objs.iter().any(|o| o.kind == ObjectKind::View && o.name == "v"));
}

#[test]
fn bad_sql_is_an_error_not_a_panic() {
    let cfg = ConnectionConfig::sqlite("t", "t", temp_db("bad"));
    let mut c = dbcraft_drivers::connect(&cfg).unwrap();
    assert!(c.query(None, "selec nonsense").is_err());
    assert!(c.query(None, "select * from missing_table").is_err());
    assert!(c.list_objects("no_such_db").is_err());
}

#[test]
fn empty_path_and_unsupported_engine_are_errors() {
    assert!(dbcraft_drivers::connect(&ConnectionConfig::sqlite("t", "t", "")).is_err());
    let mut cfg = ConnectionConfig::sqlite("t", "t", "x.db");
    cfg.kind = dbcraft_core::DbKind::Oracle;
    assert!(dbcraft_drivers::connect(&cfg).is_err());
}
