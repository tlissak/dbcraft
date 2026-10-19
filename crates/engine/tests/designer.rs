//! Table Designer and table actions end to end through the commands: SQLite always, the docker
//! servers with `--ignored`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_core::{CheckInfo, ColumnInfo, ForeignKeyInfo, GenericType as G, IndexInfo, TableSchema, TriggerInfo, UniqueInfo, Value};
use dbcraft_engine::Session;
use serde_json::{Value as Json, json};

fn sqlite_session(name: &str) -> Session {
    let d = std::env::temp_dir().join(format!("dbcraft-designer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (mut s, _) = Session::with_workspace_file(d.join("workspace.json"));
    s.dispatch("connection.save", &json!({"id": "c", "kind": "sqlite", "name": "c", "database": d.join("t.db").to_string_lossy()})).unwrap();
    s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
    s
}

fn col(name: &str, ty: &str, nullable: bool, pk: Option<u32>) -> ColumnInfo {
    ColumnInfo::new(name, ty, G::Text, nullable, pk)
}

fn rows(s: &mut Session, conn: &str, db: Option<&str>, sql: &str) -> Vec<Vec<Value>> {
    let r = s.dispatch("query.run", &json!({"connection": conn, "database": db, "sql": sql})).unwrap();
    serde_json::from_value::<dbcraft_core::ResultSet>(r["result"].clone()).unwrap().rows
}

fn schema(s: &mut Session, conn: &str, db: Option<&str>, table: &str) -> TableSchema {
    serde_json::from_value(s.dispatch("table.schema", &json!({"connection": conn, "database": db, "table": table})).unwrap()).unwrap()
}

fn save(s: &mut Session, conn: &str, db: Option<&str>, table: &TableSchema, original: Option<&TableSchema>, origins: Option<Vec<Option<String>>>) -> Json {
    s.dispatch("table.save", &json!({"connection": conn, "database": db, "table": table, "original": original, "origins": origins})).unwrap()
}

#[test]
fn sqlite_design_new_alter_rebuild_and_actions() {
    let mut s = sqlite_session("lite");
    // New tables: a parent, and a child with every component.
    let studio =
        TableSchema { name: "studio".into(), columns: vec![col("id", "INTEGER", false, Some(1)), col("name", "TEXT", false, None)], ..TableSchema::default() };
    save(&mut s, "c", None, &studio, None, None);
    let mut film = TableSchema {
        name: "film".into(),
        columns: vec![
            col("id", "INTEGER", false, Some(1)),
            col("title", "TEXT", false, None),
            col("year", "INTEGER", true, None),
            col("studio_id", "INTEGER", true, None),
        ],
        indexes: vec![IndexInfo { name: "film_year".into(), columns: vec!["year".into()], ..IndexInfo::default() }],
        uniques: vec![UniqueInfo { name: "uq_film_title".into(), columns: vec!["title".into()] }],
        checks: vec![CheckInfo { name: "year_ok".into(), expression: "year > 1880".into() }],
        foreign_keys: vec![ForeignKeyInfo {
            name: "fk_film_studio".into(),
            columns: vec!["studio_id".into()],
            ref_table: "studio".into(),
            ref_columns: vec!["id".into()],
            on_delete: Some("CASCADE".into()),
            ..ForeignKeyInfo::default()
        }],
        triggers: vec![TriggerInfo {
            name: "film_upper".into(),
            timing: "AFTER".into(),
            events: vec!["INSERT".into()],
            for_each_row: true,
            condition: None,
            body: "BEGIN UPDATE film SET title = upper(title) WHERE id = new.id; END".into(),
        }],
        ..TableSchema::default()
    };
    film.columns[2].default = Some("1900".into());
    let preview = s.dispatch("table.designSql", &json!({"connection": "c", "table": film})).unwrap();
    assert!(preview["statements"][0].as_str().unwrap().starts_with("CREATE TABLE \"film\""), "{preview}");
    save(&mut s, "c", None, &film, None, None);
    rows(
        &mut s,
        "c",
        None,
        "insert into studio (name) values ('Ufa'); insert into film (title, year, studio_id) values ('metropolis', 1927, 1), ('m', 1931, 1)",
    );
    assert_eq!(rows(&mut s, "c", None, "select title from film order by id")[0][0], Value::Text("METROPOLIS".into()), "trigger works");
    assert!(s.dispatch("query.run", &json!({"connection": "c", "sql": "insert into film (title, year) values ('x', 1700)"})).is_err(), "check works");
    let saved = schema(&mut s, "c", None, "film");
    assert_eq!((saved.indexes.len(), saved.uniques.len(), saved.checks.len(), saved.foreign_keys.len(), saved.triggers.len()), (1, 1, 1, 1, 1), "{saved:#?}");

    // Native ALTER: rename a field and add one.
    let mut edit = saved.clone();
    edit.columns[1].name = "name".into();
    edit.uniques[0].columns = vec!["name".into()];
    edit.columns.push(col("note", "TEXT", true, None));
    let origins = vec![Some("id".into()), Some("title".into()), Some("year".into()), Some("studio_id".into()), None];
    let plan = s.dispatch("table.designSql", &json!({"connection": "c", "table": edit, "original": saved, "origins": origins})).unwrap();
    // SQLite's RENAME COLUMN also renames it inside the unique: no rebuild needed.
    assert_eq!(plan["rebuild"], false, "{plan}");
    save(&mut s, "c", None, &edit, Some(&saved), Some(origins));
    let after = schema(&mut s, "c", None, "film");
    assert_eq!(after.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "name", "year", "studio_id", "note"]);
    assert_eq!(
        rows(&mut s, "c", None, "select name, year from film order by id"),
        vec![vec![Value::Text("METROPOLIS".into()), Value::Int(1927)], vec![Value::Text("M".into()), Value::Int(1931)]]
    );
    assert_eq!(after.triggers.len(), 1, "triggers recreated");
    assert_eq!(after.foreign_keys.len(), 1);

    // Rebuild of the parent must not cascade-delete the children.
    let st = schema(&mut s, "c", None, "studio");
    let mut st2 = st.clone();
    st2.columns[1].nullable = true;
    let plan = s.dispatch("table.designSql", &json!({"connection": "c", "table": st2, "original": st})).unwrap();
    assert_eq!(plan["rebuild"], true, "SQLite can't change NOT NULL in place: {plan}");
    save(&mut s, "c", None, &st2, Some(&st), None);
    assert_eq!(rows(&mut s, "c", None, "select count(*) from film")[0][0], Value::Int(2), "children survive the parent's rebuild");

    // Table actions.
    s.dispatch("table.duplicate", &json!({"connection": "c", "table": "film", "new_name": "film2"})).unwrap();
    assert_eq!(rows(&mut s, "c", None, "select count(*) from film2")[0][0], Value::Int(2));
    s.dispatch("table.rename", &json!({"connection": "c", "table": "film2", "new_name": "film3"})).unwrap();
    assert_eq!(s.dispatch("table.empty", &json!({"connection": "c", "table": "film3"})).unwrap()["deleted"], 2);
    s.dispatch("table.truncate", &json!({"connection": "c", "table": "film3"})).unwrap();
    s.dispatch("table.delete", &json!({"connection": "c", "tables": ["film3"]})).unwrap();
    assert!(s.dispatch("table.schema", &json!({"connection": "c", "table": "film3"})).is_err());
    // Bad input is an error, not a panic.
    assert!(s.dispatch("table.save", &json!({"connection": "c", "table": {"name": "", "columns": []}})).is_err());
    assert!(s.dispatch("table.delete", &json!({"connection": "c", "tables": []})).is_err());
}

/// The same flow on each server engine (types in each engine's own words).
#[test]
#[ignore = "needs docker compose servers"]
fn servers_design_new_alter_and_actions() {
    for (kind, port, int, text) in
        [("postgres", 15432, "integer", "character varying(100)"), ("mysql", 13306, "int", "varchar(100)"), ("mariadb", 13307, "int", "varchar(100)")]
    {
        let mut s = Session::new();
        s.dispatch(
            "connection.save",
            &json!({"id": "c", "kind": kind, "name": kind, "host": "127.0.0.1", "port": port, "user": "dbcraft", "password": "dbcraft", "database": "sakila_lite"}),
        )
        .unwrap();
        s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
        let db = Some("sakila_lite");
        let _ = s.dispatch("table.delete", &json!({"connection": "c", "database": db, "tables": ["dbc_design_copy"]}));
        let _ = s.dispatch("table.delete", &json!({"connection": "c", "database": db, "tables": ["dbc_design"]}));
        let mut t = TableSchema {
            name: "dbc_design".into(),
            columns: vec![col("id", int, false, Some(1)), col("title", text, false, None), col("year", int, true, None)],
            indexes: vec![IndexInfo { name: "dbc_design_year".into(), columns: vec!["year".into()], ..IndexInfo::default() }],
            checks: vec![CheckInfo { name: "dbc_year_ok".into(), expression: "year > 1880".into() }],
            comment: Some("Designed".into()),
            ..TableSchema::default()
        };
        t.columns[0].auto_increment = true;
        t.columns[2].default = Some("1900".into());
        save(&mut s, "c", db, &t, None, None);
        rows(&mut s, "c", db, "insert into dbc_design (title, year) values ('a', 1927), ('b', 1931)");
        let saved = schema(&mut s, "c", db, "dbc_design");
        assert!(saved.columns[0].auto_increment && saved.checks.len() == 1 && saved.comment.as_deref() == Some("Designed"), "{kind}: {saved:#?}");
        // Rename, retype, add a field and a unique key.
        let mut edit = saved.clone();
        edit.columns[1].name = "name".into();
        edit.columns[2].native_type = "bigint".into();
        edit.columns.push(col("note", text, true, None));
        if kind == "postgres" {
            edit.uniques.push(UniqueInfo { name: "dbc_design_name_key".into(), columns: vec!["name".into()] });
        } else {
            edit.indexes.push(IndexInfo { name: "dbc_design_name".into(), columns: vec!["name".into()], unique: true, ..IndexInfo::default() });
        }
        let origins = vec![Some("id".into()), Some("title".into()), Some("year".into()), None];
        save(&mut s, "c", db, &edit, Some(&saved), Some(origins));
        let after = schema(&mut s, "c", db, "dbc_design");
        assert_eq!(after.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "name", "year", "note"], "{kind}");
        assert_eq!(rows(&mut s, "c", db, "select count(*) from dbc_design where name in ('a', 'b')")[0][0].display(), "2", "{kind}: data kept");
        // Unchanged definition: nothing to run.
        let again = s.dispatch("table.designSql", &json!({"connection": "c", "database": db, "table": after, "original": after})).unwrap();
        assert_eq!(again["statements"], json!([]), "{kind}: {again}");
        s.dispatch("table.duplicate", &json!({"connection": "c", "database": db, "table": "dbc_design"})).unwrap();
        assert_eq!(rows(&mut s, "c", db, "select count(*) from dbc_design_copy")[0][0].display(), "2", "{kind}");
        s.dispatch("table.truncate", &json!({"connection": "c", "database": db, "table": "dbc_design_copy"})).unwrap();
        s.dispatch("table.delete", &json!({"connection": "c", "database": db, "tables": ["dbc_design_copy", "dbc_design"]})).unwrap();
    }
}
