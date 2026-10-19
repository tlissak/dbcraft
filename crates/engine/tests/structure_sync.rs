//! Structure Synchronization: compare, the deployment script, deploy, and nothing left to do after.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use serde_json::{Value as Json, json};

fn session(name: &str) -> Session {
    let d = std::env::temp_dir().join(format!("dbcraft-ssync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    for id in ["src", "dst"] {
        s.dispatch("connection.save", &json!({"id": id, "kind": "sqlite", "name": id, "database": d.join(format!("{id}.db")).to_string_lossy()})).unwrap();
        s.dispatch("connection.open", &json!({"id": id})).unwrap();
    }
    s
}

fn run(s: &mut Session, conn: &str, sql: &str) {
    s.dispatch("query.run", &json!({"connection": conn, "sql": sql})).unwrap();
}

fn diffs(r: &Json) -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = r["differences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["object"].as_str().unwrap().to_string(), d["type"].as_str().unwrap().to_string(), d["action"].as_str().unwrap().to_string()))
        .collect();
    v.sort();
    v
}

#[test]
fn sqlite_compare_deploy_and_converge() {
    let mut s = session("lite");
    run(
        &mut s,
        "src",
        "create table studio (id integer primary key, name text not null);
        create table film (id integer primary key, title text not null, year int, studio_id int references studio(id));
        create index film_year on film (year);
        create view titles as select title from film;",
    );
    run(
        &mut s,
        "dst",
        "create table film (id integer primary key, title text not null);
        create index film_title on film (title);
        insert into film (title) values ('Metropolis');
        create table junk (x int);",
    );
    let params =
        json!({"source": {"connection": "src", "database": "main"}, "target": {"connection": "dst", "database": "main"}, "options": {"drop_extra": true}});
    let r = s.dispatch("tools.structureSync", &params).unwrap();
    assert_eq!(
        diffs(&r),
        [
            ("film".to_string(), "table".to_string(), "alter".to_string()),
            ("junk".into(), "table".into(), "drop".into()),
            ("studio".into(), "table".into(), "create".into()),
            ("titles".into(), "view".into(), "create".into()),
        ],
        "{r:#}"
    );
    // Deploy everything but the drop.
    let mut deploy = params.clone();
    deploy["deploy"] = json!(true);
    deploy["objects"] = json!(["film", "studio", "titles"]);
    let r = s.dispatch("tools.structureSync", &deploy).unwrap();
    assert_eq!(r["deployed"], 3, "{r:#}");
    let r = s.dispatch("tools.structureSync", &params).unwrap();
    assert_eq!(diffs(&r), [("junk".to_string(), "table".to_string(), "drop".to_string())], "only the excluded drop is left: {r:#}");
    let kept = s.dispatch("query.run", &json!({"connection": "dst", "sql": "select title from film"})).unwrap();
    assert_eq!(kept["result"]["rows"][0][0]["v"], "Metropolis", "data kept through the alter");
    // Different engines are refused with a pointer to Data Transfer.
    s.dispatch("connection.save", &json!({"id": "pg", "kind": "postgres", "name": "pg", "host": "127.0.0.1", "port": 1})).unwrap();
    assert!(
        s.dispatch("tools.structureSync", &json!({"source": {"connection": "src", "database": "main"}, "target": {"connection": "missing", "database": "x"}}))
            .is_err()
    );
}

#[test]
#[ignore = "needs docker compose servers"]
fn postgres_between_schemas() {
    let mut s = Session::new();
    s.dispatch("connection.save", &json!({"id": "pg", "kind": "postgres", "name": "pg", "host": "127.0.0.1", "port": 15432, "user": "dbcraft", "password": "dbcraft", "database": "sakila_lite"})).unwrap();
    s.dispatch("connection.open", &json!({"id": "pg"})).unwrap();
    run(
        &mut s,
        "pg",
        "drop schema if exists ss_a cascade; drop schema if exists ss_b cascade; create schema ss_a; create schema ss_b;
        create table ss_a.studio (id serial primary key, name text not null unique);
        create table ss_a.film (id serial primary key, title varchar(100) not null default 'x', studio_id int references ss_a.studio(id) on delete set null,
                                constraint film_title_ok check (length(title) > 0));
        create index film_title_idx on ss_a.film (title);
        comment on table ss_a.film is 'Films';
        create table ss_b.film (id serial primary key, title text);",
    );
    let params = json!({"source": {"connection": "pg", "database": "sakila_lite", "schema": "ss_a"}, "target": {"connection": "pg", "database": "sakila_lite", "schema": "ss_b"}});
    let r = s.dispatch("tools.structureSync", &params).unwrap();
    assert_eq!(diffs(&r), [("film".to_string(), "table".to_string(), "alter".to_string()), ("studio".into(), "table".into(), "create".into())], "{r:#}");
    let mut deploy = params.clone();
    deploy["deploy"] = json!(true);
    let r = s.dispatch("tools.structureSync", &deploy).unwrap();
    assert_eq!(r["errors"], json!([]), "{r:#}");
    let r = s.dispatch("tools.structureSync", &params).unwrap();
    assert_eq!(r["differences"], json!([]), "converged: {r:#}");
    run(&mut s, "pg", "drop schema ss_a cascade; drop schema ss_b cascade;");
}
