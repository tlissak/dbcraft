//! Users, roles and privileges (Navicat's User / Role designers and Privilege Manager).
//!
//! Reading goes through [`Connection::query`]; writing is pure SQL generation ([`save_sql`],
//! [`drop_sql`]) so the UI can show a SQL Preview and tests need no server.

use std::collections::BTreeMap;

use dbcraft_core::{DbKind, Error, Grant, Result, UserInfo};
use dbcraft_sql::{Dialect, quote_ident, quote_literal};

use crate::Connection;
use crate::common::{cell_bool, cell_str};

/// Checkbox lists for the designer: (server-level privileges or attributes, object privileges).
pub struct PrivilegeCatalog {
    /// MySQL: privileges on `*.*`. PostgreSQL: role attributes.
    pub server: &'static [&'static str],
    /// Privileges grantable on a database (`db.*` / `DATABASE x`).
    pub database: &'static [&'static str],
    /// Privileges grantable on a table (`db.t` / `TABLE s.t`).
    pub table: &'static [&'static str],
    /// PostgreSQL schema privileges.
    pub schema: &'static [&'static str],
}

const MYSQL_SERVER: &[&str] = &[
    "ALTER",
    "ALTER ROUTINE",
    "CREATE",
    "CREATE ROLE",
    "CREATE ROUTINE",
    "CREATE TABLESPACE",
    "CREATE TEMPORARY TABLES",
    "CREATE USER",
    "CREATE VIEW",
    "DELETE",
    "DROP",
    "DROP ROLE",
    "EVENT",
    "EXECUTE",
    "FILE",
    "INDEX",
    "INSERT",
    "LOCK TABLES",
    "PROCESS",
    "REFERENCES",
    "RELOAD",
    "REPLICATION CLIENT",
    "REPLICATION SLAVE",
    "SELECT",
    "SHOW DATABASES",
    "SHOW VIEW",
    "SHUTDOWN",
    "SUPER",
    "TRIGGER",
    "UPDATE",
];
const MYSQL_DB: &[&str] = &[
    "ALTER",
    "ALTER ROUTINE",
    "CREATE",
    "CREATE ROUTINE",
    "CREATE TEMPORARY TABLES",
    "CREATE VIEW",
    "DELETE",
    "DROP",
    "EVENT",
    "EXECUTE",
    "INDEX",
    "INSERT",
    "LOCK TABLES",
    "REFERENCES",
    "SELECT",
    "SHOW VIEW",
    "TRIGGER",
    "UPDATE",
];
const MYSQL_TABLE: &[&str] = &["ALTER", "CREATE", "CREATE VIEW", "DELETE", "DROP", "INDEX", "INSERT", "REFERENCES", "SELECT", "SHOW VIEW", "TRIGGER", "UPDATE"];
const PG_ATTRS: &[&str] = &["SUPERUSER", "CREATEDB", "CREATEROLE", "INHERIT", "REPLICATION", "BYPASSRLS"];
const PG_DB: &[&str] = &["CONNECT", "CREATE", "TEMPORARY"];
const PG_TABLE: &[&str] = &["DELETE", "INSERT", "REFERENCES", "SELECT", "TRIGGER", "TRUNCATE", "UPDATE"];
const PG_SCHEMA: &[&str] = &["CREATE", "USAGE"];

pub fn catalog(kind: DbKind) -> Result<PrivilegeCatalog> {
    match kind {
        DbKind::Mysql | DbKind::Mariadb => Ok(PrivilegeCatalog { server: MYSQL_SERVER, database: MYSQL_DB, table: MYSQL_TABLE, schema: &[] }),
        DbKind::Postgres => Ok(PrivilegeCatalog { server: PG_ATTRS, database: PG_DB, table: PG_TABLE, schema: PG_SCHEMA }),
        other => Err(unsupported(other)),
    }
}

fn unsupported(kind: DbKind) -> Error {
    Error::Unsupported(format!("user management is not available for {}", kind.label()))
}

fn is_mariadb(conn: &mut dyn Connection) -> bool {
    conn.kind() == DbKind::Mariadb || conn.server_version().is_ok_and(|v| v.to_ascii_lowercase().contains("mariadb"))
}

// ---------------------------------------------------------------------------------------------
// Reading

