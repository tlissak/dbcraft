//! SQL text utilities that need no database: splitting a script into statements and quoting
//! identifiers per dialect. Standalone (no workspace deps) so it stays reusable and fuzzable.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `"ident"`, standard strings.
    Sqlite,
    /// `"ident"`, `$tag$...$tag$` bodies, `bytea` hex literals.
    Postgres,
    /// MySQL / MariaDB: `` `ident` ``, `#` comments, backslash escapes in strings.
    Mysql,
    /// SQL Server: `[ident]`.
    Mssql,
}

/// Quote an identifier for the dialect, escaping the closing quote character.
pub fn quote_ident(dialect: Dialect, name: &str) -> String {
    match dialect {
        Dialect::Sqlite | Dialect::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
        Dialect::Mysql => format!("`{}`", name.replace('`', "``")),
        Dialect::Mssql => format!("[{}]", name.replace(']', "]]")),
    }
}

/// Quote a string literal for the dialect. MySQL also treats backslash as an escape, so it is
/// doubled there. The result is always a single, closed literal, whatever the input.
pub fn quote_literal(dialect: Dialect, value: &str) -> String {
    let mut s = value.replace('\'', "''");
    if dialect == Dialect::Mysql {
        s = s.replace('\\', "\\\\");
    }
    // NUL can't appear in PostgreSQL text and truncates C strings elsewhere.
    s.retain(|c| c != '\0');
    match dialect {
        Dialect::Mssql => format!("N'{s}'"),
        _ => format!("'{s}'"),
    }
}

/// A binary literal (`X'0aff'`, PostgreSQL `'\x0aff'::bytea`, SQL Server `0x0aff`).
pub fn bytes_literal(dialect: Dialect, bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    match dialect {
        Dialect::Postgres => format!("'\\x{hex}'::bytea"),
        Dialect::Mssql => format!("0x{hex}"),
        Dialect::Sqlite | Dialect::Mysql => format!("X'{hex}'"),
    }
}

/// Escape `%`, `_` and the escape char itself for use inside `LIKE '...' ESCAPE '!'`.
pub fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '%' | '_' | '!') {
            out.push('!');
        }
        out.push(c);
    }
    out
}

