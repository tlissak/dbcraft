//! Server engines against the docker test servers. Opt-in:
//!   docker compose -f dev/docker-compose.yml up -d
//!   cargo test -p dbcraft-drivers --test servers -- --ignored

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration-test helpers outside #[test] fns

use dbcraft_core::{ConnectionConfig, DbKind, SshAuth, SshConfig, Value};
use dbcraft_model::ObjectKind;

fn cfg(kind: DbKind, port: u16) -> ConnectionConfig {
    ConnectionConfig {
        id: kind.id().into(),
        name: kind.label().into(),
        kind,
        host: "127.0.0.1".into(),
        port: Some(port),
        user: "dbcraft".into(),
        password: Some("dbcraft".into()),
        database: "sakila_lite".into(),
        ssh: None,
    }
}

fn exercise(kind: DbKind, port: u16) {
    let mut c = dbcraft_drivers::connect(&cfg(kind, port)).unwrap();
    assert!(!c.server_version().unwrap().is_empty());
    assert!(c.list_databases().unwrap().iter().any(|d| d == "sakila_lite"));
    let objs = c.list_objects("sakila_lite").unwrap();
    assert!(objs.iter().any(|o| o.kind == ObjectKind::Table && o.name == "film"));
    assert!(objs.iter().any(|o| o.kind == ObjectKind::View && o.name == "film_list"));

    let rs = c.query(Some("sakila_lite"), "select film_id, title, rating, added_at, meta from film order by film_id").unwrap();
    assert_eq!(rs.rows.len(), 3);
    assert_eq!(rs.rows[0][1], Value::Text("The Quiet Harbor".into()));
    assert_eq!(rs.rows[0][2], Value::Text("7.4".into()));
    assert_eq!(rs.rows[2][4], Value::Null);
    for row in &rs.rows {
        for v in row {
            assert!(!matches!(v, Value::Text(t) if t.starts_with('<')), "undecoded cell {v:?}");
        }
    }

    // Objects tab, information pane, editing support.
    let infos = c.table_info("sakila_lite").unwrap();
    let film = infos.iter().find(|t| t.name == "film").unwrap();
    assert_eq!(film.kind, "table");
    assert!(film.data_length.is_some(), "{film:?}");
    assert!(infos.iter().any(|t| t.name == "film_list" && t.kind == "view"));
    let schema = film.schema.as_deref();
    let ddl = c.table_ddl("sakila_lite", schema, "film").unwrap();
    assert!(ddl.to_ascii_uppercase().starts_with("CREATE TABLE"), "{ddl}");
    assert!(ddl.contains("title"), "{ddl}");
    assert!(c.table_ddl("sakila_lite", schema, "film_list").unwrap().to_ascii_uppercase().contains("VIEW"));
    assert_eq!(c.primary_key(Some("sakila_lite"), schema, "film").unwrap(), ["film_id"]);
    assert_eq!(c.primary_key(Some("sakila_lite"), schema, "film_actor").unwrap(), ["actor_id", "film_id"]);

    // Atomic batch: the failing count check rolls back the first statement.
    let t = if kind == DbKind::Postgres { "public.actor" } else { "actor" };
    let ok = (format!("update {t} set first_name = 'Changed' where actor_id = 1"), Some(1));
    let miss = (format!("update {t} set first_name = 'Nope' where actor_id = 999"), Some(1));
    assert!(c.execute_atomic(Some("sakila_lite"), &[ok.clone(), miss]).is_err());
    let name = c.query(Some("sakila_lite"), &format!("select first_name from {t} where actor_id = 1")).unwrap();
    assert_eq!(name.rows[0][0], Value::Text("Ada".into()), "rolled back");
    let undo = (format!("update {t} set first_name = 'Ada' where actor_id = 1"), Some(1));
    assert_eq!(c.execute_atomic(Some("sakila_lite"), &[ok, undo]).unwrap(), [1, 1]);
}