/// Every user and role, without grants (fast; for the list).
pub fn list(conn: &mut dyn Connection) -> Result<Vec<UserInfo>> {
    match conn.kind() {
        DbKind::Mysql | DbKind::Mariadb => {
            let maria = is_mariadb(conn);
            let sql = if maria {
                "select cast(user as char), cast(host as char), cast(plugin as char), is_role = 'Y', 0 from mysql.user order by 1, 2"
            } else {
                "select cast(user as char), cast(host as char), cast(plugin as char), \
                 (account_locked = 'Y' and authentication_string = ''), account_locked = 'Y' from mysql.user order by 1, 2"
            };
            let rs = conn.query(None, sql)?;
            Ok((0..rs.rows.len())
                .filter_map(|i| {
                    let mut u = UserInfo { name: cell_str(&rs, i, 0)?, host: cell_str(&rs, i, 1), is_role: cell_bool(&rs, i, 3), ..UserInfo::default() };
                    if let Some(p) = cell_str(&rs, i, 2).filter(|p| !p.is_empty()) {
                        u.settings.insert("plugin".into(), p);
                    }
                    if !maria {
                        u.attributes.insert("ACCOUNT LOCK".into(), cell_bool(&rs, i, 4));
                    }
                    Some(u)
                })
                .collect())
        }
        DbKind::Postgres => {
            let rs = conn.query(
                None,
                "select r.rolname::text, r.rolcanlogin, r.rolsuper, r.rolcreatedb, r.rolcreaterole, r.rolinherit, r.rolreplication, r.rolbypassrls, \
                 r.rolconnlimit::text, r.rolvaliduntil::text, \
                 array_to_string(array(select b.rolname from pg_auth_members m join pg_roles b on b.oid = m.roleid where m.member = r.oid order by 1), ',') \
                 from pg_roles r where r.rolname !~ '^pg_' order by 1",
            )?;
            Ok((0..rs.rows.len())
                .filter_map(|i| {
                    let mut u = UserInfo { name: cell_str(&rs, i, 0)?, is_role: !cell_bool(&rs, i, 1), ..UserInfo::default() };
                    for (j, attr) in PG_ATTRS.iter().enumerate() {
                        u.attributes.insert((*attr).to_string(), cell_bool(&rs, i, j + 2));
                    }
                    if let Some(l) = cell_str(&rs, i, 8).filter(|l| l != "-1") {
                        u.settings.insert("connection_limit".into(), l);
                    }
                    if let Some(v) = cell_str(&rs, i, 9) {
                        u.settings.insert("valid_until".into(), v);
                    }
                    u.member_of = cell_str(&rs, i, 10).unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
                    Some(u)
                })
                .collect())
        }
        other => Err(unsupported(other)),
    }
}

/// One user or role with its memberships and grants.
pub fn get(conn: &mut dyn Connection, name: &str, host: Option<&str>) -> Result<UserInfo> {
    let mut user = list(conn)?
        .into_iter()
        .find(|u| u.name == name && (host.is_none() || u.host.as_deref() == host))
        .ok_or_else(|| Error::NotFound(format!("user `{name}`")))?;
    match conn.kind() {
        DbKind::Mysql | DbKind::Mariadb => {
            let maria = is_mariadb(conn);
            let account = if maria && user.is_role { quote_ident(Dialect::Mysql, name) } else { mysql_account(name, user.host.as_deref().unwrap_or("%")) };
            let rs = conn.query(None, &format!("SHOW GRANTS FOR {account}"))?;
            let lines: Vec<String> = (0..rs.rows.len()).filter_map(|i| crate::common::cell_str(&rs, i, 0)).collect();
            let (grants, roles) = parse_mysql_grants(&lines);
            user.grants = grants;
            user.member_of = roles;
        }
        DbKind::Postgres => {
            let role = quote_literal(Dialect::Postgres, name);
            let grantee = format!("(select oid from pg_roles where rolname = {role})");
            let sql = format!(
                "select 'DATABASE ' || quote_ident(d.datname), string_agg(a.privilege_type, ',' order by a.privilege_type), bool_or(a.is_grantable) \
                 from pg_database d, aclexplode(d.datacl) a where a.grantee = {grantee} group by 1 \
                 union all \
                 select 'SCHEMA ' || quote_ident(n.nspname), string_agg(a.privilege_type, ',' order by a.privilege_type), bool_or(a.is_grantable) \
                 from pg_namespace n, aclexplode(n.nspacl) a where a.grantee = {grantee} group by 1 \
                 union all \
                 select 'TABLE ' || quote_ident(n.nspname) || '.' || quote_ident(c.relname), string_agg(a.privilege_type, ',' order by a.privilege_type), bool_or(a.is_grantable) \
                 from pg_class c join pg_namespace n on n.oid = c.relnamespace, aclexplode(c.relacl) a \
                 where a.grantee = {grantee} and c.relkind in ('r','v','m','p','f') group by 1 order by 1"
            );
            let rs = conn.query(None, &sql)?;
            user.grants = (0..rs.rows.len())
                .filter_map(|i| {
                    Some(Grant {
                        object: cell_str(&rs, i, 0)?,
                        privileges: cell_str(&rs, i, 1).unwrap_or_default().split(',').map(str::to_string).collect(),
                        grant_option: cell_bool(&rs, i, 2),
                    })
                })
                .collect();
        }
        other => return Err(unsupported(other)),
    }
    Ok(user)
}

