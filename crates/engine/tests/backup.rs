//! DbCraft backups (`.dbcb`): complete round trips, the Backup window's options, Extract SQL,
//! older `.sql` backups, damaged files, and speed against the older script format.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use std::path::PathBuf;
use std::time::Instant;

use dbcraft_core::Value;
use dbcraft_engine::Session;
use serde_json::json;

fn session(name: &str) -> (Session, PathBuf) {
    let d = std::env::temp_dir().join(format!("dbcraft-backup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.save", &json!({"id": "c", "kind": "sqlite", "name": "c", "database": d.join("c.db").to_string_lossy()})).unwrap();
    s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
    (s, d)
}

fn q(s: &mut Session, sql: &str) -> Vec<Vec<Value>> {
    let r = s.dispatch("query.run", &json!({"connection": "c", "sql": sql})).unwrap();
    serde_json::from_value::<dbcraft_core::ResultSet>(r["result"].clone()).unwrap().rows
}

fn n(s: &mut Session, sql: &str) -> i64 {
    match q(s, sql).first().and_then(|r| r.first()) {
        Some(Value::Int(n)) => *n,
        v => panic!("{v:?}"),
    }
}

const SCHEMA: &str = "
    create table studio (id integer primary key, name text not null unique);
    create table film (id integer primary key, title text not null default 'untitled', year int check (year > 1880),
                       studio_id int references studio(id) on delete cascade);
    create index film_year on film (year);
    create table log (msg text);
    create trigger film_log after insert on film begin insert into log values (new.title); end;
    create view titles as select title from film;
    insert into studio (name) values ('Ufa'), ('MGM');
    insert into film (title, year, studio_id) values ('Metropolis', 1927, 1), ('Freaks', 1932, 2), ('M', 1931, 1);";

#[test]
fn complete_round_trip() {
    let (mut s, d) = session("round");
    q(&mut s, SCHEMA);
    let b = s.dispatch("backup.create", &json!({"connection": "c", "database": "main", "comment": "nightly"})).unwrap();
    assert_eq!(
        (b["format"].as_str(), b["tables"].as_u64(), b["views"].as_u64(), b["rows"].as_u64(), b["complete"].as_bool()),
        (Some("dbcb"), Some(3), Some(1), Some(8), Some(true)),
        "{b}"
    );
    let path = b["path"].as_str().unwrap().to_string();
    assert!(path.ends_with(".dbcb"));

    // Wreck everything, then restore.
    q(&mut s, "drop view titles; drop trigger film_log; drop table film; delete from studio; create table film (x int)");
    let r = s.dispatch("backup.restore", &json!({"connection": "c", "database": "main", "path": path})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(n(&mut s, "select count(*) from film"), 3);
    assert_eq!(n(&mut s, "select count(*) from studio"), 2);
    assert_eq!(n(&mut s, "select count(*) from titles"), 3, "view restored");
    let sch = s.dispatch("table.schema", &json!({"connection": "c", "table": "film"})).unwrap();
    assert_eq!(sch["indexes"][0]["name"], "film_year");
    assert_eq!(sch["foreign_keys"][0]["on_delete"], "CASCADE");
    assert_eq!(sch["checks"].as_array().map(Vec::len), Some(1));
    assert_eq!(sch["triggers"][0]["name"], "film_log", "trigger restored (after the data, so it didn't fire)");
    assert_eq!(n(&mut s, "select count(*) from log"), 3, "the log kept its 3 rows, no duplicates from the trigger");
    assert!(s.dispatch("query.run", &json!({"connection": "c", "sql": "insert into film (title, year) values ('x', 1700)"})).is_err(), "check restored");
    let st = s.dispatch("table.schema", &json!({"connection": "c", "table": "studio"})).unwrap();
    assert_eq!(st["uniques"].as_array().map(Vec::len), Some(1), "unique restored");
    let _ = d;
}

#[test]
fn window_options_and_extract_sql() {
    let (mut s, d) = session("options");
    q(&mut s, SCHEMA);
    // Object selection + Use specified file name + single transaction.
    let b = s
        .dispatch(
            "backup.create",
            &json!({"connection": "c", "database": "main", "tables": [{"name": "studio"}], "views": false, "file_name": "before upgrade", "single_transaction": true}),
        )
        .unwrap();
    assert!(b["file"].as_str().unwrap() == "before upgrade.dbcb", "{b}");
    assert_eq!((b["tables"].as_u64(), b["views"].as_u64(), b["rows"].as_u64()), (Some(1), Some(0), Some(2)));
    // Lock all tables (SQLite: BEGIN IMMEDIATE) works too.
    let b2 = s.dispatch("backup.create", &json!({"connection": "c", "database": "main", "lock_tables": true})).unwrap();
    assert_eq!(b2["rows"].as_u64(), Some(8));
    // A file name can't leave the backups folder.
    assert!(s.dispatch("backup.create", &json!({"connection": "c", "database": "main", "file_name": "../x"})).is_err());
    // Restore only `film` from the full backup: `studio` is left alone.
    q(&mut s, "update studio set name = name || ' (changed)'; delete from film");
    let r = s.dispatch("backup.restore", &json!({"connection": "c", "database": "main", "path": b2["path"], "tables": ["film"]})).unwrap();
    assert_eq!(r["tables"], 1, "{r}");
    assert_eq!(n(&mut s, "select count(*) from film"), 3);
    assert_eq!(n(&mut s, "select count(*) from studio where name like '% (changed)'"), 2);
    // Extract SQL: a script that rebuilds the backup elsewhere.
    let out = d.join("extract.sql");
    s.dispatch("backup.extractSql", &json!({"path": b2["path"], "out": out.to_string_lossy()})).unwrap();
    let sql = std::fs::read_to_string(&out).unwrap();
    assert!(sql.contains("CREATE TABLE \"film\"") && sql.contains("INSERT INTO \"film\"") && sql.contains("CREATE VIEW \"titles\""), "{sql}");
    let list = s.dispatch("backup.list", &json!({"connection": "c", "database": "main"})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
}

#[test]
fn older_sql_backups_and_damaged_files() {
    let (mut s, d) = session("legacy");
    q(&mut s, SCHEMA);
    let old = s.dispatch("backup.create", &json!({"connection": "c", "database": "main", "format": "sql"})).unwrap();
    assert_eq!(old["format"], "sql");
    q(&mut s, "delete from studio");
    s.dispatch("backup.restore", &json!({"connection": "c", "database": "main", "path": old["path"]})).unwrap();
    assert_eq!(n(&mut s, "select count(*) from studio"), 2, "older script backups still restore");
    // Damaged or foreign files are errors, never panics.
    let dir = d.join("data").join("backups").join("c").join("main");
    std::fs::write(dir.join("junk.dbcb"), b"not a tar").unwrap();
    let list = s.dispatch("backup.list", &json!({"connection": "c", "database": "main"})).unwrap();
    let junk = list.as_array().unwrap().iter().find(|b| b["file"] == "junk.dbcb").unwrap().clone();
    assert_eq!(junk["complete"], false);
    assert!(s.dispatch("backup.restore", &json!({"connection": "c", "database": "main", "path": junk["path"]})).is_err());
    let good = s.dispatch("backup.create", &json!({"connection": "c", "database": "main"})).unwrap();
    let mut bytes = std::fs::read(good["path"].as_str().unwrap()).unwrap();
    let cut = bytes.len() * 2 / 3;
    bytes.truncate(cut);
    std::fs::write(dir.join("cut.dbcb"), &bytes).unwrap();
    assert!(
        s.dispatch(
            "backup.restore",
            &json!({"connection": "c", "database": "main", "path": dir.join("cut.dbcb").to_string_lossy(), "continue_on_error": true})
        )
        .is_err(),
        "a cut archive is reported"
    );
}

/// The archive format against the older script on 200,000 rows (both restore the same data).
#[test]
fn faster_than_the_script_format() {
    let (mut s, _d) = session("speed");
    q(&mut s, "create table big (id integer primary key, name text, amount real, note text)");
    q(
        &mut s,
        "with recursive n(x) as (select 1 union all select x + 1 from n where x < 200000) insert into big (name, amount, note) select 'name ' || x, x * 1.5, 'a fairly ordinary note for row ' || x from n",
    );
    let t = Instant::now();
    let new = s.dispatch("backup.create", &json!({"connection": "c", "database": "main"})).unwrap();
    let new_backup = t.elapsed();
    let t = Instant::now();
    let old = s.dispatch("backup.create", &json!({"connection": "c", "database": "main", "format": "sql"})).unwrap();
    let old_backup = t.elapsed();
    let t = Instant::now();
    s.dispatch("backup.restore", &json!({"connection": "c", "database": "main", "path": new["path"]})).unwrap();
    let new_restore = t.elapsed();
    assert_eq!(n(&mut s, "select count(*) from big"), 200_000);
    let t = Instant::now();
    s.dispatch("backup.restore", &json!({"connection": "c", "database": "main", "path": old["path"]})).unwrap();
    let old_restore = t.elapsed();
    assert_eq!(n(&mut s, "select count(*) from big"), 200_000);
    let (new_size, old_size) = (new["size"].as_u64().unwrap(), old["size"].as_u64().unwrap());
    eprintln!(
        "backup: dbcb {new_backup:?} / sql {old_backup:?}; restore: dbcb {new_restore:?} / sql {old_restore:?}; size: dbcb {} KB / sql {} KB",
        new_size / 1024,
        old_size / 1024
    );
    assert!(new_size * 3 < old_size, "compressed: {new_size} vs {old_size}");
    assert!(new_restore < old_restore, "restore faster: {new_restore:?} vs {old_restore:?}");
}