#[test]
#[ignore = "needs dev/docker-compose.yml (ssh service)"]
fn postgres_and_mysql_over_ssh() {
    let ssh = SshConfig {
        host: "127.0.0.1".into(),
        port: 12222,
        user: "dbcraft".into(),
        auth: SshAuth::Password,
        password: Some("dbcraft".into()),
        private_key: None,
    };
    // Database hosts are names on the docker network, reachable only through the bastion.
    for (kind, host, port) in [(DbKind::Postgres, "postgres", 5432), (DbKind::Mysql, "mysql", 3306)] {
        let mut cfg = cfg(kind, port);
        cfg.host = host.into();
        cfg.ssh = Some(ssh.clone());
        let mut c = dbcraft_drivers::connect(&cfg).unwrap_or_else(|e| panic!("{kind:?} over ssh: {e}"));
        assert_eq!(c.query(Some("sakila_lite"), "select count(*) from film").unwrap().rows.len(), 1);
    }
    let mut bad = cfg(DbKind::Postgres, 5432);
    bad.host = "postgres".into();
    bad.ssh = Some(SshConfig { password: Some("wrong".into()), ..ssh });
    let err = dbcraft_drivers::connect(&bad).err().unwrap().to_string();
    assert!(err.contains("SSH"), "{err}");
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn postgres() {
    exercise(DbKind::Postgres, 15432);
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn mysql() {
    exercise(DbKind::Mysql, 13306);
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn mariadb() {
    exercise(DbKind::Mariadb, 13307);
}

#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn columns_have_generic_types() {
    use dbcraft_core::GenericType as G;
    for (kind, port, schema) in [(DbKind::Postgres, 15432, Some("public")), (DbKind::Mysql, 13306, None), (DbKind::Mariadb, 13307, None)] {
        let mut c = dbcraft_drivers::connect(&cfg(kind, port)).unwrap();
        let cols = c.columns(Some("sakila_lite"), schema, "film").unwrap();
        let by = |n: &str| cols.iter().find(|c| c.name == n).unwrap_or_else(|| panic!("{kind:?} {n}"));
        assert_eq!(by("film_id").pk, Some(1), "{kind:?}");
        assert!(!by("film_id").nullable);
        assert_eq!(by("rating").generic, G::Decimal { precision: 3, scale: 1 }, "{kind:?}");
        let want_title = if kind == DbKind::Postgres { G::Text } else { G::Varchar { len: 128 } }; // seeds differ
        assert_eq!(by("title").generic, want_title, "{kind:?}");
        assert_eq!(by("added_at").generic, G::DateTime, "{kind:?}");
        assert!(matches!(by("meta").generic, G::Json | G::Text), "{kind:?} {:?}", by("meta"));
        assert!(c.columns(Some("sakila_lite"), schema, "nope").is_err());
    }
}

/// Create a user, grant, read back, change, drop: against each server as an admin.
#[test]
#[ignore = "needs dev/docker-compose.yml"]
fn user_lifecycle() {
    use dbcraft_core::{Grant, UserInfo};
    use dbcraft_drivers::users;
    for (kind, port) in [(DbKind::Postgres, 15432), (DbKind::Mysql, 13306), (DbKind::Mariadb, 13307)] {
        let mut admin = cfg(kind, port);
        if kind != DbKind::Postgres {
            admin.user = "root".into();
        }
        let mut c = dbcraft_drivers::connect(&admin).unwrap();
        let maria = users::server_is_mariadb(c.as_mut());
        let (object, privs) = match kind {
            DbKind::Postgres => ("TABLE public.film", vec!["SELECT".to_string(), "UPDATE".to_string()]),
            _ => ("sakila_lite.*", vec!["SELECT".to_string(), "UPDATE".to_string()]),
        };
        let host = (kind != DbKind::Postgres).then(|| "%".to_string());
        // Clean up a previous failed run.
        let probe = UserInfo { name: "dbc_test_user".into(), host: host.clone(), ..UserInfo::default() };
        if let Ok(old) = users::get(c.as_mut(), "dbc_test_user", host.as_deref()) {
            for sql in users::drop_sql(kind, maria, &old).unwrap() {
                let _ = c.query(None, &sql);
            }
        }
        let _ = probe;

        let new = UserInfo {
            name: "dbc_test_user".into(),
            host: host.clone(),
            grants: vec![Grant { object: object.into(), privileges: privs.clone(), grant_option: false }],
            ..UserInfo::default()
        };
        for sql in users::save_sql(kind, maria, None, &new, Some("S3cret!pw")).unwrap() {
            c.query(None, &sql).unwrap_or_else(|e| panic!("{kind:?} {sql}: {e}"));
        }
        let back = users::get(c.as_mut(), "dbc_test_user", host.as_deref()).unwrap();
        let g = back.grants.iter().find(|g| g.object.replace('"', "") == object).unwrap_or_else(|| panic!("{kind:?} grants {:?}", back.grants));
        assert_eq!(g.privileges, privs, "{kind:?}");
        assert!(users::list(c.as_mut()).unwrap().iter().any(|u| u.name == "dbc_test_user"));

        // The new user can log in with its password.
        let mut login = cfg(kind, port);
        login.user = "dbc_test_user".into();
        login.password = Some("S3cret!pw".into());
        dbcraft_drivers::connect(&login).unwrap_or_else(|e| panic!("{kind:?} login: {e}"));

        // Revoke UPDATE.
        let mut changed = back.clone();
        if let Some(g) = changed.grants.iter_mut().find(|g| g.object.replace('"', "") == object) {
            g.privileges.retain(|p| p != "UPDATE");
        }
        for sql in users::save_sql(kind, maria, Some(&back), &changed, None).unwrap() {
            c.query(None, &sql).unwrap_or_else(|e| panic!("{kind:?} {sql}: {e}"));
        }
        let back2 = users::get(c.as_mut(), "dbc_test_user", host.as_deref()).unwrap();
        let g = back2.grants.iter().find(|g| g.object.replace('"', "") == object).unwrap();
        assert_eq!(g.privileges, ["SELECT"], "{kind:?}");

        for sql in users::drop_sql(kind, maria, &back2).unwrap() {
            c.query(None, &sql).unwrap_or_else(|e| panic!("{kind:?} {sql}: {e}"));
        }
        assert!(!users::list(c.as_mut()).unwrap().iter().any(|u| u.name == "dbc_test_user"));
    }
}