/// Split at commas outside parentheses and backticks.
fn split_top(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut tick) = (Vec::new(), String::new(), 0i32, false);
    for c in s.chars() {
        match c {
            '`' => tick = !tick,
            '(' if !tick => depth += 1,
            ')' if !tick => depth -= 1,
            ',' if !tick && depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn unquote_mysql(s: &str) -> String {
    let s = s.trim();
    // `a`@`b` or 'a'@'b' or `db`.`t`
    s.replace(['`', '\''], "")
}

/// Parse `SHOW GRANTS` output into object grants and role memberships.
pub fn parse_mysql_grants(lines: &[String]) -> (Vec<Grant>, Vec<String>) {
    let mut grants: BTreeMap<String, Grant> = BTreeMap::new();
    let mut roles = Vec::new();
    for line in lines {
        let Some(rest) = line.strip_prefix("GRANT ") else { continue };
        match (rest.find(" ON "), rest.rfind(" TO ")) {
            (Some(on), Some(to)) if on < to => {
                let privs = rest.get(..on).unwrap_or("");
                let object = unquote_mysql(rest.get(on + 4..to).unwrap_or(""));
                if privs.trim() == "PROXY" {
                    continue;
                }
                let grant_option = rest.contains("WITH GRANT OPTION");
                let list: Vec<String> = split_top(privs).into_iter().filter(|p| p != "USAGE").collect();
                if list.is_empty() && !grant_option {
                    continue;
                }
                let g = grants.entry(object.clone()).or_insert_with(|| Grant { object, ..Grant::default() });
                for p in list {
                    if !g.privileges.contains(&p) {
                        g.privileges.push(p);
                    }
                }
                g.grant_option |= grant_option;
            }
            (None, Some(to)) => {
                // GRANT `role`@`%`, `r2`@`%` TO `user`@`%`
                roles.extend(split_top(rest.get(..to).unwrap_or("")).iter().map(|r| unquote_mysql(r)));
            }
            _ => {}
        }
    }
    let mut v: Vec<Grant> = grants.into_values().collect();
    for g in &mut v {
        g.privileges.sort();
    }
    (v, roles)
}

// ---------------------------------------------------------------------------------------------
// Writing

fn mysql_account(name: &str, host: &str) -> String {
    format!("{}@{}", quote_literal(Dialect::Mysql, name), quote_literal(Dialect::Mysql, host))
}

/// `r@%` → `'r'@'%'`; `r` (MariaDB role) → `` `r` ``.
fn mysql_role_ref(role: &str) -> String {
    match role.rsplit_once('@') {
        Some((n, h)) => mysql_account(n, h),
        None => quote_ident(Dialect::Mysql, role),
    }
}

/// `db.*` → `` `db`.* ``, `*.*` stays.
fn mysql_object(obj: &str) -> Result<String> {
    let (db, t) = obj.split_once('.').ok_or_else(|| Error::InvalidParam(format!("object `{obj}` must be db.table, db.* or *.*")))?;
    let q = |p: &str| if p == "*" { "*".to_string() } else { quote_ident(Dialect::Mysql, p) };
    Ok(format!("{}.{}", q(db), q(t)))
}

/// `TABLE s.t` / `DATABASE d` / `SCHEMA s` → keyword + quoted name.
fn pg_object(obj: &str) -> Result<String> {
    let (kw, name) =
        obj.trim().split_once(' ').ok_or_else(|| Error::InvalidParam(format!("object `{obj}` must be `TABLE s.t`, `DATABASE d` or `SCHEMA s`")))?;
    let kw = kw.to_ascii_uppercase();
    if !matches!(kw.as_str(), "TABLE" | "DATABASE" | "SCHEMA") {
        return Err(Error::InvalidParam(format!("unknown object type `{kw}`")));
    }
    let name = name.trim().replace('"', "");
    let quoted = name.split('.').map(|p| quote_ident(Dialect::Postgres, p)).collect::<Vec<_>>().join(".");
    Ok(format!("{kw} {quoted}"))
}

fn check_privileges(privs: &[String]) -> Result<()> {
    for p in privs {
        // Keywords, optionally with a backtick-quoted column list: `UPDATE (`a`, `b`)`. Nothing that
        // could end the statement or start a comment.
        let ok_chars = p.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '(' | ')' | '`' | ','));
        if p.trim().is_empty() || !ok_chars || p.contains("--") || !p.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return Err(Error::InvalidParam(format!("bad privilege name `{p}`")));
        }
    }
    Ok(())
}

