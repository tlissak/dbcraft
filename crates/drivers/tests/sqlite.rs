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
    let c = dbcraft_drivers::connect(&cfg).unwrap();
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
    let c = dbcraft_drivers::connect(&cfg).unwrap();
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

/// Stop (ROADMAP 1.1): an endless query returns `Cancelled` quickly, SQLite really stops it (the
/// connection is usable right after), and a later query isn't interrupted by the old handler.
#[test]
fn stop_interrupts_an_endless_query() {
    let cfg = ConnectionConfig::sqlite("t", "t", temp_db("stop"));
    let c = std::sync::Arc::<dyn dbcraft_drivers::Connection>::from(dbcraft_drivers::connect(&cfg).unwrap());
    let cancel = dbcraft_drivers::Cancel::new();
    let (c2, k2) = (c.clone(), cancel.clone());
    let worker = std::thread::spawn(move || {
        dbcraft_drivers::with_cancel(&k2, || c2.query(None, "with recursive n(x) as (select 1 union all select x + 1 from n) select count(*) from n"))
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    let start = std::time::Instant::now();
    cancel.cancel();
    let r = worker.join().unwrap();
    assert!(matches!(r, Err(dbcraft_core::Error::Cancelled)), "{r:?}");
    assert!(start.elapsed() < std::time::Duration::from_secs(2), "stopped in {:?}", start.elapsed());
    // Every pooled connection still works (none left interrupted or busy).
    for _ in 0..8 {
        assert_eq!(c.query(None, "select 41 + 1").unwrap().rows[0][0], Value::Int(42));
    }
    // A new Stop button works on the next query.
    let fresh = dbcraft_drivers::Cancel::new();
    let rs = dbcraft_drivers::with_cancel(&fresh, || c.query(None, "select 7")).unwrap();
    assert_eq!(rs.rows[0][0], Value::Int(7));
}

#[test]
fn table_schema_and_views() {
    let cfg = ConnectionConfig::sqlite("t", "t", temp_db("schema"));
    let c = dbcraft_drivers::connect(&cfg).unwrap();
    for sql in [
        "create table studio (id integer primary key, name text not null unique)",
        "create table film (id integer primary key autoincrement, title text not null default 'untitled', added text default current_timestamp, \
         studio_id int references studio on delete cascade, code text)",
        "create index film_title on film (title, id)",
        "create index film_lower on film (lower(code))",
        "create view titles as select title from film",
    ] {
        c.query(None, sql).unwrap();
    }
    let film = c.table_schema(None, None, "film").unwrap();
    let id = &film.columns[0];
    assert!(id.auto_increment && id.pk == Some(1));
    assert_eq!(film.columns[1].default.as_deref(), Some("'untitled'"));
    assert_eq!(film.columns[2].default.as_deref().map(str::to_ascii_lowercase).as_deref(), Some("current_timestamp"));
    let plain = film.indexes.iter().find(|i| i.name == "film_title").unwrap();
    assert_eq!((plain.columns.as_slice(), plain.unique, plain.native_ddl.is_none()), (&["title".to_string(), "id".to_string()][..], false, true));
    let expr = film.indexes.iter().find(|i| i.name == "film_lower").unwrap();
    assert!(expr.native_ddl.as_deref().unwrap_or("").contains("lower(code)"), "{expr:?}");
    let fk = &film.foreign_keys[0];
    assert_eq!((fk.ref_table.as_str(), fk.columns.as_slice(), fk.ref_columns.as_slice()), ("studio", &["studio_id".to_string()][..], &["id".to_string()][..]));
    assert_eq!(fk.on_delete.as_deref(), Some("CASCADE"));
    // UNIQUE constraint: an index with a name that can be created again.
    let studio = c.table_schema(None, None, "studio").unwrap();
    assert!(studio.indexes.iter().any(|i| i.unique && i.columns == ["name"] && !i.name.starts_with("sqlite_")), "{:?}", studio.indexes);
    let views = c.views("main").unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].definition, "select title from film");
}
