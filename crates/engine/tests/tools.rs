//! Saved queries, Data Transfer, Backup/Restore and Automation, end to end on SQLite.
//! Cross-engine transfer runs against the docker servers in `tests/tools_servers.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use std::path::PathBuf;

use dbcraft_engine::Session;
use serde_json::{Value, json};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dbcraft-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A session with a workspace file, a `src` SQLite db with data and an empty `dst` db.
fn session(name: &str) -> (Session, PathBuf) {
    let d = dir(name);
    let (mut s, err) = Session::with_workspace_file(d.join("workspace.json"));
    assert!(err.is_none());
    for id in ["src", "dst"] {
        s.dispatch("connection.save", &json!({"id": id, "kind": "sqlite", "name": id, "database": d.join(format!("{id}.db")).to_string_lossy()})).unwrap();
    }
    s.dispatch("connection.open", &json!({"id": "src"})).unwrap();
    s.dispatch(
        "query.run",
        &json!({"connection": "src", "sql": "
            create table film (id integer primary key, title varchar(100) not null, rating real, poster blob, active boolean);
            insert into film (title, rating, poster, active) values ('Metropolis', 8.3, x'cafe', 1), ('It''s alive', null, null, 0);
            create table tag (film_id int, tag text);
            insert into tag values (1, 'silent'), (1, 'sci-fi'), (2, 'horror');"}),
    )
    .unwrap();
    (s, d)
}

fn rows(s: &mut Session, conn: &str, sql: &str) -> Value {
    s.dispatch("connection.open", &json!({"id": conn})).ok();
    s.dispatch("query.run", &json!({"connection": conn, "sql": sql})).unwrap()["result"]["rows"].clone()
}

#[test]
fn query_run_returns_one_result_per_select() {
    let (mut s, _) = session("results");
    let r = s.dispatch("query.run", &json!({"connection": "src", "sql": "select 1; update tag set tag = tag; select 2, 3"})).unwrap();
    assert_eq!(r["results"].as_array().unwrap().len(), 2);
    assert_eq!(r["results"][1]["columns"].as_array().unwrap().len(), 2);
}

#[test]
fn saved_queries_persist_in_the_workspace() {
    let (mut s, d) = session("queries");
    let id = s.dispatch("query.save", &json!({"name": "Top films", "connection": "src", "database": "main", "sql": "select * from film"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.dispatch("query.save", &json!({"name": "top FILMS", "connection": "src", "database": "main", "sql": "x"})).is_err(), "duplicate name");
    s.dispatch("query.save", &json!({"id": id, "name": "Top films", "connection": "src", "database": "main", "sql": "select title from film"})).unwrap();
    let (reloaded, _) = Session::with_workspace_file(d.join("workspace.json"));
    assert_eq!(reloaded.workspace.queries.len(), 1);
    assert_eq!(reloaded.workspace.queries[0].sql, "select title from film");
    assert!(s.dispatch("query.save", &json!({"name": " ", "connection": "src", "sql": ""})).is_err());
    assert!(s.dispatch("query.save", &json!({"name": "x", "connection": "nope", "sql": ""})).is_err());
    s.dispatch("query.delete", &json!({"id": id})).unwrap();
    assert!(s.dispatch("query.delete", &json!({"id": id})).is_err());
}

#[test]
fn data_transfer_between_connections_and_to_file() {
    let (mut s, d) = session("transfer");
    let r = s
        .dispatch("tools.dataTransfer", &json!({"source": {"connection": "src", "database": "main"}, "target": {"connection": "dst", "database": "main"}}))
        .unwrap();
    assert_eq!((r["tables"].as_u64(), r["rows"].as_u64()), (Some(2), Some(5)));
    assert_eq!(
        rows(&mut s, "dst", "select title, rating, hex(poster), active from film order by id").to_string(),
        rows(&mut s, "src", "select title, rating, hex(poster), active from film order by id").to_string()
    );

    // Again without drop: the tables exist, so it fails; with drop_existing it replaces them.
    assert!(
        s.dispatch("tools.dataTransfer", &json!({"source": {"connection": "src", "database": "main"}, "target": {"connection": "dst", "database": "main"}}))
            .is_err()
    );
    let r = s
        .dispatch(
            "tools.dataTransfer",
            &json!({"source": {"connection": "src", "database": "main"}, "target": {"connection": "dst", "database": "main"}, "tables": [{"name": "tag"}], "options": {"drop_existing": true, "batch_size": 2}}),
        )
        .unwrap();
    assert_eq!(r["rows"], 3);

    // To a PostgreSQL-dialect script.
    let file = d.join("dump.sql");
    s.dispatch(
        "tools.dataTransfer",
        &json!({"source": {"connection": "src", "database": "main"}, "target": {"file": file.to_string_lossy(), "dialect": "postgres"}}),
    )
    .unwrap();
    let sql = std::fs::read_to_string(&file).unwrap();
    assert!(sql.contains("CREATE TABLE \"film\" (\n  \"id\" BIGINT NOT NULL,\n  \"title\" VARCHAR(100) NOT NULL"), "{sql}");
    assert!(sql.contains("'\\xcafe'::bytea"), "{sql}");
    assert!(sql.contains("'It''s alive'"));

    // Same database with drop is refused (it would destroy the source).
    assert!(
        s.dispatch("tools.dataTransfer", &json!({"source": {"connection": "src", "database": "main"}, "target": {"connection": "src", "database": "main"}, "options": {"drop_existing": true}}))
            .is_err()
    );
}

#[test]
fn backup_and_restore_round_trip() {
    let (mut s, d) = session("backup");
    let b = s.dispatch("backup.create", &json!({"connection": "src", "database": "main", "comment": "before cleanup"})).unwrap();
    assert_eq!((b["tables"].as_u64(), b["rows"].as_u64()), (Some(2), Some(5)));
    assert_eq!(b["comment"], "before cleanup");
    let path = b["path"].as_str().unwrap().to_string();
    assert!(path.starts_with(&d.join("data").join("backups").to_string_lossy().into_owned()), "{path}");

    let list = s.dispatch("backup.list", &json!({"connection": "src", "database": "main"})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["comment"], "before cleanup");

    // Damage the data, restore, and it's back.
    rows(&mut s, "src", "delete from tag; update film set title = 'x'");
    s.dispatch("backup.restore", &json!({"connection": "src", "database": "main", "path": path})).unwrap();
    assert_eq!(rows(&mut s, "src", "select count(*) from tag")[0][0]["v"], 3);
    assert_eq!(rows(&mut s, "src", "select title from film where id = 2")[0][0]["v"], "It's alive");

    // Delete only works inside the backups folder.
    let outside = d.join("outside.sql");
    std::fs::write(&outside, "select 1;").unwrap();
    assert!(s.dispatch("backup.delete", &json!({"path": outside.to_string_lossy()})).is_err());
    assert!(outside.exists());
    s.dispatch("backup.delete", &json!({"path": path})).unwrap();
    assert!(s.dispatch("backup.list", &json!({"connection": "src", "database": "main"})).unwrap().as_array().unwrap().is_empty());
}

#[test]
fn automation_runs_steps_and_records_the_run() {
    let (mut s, d) = session("automation");
    let id = s
        .dispatch(
            "automation.save",
            &json!({"name": "Nightly", "steps": [
                {"command": "backup.create", "params": {"connection": "src", "database": "main"}, "label": "Backup src"},
                {"command": "query.run", "params": {"connection": "dst", "sql": "create table if not exists ran (at text); insert into ran values (datetime('now'))"}, "label": "Mark"},
            ]}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        s.dispatch("automation.save", &json!({"name": "bad", "steps": [{"command": "connection.delete", "params": {"id": "src"}}]})).is_err(),
        "only job commands"
    );

    // A fresh session, like the scheduled `dbcraft-cli batchjob` run: connections open on demand.
    let (mut fresh, _) = Session::with_workspace_file(d.join("workspace.json"));
    let r = fresh.dispatch("automation.run", &json!({"id": id})).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(fresh.dispatch("backup.list", &json!({"connection": "src", "database": "main"})).unwrap().as_array().unwrap().len(), 1);
    let (reloaded, _) = Session::with_workspace_file(d.join("workspace.json"));
    assert!(reloaded.workspace.automations[0].last_run.as_deref().unwrap_or("").contains("ok"));

    // Scheduling, without touching the real Task Scheduler.
    let r = s
        .dispatch("automation.schedule", &json!({"id": id, "frequency": "daily", "time": "02:00", "cli_path": "C:\\x\\dbcraft-cli.exe", "dry_run": true}))
        .unwrap();
    assert!(r["xml"].as_str().unwrap().contains(&format!("batchjob {id}</Arguments>")));
    assert!(s.dispatch("automation.schedule", &json!({"id": id, "frequency": "daily", "time": "2am", "cli_path": "x", "dry_run": true})).is_err());

    // A failing step: the job reports it and, with continue_on_error, still runs the rest.
    s.dispatch("automation.save", &json!({"id": id, "name": "Nightly", "steps": [{"command": "query.run", "params": {"connection": "dst", "sql": "selec oops"}}, {"command": "query.run", "params": {"connection": "dst", "sql": "select 1"}}]}))
        .unwrap();
    let r = s.dispatch("automation.run", &json!({"id": id})).unwrap();
    assert_eq!(r["ok"], false);
    assert_eq!(r["steps"][1]["ok"], true);
}

#[test]
fn backups_in_the_same_second_do_not_overwrite_each_other() {
    let (mut s, _) = session("samesecond");
    let a = s.dispatch("backup.create", &json!({"connection": "src", "database": "main"})).unwrap();
    let b = s.dispatch("backup.create", &json!({"connection": "src", "database": "main"})).unwrap();
    assert_ne!(a["path"], b["path"]);
    assert_eq!(s.dispatch("backup.list", &json!({"connection": "src", "database": "main"})).unwrap().as_array().unwrap().len(), 2);
}