/// Per object: (object, privileges to grant, privileges to revoke, grant-option change).
type GrantDiff = (String, Vec<String>, Vec<String>, Option<bool>);

fn grant_diff(old: &[Grant], new: &[Grant]) -> Vec<GrantDiff> {
    // (object, to grant, to revoke, grant option change)
    let mut objects: Vec<&str> = old.iter().chain(new).map(|g| g.object.as_str()).collect();
    objects.sort_unstable();
    objects.dedup();
    let empty = Grant::default();
    objects
        .into_iter()
        .map(|o| {
            let a = old.iter().find(|g| g.object == o).unwrap_or(&empty);
            let b = new.iter().find(|g| g.object == o).unwrap_or(&empty);
            let add = b.privileges.iter().filter(|p| !a.privileges.contains(p)).cloned().collect();
            let del = a.privileges.iter().filter(|p| !b.privileges.contains(p)).cloned().collect();
            let opt = (a.grant_option != b.grant_option).then_some(b.grant_option);
            (o.to_string(), add, del, opt)
        })
        .collect()
}

/// Statements that turn `old` (or nothing) into `new`. `password` is set when given.
pub fn save_sql(kind: DbKind, mariadb: bool, old: Option<&UserInfo>, new: &UserInfo, password: Option<&str>) -> Result<Vec<String>> {
    if new.name.trim().is_empty() {
        return Err(Error::InvalidParam("user name is required".into()));
    }
    for g in &new.grants {
        check_privileges(&g.privileges)?;
    }
    let mut out = Vec::new();
    match kind {
        DbKind::Mysql | DbKind::Mariadb => {
            let host = new.host.clone().unwrap_or_else(|| "%".into());
            let me = if mariadb && new.is_role { quote_ident(Dialect::Mysql, &new.name) } else { mysql_account(&new.name, &host) };
            match old {
                None if new.is_role => out.push(format!("CREATE ROLE {me}")),
                None => {
                    let mut s = format!("CREATE USER {me}");
                    if let Some(plugin) = new.settings.get("plugin").filter(|p| !p.is_empty()) {
                        if !plugin.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                            return Err(Error::InvalidParam(format!("bad plugin name `{plugin}`")));
                        }
                        s.push_str(&format!(" IDENTIFIED WITH {plugin}"));
                        if let Some(pw) = password {
                            s.push_str(&format!(" BY {}", quote_literal(Dialect::Mysql, pw)));
                        }
                    } else if let Some(pw) = password {
                        s.push_str(&format!(" IDENTIFIED BY {}", quote_literal(Dialect::Mysql, pw)));
                    }
                    if !mariadb && new.attributes.get("ACCOUNT LOCK") == Some(&true) {
                        s.push_str(" ACCOUNT LOCK");
                    }
                    out.push(s);
                }
                Some(o) => {
                    let old_host = o.host.clone().unwrap_or_else(|| "%".into());
                    if o.name != new.name || old_host != host {
                        out.push(format!("RENAME USER {} TO {me}", mysql_account(&o.name, &old_host)));
                    }
                    if let Some(pw) = password.filter(|_| !new.is_role) {
                        out.push(format!("ALTER USER {me} IDENTIFIED BY {}", quote_literal(Dialect::Mysql, pw)));
                    }
                    let lock_old = o.attributes.get("ACCOUNT LOCK").copied().unwrap_or(false);
                    let lock_new = new.attributes.get("ACCOUNT LOCK").copied().unwrap_or(false);
                    if !mariadb && !new.is_role && lock_old != lock_new {
                        out.push(format!("ALTER USER {me} ACCOUNT {}", if lock_new { "LOCK" } else { "UNLOCK" }));
                    }
                }
            }
            let old_roles = old.map(|o| o.member_of.clone()).unwrap_or_default();
            for r in new.member_of.iter().filter(|r| !old_roles.contains(r)) {
                out.push(format!("GRANT {} TO {me}", mysql_role_ref(r)));
            }
            for r in old_roles.iter().filter(|r| !new.member_of.contains(r)) {
                out.push(format!("REVOKE {} FROM {me}", mysql_role_ref(r)));
            }
            for (obj, add, del, opt) in grant_diff(old.map(|o| o.grants.as_slice()).unwrap_or(&[]), &new.grants) {
                let target = mysql_object(&obj)?;
                if !add.is_empty() {
                    let with = if new.grants.iter().any(|g| g.object == obj && g.grant_option) { " WITH GRANT OPTION" } else { "" };
                    out.push(format!("GRANT {} ON {target} TO {me}{with}", add.join(", ")));
                } else if opt == Some(true) {
                    let privs = new.grants.iter().find(|g| g.object == obj).map(|g| g.privileges.join(", ")).unwrap_or_default();
                    out.push(format!("GRANT {} ON {target} TO {me} WITH GRANT OPTION", if privs.is_empty() { "USAGE".to_string() } else { privs }));
                }
                if !del.is_empty() {
                    out.push(format!("REVOKE {} ON {target} FROM {me}", del.join(", ")));
                }
                if opt == Some(false) {
                    out.push(format!("REVOKE GRANT OPTION ON {target} FROM {me}"));
                }
            }
        }
        DbKind::Postgres => {
            let me = quote_ident(Dialect::Postgres, &new.name);
            let attr = |u: &UserInfo, a: &str| u.attributes.get(a).copied().unwrap_or(a == "INHERIT");
            let mut opts: Vec<String> = Vec::new();
            let login_changed = old.is_none_or(|o| o.is_role != new.is_role);
            if login_changed {
                opts.push(if new.is_role { "NOLOGIN".into() } else { "LOGIN".into() });
            }
            for a in PG_ATTRS {
                if old.is_none_or(|o| attr(o, a) != attr(new, a)) {
                    opts.push(if attr(new, a) { (*a).to_string() } else { format!("NO{a}") });
                }
            }
            let limit = new.settings.get("connection_limit").filter(|l| !l.is_empty());
            if old.is_none_or(|o| o.settings.get("connection_limit").filter(|l| !l.is_empty()) != limit) {
                let n: i64 = limit
                    .map(|l| l.trim().parse().map_err(|_| Error::InvalidParam(format!("connection limit `{l}` is not a number"))))
                    .transpose()?
                    .unwrap_or(-1);
                opts.push(format!("CONNECTION LIMIT {n}"));
            }
            let until = new.settings.get("valid_until").filter(|v| !v.is_empty());
            if old.is_none_or(|o| o.settings.get("valid_until").filter(|v| !v.is_empty()) != until) {
                opts.push(match until {
                    Some(v) => format!("VALID UNTIL {}", quote_literal(Dialect::Postgres, v)),
                    None => "VALID UNTIL 'infinity'".into(),
                });
            }
            if let Some(pw) = password {
                opts.push(format!("PASSWORD {}", quote_literal(Dialect::Postgres, pw)));
            }
            match old {
                None => out.push(format!("CREATE ROLE {me}{}", if opts.is_empty() { String::new() } else { format!(" WITH {}", opts.join(" ")) })),
                Some(o) => {
                    if o.name != new.name {
                        out.push(format!("ALTER ROLE {} RENAME TO {me}", quote_ident(Dialect::Postgres, &o.name)));
                    }
                    if !opts.is_empty() {
                        out.push(format!("ALTER ROLE {me} WITH {}", opts.join(" ")));
                    }
                }
            }
            let old_roles = old.map(|o| o.member_of.clone()).unwrap_or_default();
            for r in new.member_of.iter().filter(|r| !old_roles.contains(r)) {
                out.push(format!("GRANT {} TO {me}", quote_ident(Dialect::Postgres, r)));
            }
            for r in old_roles.iter().filter(|r| !new.member_of.contains(r)) {
                out.push(format!("REVOKE {} FROM {me}", quote_ident(Dialect::Postgres, r)));
            }
            for (obj, add, del, opt) in grant_diff(old.map(|o| o.grants.as_slice()).unwrap_or(&[]), &new.grants) {
                let target = pg_object(&obj)?;
                let with = new.grants.iter().any(|g| g.object == obj && g.grant_option);
                if !add.is_empty() {
                    out.push(format!("GRANT {} ON {target} TO {me}{}", add.join(", "), if with { " WITH GRANT OPTION" } else { "" }));
                } else if opt == Some(true) {
                    let privs = new.grants.iter().find(|g| g.object == obj).map(|g| g.privileges.join(", ")).unwrap_or_default();
                    if !privs.is_empty() {
                        out.push(format!("GRANT {privs} ON {target} TO {me} WITH GRANT OPTION"));
                    }
                }
                if !del.is_empty() {
                    out.push(format!("REVOKE {} ON {target} FROM {me}", del.join(", ")));
                }
                if opt == Some(false) {
                    let privs = new.grants.iter().find(|g| g.object == obj).map(|g| g.privileges.join(", ")).unwrap_or_default();
                    if !privs.is_empty() {
                        out.push(format!("REVOKE GRANT OPTION FOR {privs} ON {target} FROM {me}"));
                    }
                }
            }
        }
        other => return Err(unsupported(other)),
    }
    Ok(out)
}

