//! Cross-engine Data Transfer and server backups against `dev/docker-compose.yml`:
//!   cargo test -p dbcraft-engine --test tools_servers -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::Session;
use serde_json::{Value, json};

fn session(name: &str) -> Session {
    let d = std::env::temp_dir().join(format!("dbcraft-tools-servers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    for (id, kind, port) in [("pg", "postgres", 15432), ("my", "mysql", 13306), ("maria", "mariadb", 13307)] {
        s.dispatch("connection.save", &json!({"id": id, "kind": kind, "name": id, "host": "127.0.0.1", "port": port, "user": "dbcraft", "password": "dbcraft", "database": "sakila_lite"}))
            .unwrap();
    }
    s.dispatch("connection.save", &json!({"id": "lite", "kind": "sqlite", "name": "lite", "database": d.join("copy.db").to_string_lossy()})).unwrap();
    s
}

fn q(s: &mut Session, conn: &str, sql: &str) -> Value {
    s.dispatch("connection.open", &json!({"id": conn})).ok();
    s.dispatch("query.run", &json!({"connection": conn, "database": "sakila_lite", "sql": sql})).unwrap_or_else(|e| panic!("{conn}: {sql}: {e}"))["result"]["rows"].clone()
}

fn cell(v: &Value) -> String {
    match &v["v"] {
        Value::String(s) => s.clone(),
        Value::Null => "NULL".into(),
        other => other.to_string(),
    }
}

/// Copy `film` and `actor` from one engine to another into tables prefixed `xfer_`, then compare.
#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn transfer_across_engines() {
    let mut s = session("xfer");
    for (src, dst) in [("pg", "my"), ("my", "pg"), ("maria", "lite"), ("lite", "maria")] {
        // Fresh copies each time: src tables → dst (lite gets filled by the maria → lite step first).
        let (src_db, dst_db) = (if src == "lite" { "main" } else { "sakila_lite" }, if dst == "lite" { "main" } else { "sakila_lite" });
        let tables = match src {
            "pg" => json!([{"name": "film", "schema": "public"}, {"name": "actor", "schema": "public"}]),
            // The SQLite copy was made by the maria -> lite step, under the xfer_ names.
            "lite" => json!([{"name": "xfer_film"}, {"name": "xfer_actor"}]),
            _ => json!([{"name": "film"}, {"name": "actor"}]),
        };
        // Copy into a scratch schema name on the target by transferring into the same DB with drop_existing
        // only when the target is not the seed DB's owner of these tables: use a file dump + rename instead.
        let file = std::env::temp_dir().join(format!("dbcraft-xfer-{src}-{dst}-{}.sql", std::process::id()));
        let dialect = match dst {
            "pg" => "postgres",
            "lite" => "sqlite",
            _ => "mysql",
        };
        s.dispatch(
            "tools.dataTransfer",
            &json!({"source": {"connection": src, "database": src_db}, "target": {"file": file.to_string_lossy(), "dialect": dialect}, "tables": tables}),
        )
        .unwrap_or_else(|e| panic!("{src}->{dst} dump: {e}"));
        // Load the script into xfer_ tables on the target.
        let script = std::fs::read_to_string(&file)
            .unwrap()
            .replace("\"film\"", "\"xfer_film\"")
            .replace("\"actor\"", "\"xfer_actor\"")
            .replace("`film`", "`xfer_film`")
            .replace("`actor`", "`xfer_actor`");
        s.dispatch("connection.open", &json!({"id": dst})).ok();
        let _ =
            s.dispatch("query.run", &json!({"connection": dst, "database": dst_db, "sql": "DROP TABLE IF EXISTS xfer_film; DROP TABLE IF EXISTS xfer_actor"}));
        s.dispatch("query.run", &json!({"connection": dst, "database": dst_db, "sql": script})).unwrap_or_else(|e| panic!("{src}->{dst} load: {e}"));

        let a = q(
            &mut s,
            src,
            if src == "lite" {
                "select title, rating, release_year from xfer_film order by 1"
            } else {
                "select title, rating, release_year from film order by 1"
            },
        );
        let b = q(&mut s, dst, "select title, rating, release_year from xfer_film order by 1");
        let norm = |v: &Value| v.as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(cell).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(norm(&a), norm(&b), "{src} -> {dst}");
        assert_eq!(q(&mut s, dst, "select count(*) from xfer_actor")[0][0]["v"].as_i64(), Some(3), "{src} -> {dst}");
    }
    for c in ["pg", "my", "maria"] {
        let _ = s.dispatch(
            "query.run",
            &json!({"connection": c, "database": "sakila_lite", "sql": "DROP TABLE IF EXISTS xfer_film; DROP TABLE IF EXISTS xfer_actor"}),
        );
    }
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn backup_restore_on_servers() {
    let mut s = session("backup");
    for c in ["pg", "my", "maria"] {
        q(
            &mut s,
            c,
            "create table if not exists bk_test (id int primary key, note varchar(20)); delete from bk_test; insert into bk_test values (1, 'keep'), (2, 'me')",
        );
        let tables = if c == "pg" { json!([{"name": "bk_test", "schema": "public"}]) } else { json!([{"name": "bk_test"}]) };
        let b = s.dispatch("backup.create", &json!({"connection": c, "database": "sakila_lite", "tables": tables})).unwrap_or_else(|e| panic!("{c}: {e}"));
        assert_eq!(b["rows"], 2, "{c}");
        q(&mut s, c, "delete from bk_test");
        s.dispatch("backup.restore", &json!({"connection": c, "database": "sakila_lite", "path": b["path"]})).unwrap_or_else(|e| panic!("{c}: {e}"));
        assert_eq!(
            q(&mut s, c, "select note from bk_test order by id").to_string(),
            json!([[{"t": "Text", "v": "keep"}], [{"t": "Text", "v": "me"}]]).to_string(),
            "{c}"
        );
        q(&mut s, c, "drop table bk_test");
    }
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn direct_transfer_postgres_to_sqlite() {
    let mut s = session("direct");
    let r = s
        .dispatch(
            "tools.dataTransfer",
            &json!({"source": {"connection": "pg", "database": "sakila_lite"}, "target": {"connection": "lite", "database": "main"}, "options": {"drop_existing": true}}),
        )
        .unwrap();
    assert_eq!(r["errors"], json!([]));
    assert_eq!(r["tables"], 3, "film, actor, film_actor");
    s.dispatch("connection.open", &json!({"id": "lite"})).ok();
    let rows =
        s.dispatch("query.run", &json!({"connection": "lite", "sql": "select title, meta from film order by film_id"})).unwrap()["result"]["rows"].clone();
    assert_eq!(rows[0][0]["v"], "The Quiet Harbor");
    assert!(rows[0][1]["v"].as_str().unwrap_or("").contains("drama"), "jsonb carried as text: {rows}");
}
