//! Objects tab + table viewer commands, end to end on SQLite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_engine::{Session, TablePage};
use serde_json::{Value, json};

fn session(name: &str) -> Session {
    let db = std::env::temp_dir().join(format!("dbcraft-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let mut s = Session::new();
    s.dispatch("connection.save", &json!({"id": "c", "kind": "sqlite", "name": "c", "database": db.to_string_lossy()})).unwrap();
    s.dispatch("connection.open", &json!({"id": "c"})).unwrap();
    s.dispatch(
        "query.run",
        &json!({"connection": "c", "sql": "
            create table film (id integer primary key, title text, year int, note text);
            insert into film (title, year, note) values ('Metropolis', 1927, null), ('Nosferatu', 1922, ''), ('Sunrise', 1927, '50% off'), ('M', 1931, 'it''s');
            create table nokey (a int, b text);
            insert into nokey values (1, 'x'), (1, 'x'), (2, 'y');
            create view v_film as select title from film;"}),
    )
    .unwrap();
    s
}

fn page(s: &mut Session, extra: Value) -> TablePage {
    let mut p = json!({"connection": "c", "database": "main", "table": "film"});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    serde_json::from_value(s.dispatch("table.data", &p).unwrap()).unwrap()
}

fn titles(p: &TablePage) -> Vec<String> {
    p.result.rows.iter().map(|r| r[1].display()).collect()
}

#[test]
fn objects_tab_lists_tables_with_info_and_ddl() {
    let mut s = session("objects");
    let t = s.dispatch("database.tables", &json!({"connection": "c", "database": "main"})).unwrap();
    let film = t.as_array().unwrap().iter().find(|x| x["name"] == "film").unwrap();
    assert_eq!(film["rows"], 4);
    assert_eq!(film["kind"], "table");
    let view = t.as_array().unwrap().iter().find(|x| x["name"] == "v_film").unwrap();
    assert_eq!(view["kind"], "view");

    let ddl = s.dispatch("table.ddl", &json!({"connection": "c", "database": "main", "table": "film"})).unwrap();
    assert!(ddl["ddl"].as_str().unwrap().starts_with("CREATE TABLE film"));
    assert!(s.dispatch("table.ddl", &json!({"connection": "c", "database": "main", "table": "nope"})).is_err());
}

#[test]
fn filter_sort_and_page() {
    let mut s = session("filter");
    let p = page(&mut s, json!({}));
    assert_eq!((p.total, p.result.rows.len()), (4, 4));
    assert_eq!(p.primary_key, ["id"]);

    let p = page(&mut s, json!({"filters": [{"column": "year", "op": "=", "value": "1927"}], "sort": [{"column": "title", "desc": true}]}));
    assert_eq!(titles(&p), ["Sunrise", "Metropolis"]);
    assert_eq!(p.total, 2);

    // LIKE wildcards in the value are literal.
    let p = page(&mut s, json!({"filters": [{"column": "note", "op": "contains", "value": "%"}]}));
    assert_eq!(titles(&p), ["Sunrise"]);
    let p = page(&mut s, json!({"filters": [{"column": "note", "op": "contains", "value": "'"}]}));
    assert_eq!(titles(&p), ["M"]);

    let p = page(
        &mut s,
        json!({"filters": [{"column": "note", "op": "is_null"}, {"column": "note", "op": "is_empty"}], "match": "any", "sort": [{"column": "id"}]}),
    );
    assert_eq!(titles(&p), ["Metropolis", "Nosferatu"]);

    let p = page(&mut s, json!({"sort": [{"column": "id"}], "limit": 2, "offset": 2}));
    assert_eq!((titles(&p), p.total), (vec!["Sunrise".to_string(), "M".to_string()], 4));

    for bad in [
        json!({"filters": [{"column": "year", "op": "~~"}]}),
        json!({"filters": [{"column": "year", "op": "="}]}),
        json!({"filters": "x"}),
        json!({"match": "some", "filters": [{"column": "year", "op": "is_null"}]}),
        // (An unknown column is not an error on SQLite: it reads a double-quoted unknown
        // identifier as a string literal. The UI only offers real columns.)
        json!({"filters": [{"column": "year", "op": "is_null"}], "sort": "year"}),
    ] {
        let mut p = json!({"connection": "c", "database": "main", "table": "film"});
        p.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        assert!(s.dispatch("table.data", &p).is_err(), "{bad}");
    }
}

#[test]
fn live_edits_apply_atomically() {
    let mut s = session("edits");
    let r = s
        .dispatch(
            "table.applyEdits",
            &json!({"connection": "c", "database": "main", "table": "film", "changes": [
                {"op": "update", "key": {"id": 1}, "set": {"title": "Metropolis (restored)", "note": null}},
                {"op": "insert", "values": {"title": "Faust", "year": 1926}},
                {"op": "delete", "key": {"id": 4}},
            ]}),
        )
        .unwrap();
    assert_eq!(r["applied"], 3);
    let p = page(&mut s, json!({"sort": [{"column": "id"}]}));
    assert_eq!(titles(&p), ["Metropolis (restored)", "Nosferatu", "Sunrise", "Faust"]);

    // Two identical rows: an update keyed on all columns would hit both, so nothing is saved.
    let err = s
        .dispatch(
            "table.applyEdits",
            &json!({"connection": "c", "database": "main", "table": "nokey", "changes": [
                {"op": "update", "key": {"a": 2, "b": "y"}, "set": {"b": "z"}},
                {"op": "update", "key": {"a": 1, "b": "x"}, "set": {"b": "w"}},
            ]}),
        )
        .unwrap_err();
    assert!(err.to_string().contains("nothing was saved"), "{err}");
    let rs = s.dispatch("query.run", &json!({"connection": "c", "sql": "select b from nokey order by a, b"})).unwrap();
    assert_eq!(rs["result"]["rows"].to_string(), json!([[{"t":"Text","v":"x"}],[{"t":"Text","v":"x"}],[{"t":"Text","v":"y"}]]).to_string());

    // A key that matches nothing is also rejected.
    assert!(
        s.dispatch("table.applyEdits", &json!({"connection": "c", "database": "main", "table": "film", "changes": [{"op": "delete", "key": {"id": 999}}]}))
            .is_err()
    );
}

#[test]
fn hostile_values_stay_values() {
    let mut s = session("hostile");
    let evil = "x'); drop table film; --";
    s.dispatch(
        "table.applyEdits",
        &json!({"connection": "c", "database": "main", "table": "film", "changes": [{"op": "update", "key": {"id": 2}, "set": {"title": evil}}]}),
    )
    .unwrap();
    let p = page(&mut s, json!({"filters": [{"column": "id", "op": "=", "value": "2"}]}));
    assert_eq!(titles(&p), [evil]);
    let p = page(&mut s, json!({"filters": [{"column": "title", "op": "=", "value": evil}]}));
    assert_eq!(p.total, 1);
}