/// Statements to delete a user or role. PostgreSQL refuses to drop a role that still holds
/// privileges, so its known grants are revoked first; objects it owns are left alone (the drop
/// then fails with the server's message, as it should).
pub fn drop_sql(kind: DbKind, mariadb: bool, user: &UserInfo) -> Result<Vec<String>> {
    match kind {
        DbKind::Mysql | DbKind::Mariadb if user.is_role && mariadb => Ok(vec![format!("DROP ROLE {}", quote_ident(Dialect::Mysql, &user.name))]),
        DbKind::Mysql | DbKind::Mariadb => {
            let acc = mysql_account(&user.name, user.host.as_deref().unwrap_or("%"));
            Ok(vec![if user.is_role { format!("DROP ROLE {acc}") } else { format!("DROP USER {acc}") }])
        }
        DbKind::Postgres => {
            let me = quote_ident(Dialect::Postgres, &user.name);
            let mut out = Vec::new();
            for g in &user.grants {
                out.push(format!("REVOKE ALL ON {} FROM {me}", pg_object(&g.object)?));
            }
            out.push(format!("DROP ROLE {me}"));
            Ok(out)
        }
        other => Err(unsupported(other)),
    }
}

/// Whether the server behind a MySQL-protocol connection is MariaDB (its role syntax differs).
pub fn server_is_mariadb(conn: &mut dyn Connection) -> bool {
    is_mariadb(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> UserInfo {
        UserInfo { name: name.into(), host: Some("%".into()), ..UserInfo::default() }
    }

    #[test]
    fn parses_show_grants() {
        let lines: Vec<String> = [
            "GRANT USAGE ON *.* TO `app`@`%`",
            "GRANT SELECT, INSERT, UPDATE (`a`, `b`) ON `shop`.* TO `app`@`%` WITH GRANT OPTION",
            "GRANT APPLICATION_PASSWORD_ADMIN,AUDIT_ADMIN ON *.* TO `app`@`%`",
            "GRANT `reader`@`%`,`writer`@`%` TO `app`@`%`",
            "GRANT PROXY ON ''@'' TO 'root'@'localhost' WITH GRANT OPTION",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (grants, roles) = parse_mysql_grants(&lines);
        assert_eq!(roles, ["reader@%", "writer@%"]);
        assert_eq!(grants.len(), 2);
        let shop = grants.iter().find(|g| g.object == "shop.*").unwrap();
        assert_eq!(shop.privileges, ["INSERT", "SELECT", "UPDATE (`a`, `b`)"]);
        assert!(shop.grant_option);
        assert_eq!(grants.iter().find(|g| g.object == "*.*").unwrap().privileges, ["APPLICATION_PASSWORD_ADMIN", "AUDIT_ADMIN"]);
    }

    #[test]
    fn mysql_create_alter_and_revoke() {
        let mut new = user("app");
        new.grants = vec![Grant { object: "shop.*".into(), privileges: vec!["SELECT".into(), "INSERT".into()], grant_option: false }];
        new.member_of = vec!["reader@%".into()];
        let sql = save_sql(DbKind::Mysql, false, None, &new, Some("p'w")).unwrap();
        assert_eq!(sql, ["CREATE USER 'app'@'%' IDENTIFIED BY 'p''w'", "GRANT 'reader'@'%' TO 'app'@'%'", "GRANT SELECT, INSERT ON `shop`.* TO 'app'@'%'"]);

        let old = new.clone();
        let mut changed = new.clone();
        changed.grants[0].privileges = vec!["SELECT".into(), "DELETE".into()];
        changed.member_of.clear();
        changed.attributes.insert("ACCOUNT LOCK".into(), true);
        let sql = save_sql(DbKind::Mysql, false, Some(&old), &changed, None).unwrap();
        assert_eq!(
            sql,
            [
                "ALTER USER 'app'@'%' ACCOUNT LOCK",
                "REVOKE 'reader'@'%' FROM 'app'@'%'",
                "GRANT DELETE ON `shop`.* TO 'app'@'%'",
                "REVOKE INSERT ON `shop`.* FROM 'app'@'%'"
            ]
        );
    }

    #[test]
    fn postgres_role_sql() {
        let mut new = UserInfo { name: "report".into(), is_role: true, ..UserInfo::default() };
        new.attributes.insert("CREATEDB".into(), true);
        new.grants = vec![Grant { object: "TABLE public.film".into(), privileges: vec!["SELECT".into()], grant_option: true }];
        let sql = save_sql(DbKind::Postgres, false, None, &new, None).unwrap();
        assert_eq!(
            sql[0],
            "CREATE ROLE \"report\" WITH NOLOGIN NOSUPERUSER CREATEDB NOCREATEROLE INHERIT NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 VALID UNTIL 'infinity'"
        );
        assert_eq!(sql[1], "GRANT SELECT ON TABLE \"public\".\"film\" TO \"report\" WITH GRANT OPTION");

        let mut renamed = new.clone();
        renamed.name = "reports".into();
        renamed.is_role = false;
        let sql = save_sql(DbKind::Postgres, false, Some(&new), &renamed, Some("x")).unwrap();
        assert_eq!(sql, ["ALTER ROLE \"report\" RENAME TO \"reports\"", "ALTER ROLE \"reports\" WITH LOGIN PASSWORD 'x'"]);
    }

    #[test]
    fn hostile_input_is_rejected_or_quoted() {
        let mut u = user("x");
        u.grants = vec![Grant { object: "db.*".into(), privileges: vec!["SELECT; DROP DATABASE db".into()], grant_option: false }];
        assert!(save_sql(DbKind::Mysql, false, None, &u, None).is_err());
        u.grants[0].privileges = vec!["SELECT".into()];
        u.grants[0].object = "nodot".into();
        assert!(save_sql(DbKind::Mysql, false, None, &u, None).is_err());
        let evil = UserInfo { name: "a'; drop table t; --".into(), ..UserInfo::default() };
        let sql = save_sql(DbKind::Postgres, false, None, &evil, None).unwrap();
        assert!(sql[0].starts_with("CREATE ROLE \"a'; drop table t; --\""));
        assert!(save_sql(DbKind::Sqlite, false, None, &evil, None).is_err());
        assert!(save_sql(DbKind::Mysql, false, None, &UserInfo::default(), None).is_err());
    }
}