/// Split a script on top-level `;`, ignoring semicolons inside strings, quoted identifiers,
/// comments, PostgreSQL dollar-quoted bodies and `BEGIN … END` bodies of `CREATE TRIGGER /
/// PROCEDURE / FUNCTION / EVENT`. Returns trimmed, non-empty statements without the trailing `;`.
/// Never panics: unterminated quotes or comments run to the end of the input.
pub fn split_statements(dialect: Dialect, sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let at = |j: usize| chars.get(j).copied();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    // Open BEGIN/CASE blocks inside a compound statement's body.
    let mut depth: i32 = 0;
    // The previous keyword was END (to recognise `END IF`, `END LOOP`…).
    let mut after_end = false;

    while let Some(c) = at(i) {
        match c {
            '\'' | '"' | '`' => {
                cur.push(c);
                i += 1;
                while let Some(d) = at(i) {
                    cur.push(d);
                    i += 1;
                    if d == c {
                        if at(i) == Some(c) {
                            cur.push(c);
                            i += 1;
                        } else {
                            break;
                        }
                    } else if d == '\\'
                        && dialect == Dialect::Mysql
                        && let Some(e) = at(i)
                    {
                        cur.push(e);
                        i += 1;
                    }
                }
            }
            '[' if dialect == Dialect::Mssql => {
                while let Some(d) = at(i) {
                    cur.push(d);
                    i += 1;
                    if d == ']' {
                        break;
                    }
                }
            }
            '-' if at(i + 1) == Some('-') => i = take_line(&chars, i, &mut cur),
            '#' if dialect == Dialect::Mysql => i = take_line(&chars, i, &mut cur),
            '/' if at(i + 1) == Some('*') => {
                cur.push_str("/*");
                i += 2;
                while let Some(d) = at(i) {
                    cur.push(d);
                    i += 1;
                    if d == '*' && at(i) == Some('/') {
                        cur.push('/');
                        i += 1;
                        break;
                    }
                }
            }
            '$' if dialect == Dialect::Postgres => {
                // $tag$ ... $tag$ (the tag may be empty).
                let mut j = i + 1;
                while at(j).is_some_and(|d| d.is_alphanumeric() || d == '_') {
                    j += 1;
                }
                let tag: Option<Vec<char>> = if at(j) == Some('$') { chars.get(i..=j).map(<[char]>::to_vec) } else { None };
                match tag {
                    Some(tag) => {
                        cur.extend(tag.iter());
                        i = j + 1;
                        while let Some(d) = at(i) {
                            if chars.get(i..i.saturating_add(tag.len())) == Some(&tag[..]) {
                                cur.extend(tag.iter());
                                i += tag.len();
                                break;
                            }
                            cur.push(d);
                            i += 1;
                        }
                    }
                    None => {
                        cur.push(c);
                        i += 1;
                    }
                }
            }
            ';' if depth > 0 => {
                cur.push(c);
                i += 1;
            }
            ';' => {
                push_stmt(&mut out, &mut cur);
                i += 1;
                after_end = false;
            }
            c if (c.is_alphabetic() || c == '_') && !i.checked_sub(1).and_then(at).is_some_and(|p| p.is_alphanumeric() || p == '_') => {
                let start = i;
                while at(i).is_some_and(|d| d.is_alphanumeric() || d == '_') {
                    i += 1;
                }
                let word: String = chars.get(start..i).map(|w| w.iter().collect()).unwrap_or_default();
                let upper = word.to_ascii_uppercase();
                let closes_other = matches!(upper.as_str(), "IF" | "LOOP" | "WHILE" | "REPEAT");
                if after_end && closes_other {
                    // `END IF` etc.: the END closed this keyword's block, not a BEGIN.
                    depth += 1;
                }
                after_end = false;
                match upper.as_str() {
                    "BEGIN" if depth > 0 || is_compound(&cur) => depth += 1,
                    "CASE" if depth > 0 => depth += 1,
                    "END" if depth > 0 => {
                        depth -= 1;
                        after_end = true;
                    }
                    _ => {}
                }
                cur.push_str(&word);
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    push_stmt(&mut out, &mut cur);
    out
}

/// A `CREATE [OR REPLACE] [DEFINER=…] [TEMP] TRIGGER / PROCEDURE / FUNCTION / EVENT` statement,
/// whose body may hold `;`-separated statements between BEGIN and END.
fn is_compound(stmt: &str) -> bool {
    let code: String = stmt.lines().filter(|l| !l.trim_start().starts_with("--")).collect::<Vec<_>>().join(" ").to_ascii_uppercase();
    let mut words = code.split_whitespace();
    words.next() == Some("CREATE") && words.take(6).any(|w| matches!(w, "TRIGGER" | "PROCEDURE" | "FUNCTION" | "EVENT"))
}

fn take_line(chars: &[char], mut i: usize, cur: &mut String) -> usize {
    while let Some(&d) = chars.get(i) {
        cur.push(d);
        i += 1;
        if d == '\n' {
            break;
        }
    }
    i
}

fn push_stmt(out: &mut Vec<String>, cur: &mut String) {
    let s = cur.trim();
    if !s.is_empty() && !is_only_line_comments(s) {
        out.push(s.to_string());
    }
    cur.clear();
}

fn is_only_line_comments(s: &str) -> bool {
    s.lines().all(|l| {
        let l = l.trim();
        l.is_empty() || l.starts_with("--") || l.starts_with('#')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_compound_bodies_whole() {
        let s = "create trigger t after insert on film begin insert into log values (new.title); update n set c = c + 1; end; select 1;";
        assert_eq!(
            split_statements(Dialect::Sqlite, s),
            vec!["create trigger t after insert on film begin insert into log values (new.title); update n set c = c + 1; end", "select 1"]
        );
        let my = "CREATE PROCEDURE p() BEGIN IF x THEN SELECT CASE WHEN a THEN 1 ELSE 2 END; END IF; WHILE y DO SET y = 0; END WHILE; END; SELECT 2";
        assert_eq!(split_statements(Dialect::Mysql, my).len(), 2, "{:?}", split_statements(Dialect::Mysql, my));
        // A transaction BEGIN is not a body.
        assert_eq!(split_statements(Dialect::Sqlite, "begin; insert into t values (1); commit;").len(), 3);
        // `end` inside strings and identifiers doesn't count.
        assert_eq!(split_statements(Dialect::Sqlite, "create trigger t after insert on x begin select 'end'; select ending; end; select 3").len(), 2);
        // Unbalanced: runs to the end, never panics.
        assert_eq!(split_statements(Dialect::Sqlite, "create trigger t after insert on x begin select 1;").len(), 1);
    }

    #[test]
    fn splits_simple_script() {
        assert_eq!(split_statements(Dialect::Postgres, "select 1; select 2;"), vec!["select 1", "select 2"]);
    }

    #[test]
    fn ignores_semicolons_in_strings_and_comments() {
        let s = "insert into t values ('a;b'); -- c;d\nselect \"x;y\" from t /* ; */;";
        assert_eq!(split_statements(Dialect::Postgres, s), vec!["insert into t values ('a;b')", "-- c;d\nselect \"x;y\" from t /* ; */"]);
    }

    #[test]
    fn handles_dollar_quotes() {
        let s = "create function f() returns int as $$ begin return 1; end $$ language plpgsql; select 1";
        let v = split_statements(Dialect::Postgres, s);
        assert_eq!(v.len(), 2);
        assert!(v[0].ends_with("plpgsql"));
    }

    #[test]
    fn mysql_backslash_escape_and_hash_comment() {
        let v = split_statements(Dialect::Mysql, "select 'it\\'s;'; # x;\nselect 2");
        assert_eq!(v, vec!["select 'it\\'s;'", "# x;\nselect 2"]);
    }

    #[test]
    fn drops_comment_only_trailers() {
        assert_eq!(split_statements(Dialect::Postgres, "select 1; -- done"), vec!["select 1"]);
    }

    #[test]
    fn unterminated_input_never_panics() {
        for s in ["'abc", "\"x", "/* open", "$tag$ body", "$$", "[ab", "--", "$", ";;;", "", "é'ü;"] {
            for d in [Dialect::Sqlite, Dialect::Postgres, Dialect::Mysql, Dialect::Mssql] {
                let _ = split_statements(d, s);
            }
        }
    }

    #[test]
    fn quotes_identifiers() {
        assert_eq!(quote_ident(Dialect::Postgres, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident(Dialect::Mysql, "a`b"), "`a``b`");
        assert_eq!(quote_ident(Dialect::Mssql, "a]b"), "[a]]b]");
    }

    #[test]
    fn quotes_literals_safely() {
        assert_eq!(quote_literal(Dialect::Postgres, "it's"), "'it''s'");
        assert_eq!(quote_literal(Dialect::Mysql, r"a\' or 1=1 --"), r"'a\\'' or 1=1 --'");
        assert_eq!(quote_literal(Dialect::Sqlite, "nul\0x"), "'nulx'");
        assert_eq!(bytes_literal(Dialect::Postgres, &[1, 255]), r"'\x01ff'::bytea");
        assert_eq!(bytes_literal(Dialect::Mysql, &[1, 255]), "X'01ff'");
        assert_eq!(escape_like("50%_!"), "50!%!_!!");
        // A quoted literal never splits into two statements.
        for evil in ["'; drop table t; --", r"\'; drop table t; --", "''''", "$$; drop table t; $$"] {
            for d in [Dialect::Sqlite, Dialect::Postgres, Dialect::Mysql] {
                let sql = format!("select {}", quote_literal(d, evil));
                assert_eq!(split_statements(d, &sql).len(), 1, "{d:?} {evil}");
            }
        }
    }
}
