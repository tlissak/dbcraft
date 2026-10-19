//! Reading parts out of stored DDL text (SQLite keeps only `CREATE …` statements; PostgreSQL's
//! `pg_get_triggerdef` returns one). A small scanner that respects quotes, comments and nesting.

/// Byte ranges of the text outside quotes and comments, as (start, end, is_code) pieces.
fn code_mask(sql: &str) -> Vec<bool> {
    let b = sql.as_bytes();
    let mut mask = vec![true; b.len()];
    let mut i = 0;
    while i < b.len() {
        let c = b.get(i).copied().unwrap_or(0);
        let close = match c {
            b'\'' => Some(b'\''),
            b'"' => Some(b'"'),
            b'`' => Some(b'`'),
            b'[' => Some(b']'),
            _ => None,
        };
        if let Some(close) = close {
            let start = i;
            i += 1;
            while i < b.len() {
                if b.get(i) == Some(&close) {
                    // Doubled quote inside a literal.
                    if close != b']' && b.get(i + 1) == Some(&close) {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            for m in mask.iter_mut().take((i + 1).min(b.len())).skip(start) {
                *m = false;
            }
            i += 1;
        } else if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b.get(i) != Some(&b'\n') {
                if let Some(m) = mask.get_mut(i) {
                    *m = false;
                }
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let start = i;
            i += 2;
            while i + 1 < b.len() && !(b.get(i) == Some(&b'*') && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            for m in mask.iter_mut().take(i).skip(start) {
                *m = false;
            }
        } else {
            i += 1;
        }
    }
    mask
}

/// Positions where `word` (case-insensitive, whole word) appears outside quotes and comments.
pub fn find_words(sql: &str, word: &str) -> Vec<usize> {
    let mask = code_mask(sql);
    let lower = sql.to_ascii_lowercase();
    let w = word.to_ascii_lowercase();
    let b = lower.as_bytes();
    let ident = |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = lower.get(from..).and_then(|s| s.find(&w)) {
        let at = from + rel;
        let before = at.checked_sub(1).and_then(|i| b.get(i));
        if mask.get(at).copied().unwrap_or(false) && !ident(before) && !ident(b.get(at + w.len())) {
            out.push(at);
        }
        from = at + w.len().max(1);
    }
    out
}

/// The text inside the parentheses that open at or after `from` (balanced, quote-aware), and the
/// index just past the closing parenthesis.
pub fn parens_after(sql: &str, from: usize) -> Option<(String, usize)> {
    let mask = code_mask(sql);
    let b = sql.as_bytes();
    let open = (from..b.len()).find(|&i| b.get(i) == Some(&b'(') && mask.get(i).copied().unwrap_or(false))?;
    let mut depth = 0i32;
    for i in open..b.len() {
        if !mask.get(i).copied().unwrap_or(false) {
            continue;
        }
        match b.get(i) {
            Some(b'(') => depth += 1,
            Some(b')') => {
                depth -= 1;
                if depth == 0 {
                    return Some((sql.get(open + 1..i)?.trim().to_string(), i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Identifier text without its quotes.
pub fn unquote_ident(s: &str) -> String {
    let s = s.trim();
    for (o, c) in [('"', '"'), ('`', '`'), ('[', ']')] {
        if let Some(inner) = s.strip_prefix(o).and_then(|r| r.strip_suffix(c)) {
            return inner.to_string();
        }
    }
    s.to_string()
}

/// `CHECK (...)` clauses in a `CREATE TABLE` (column or table level), with the name of a
/// directly preceding `CONSTRAINT name`, if any.
pub fn checks(create: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    for at in find_words(create, "check") {
        let Some((expr, _)) = parens_after(create, at) else { continue };
        // `CONSTRAINT name CHECK`: look at the two words before.
        let before = create.get(..at).unwrap_or("").trim_end();
        let mut words = before.rsplitn(3, char::is_whitespace);
        let name = words.next().map(str::to_string);
        let kw = words.next().map(str::to_ascii_lowercase);
        let name = if kw.as_deref() == Some("constraint") { name.map(|n| unquote_ident(&n)) } else { None };
        out.push((name, expr));
    }
    out
}

/// (timing, events, for each row, condition, body).
pub type TriggerParts = (String, Vec<String>, bool, Option<String>, String);

/// Parts of a `CREATE TRIGGER`.
pub fn trigger_parts(create: &str) -> Option<TriggerParts> {
    let lower = create.to_ascii_lowercase();
    let on = *find_words(create, "on").first()?;
    let header = lower.get(..on)?;
    let timing = if header.contains("instead of") {
        "INSTEAD OF"
    } else if !find_words(header, "before").is_empty() {
        "BEFORE"
    } else {
        "AFTER"
    };
    let mut events = Vec::new();
    for e in ["insert", "update", "delete", "truncate"] {
        if !find_words(header, e).is_empty() {
            // `UPDATE OF a, b` keeps its column list.
            if e == "update"
                && let Some(of) = find_words(header, "of").into_iter().find(|&p| find_words(header, "update").first().is_some_and(|&u| p > u))
            {
                let cols = create.get(of + 2..on).unwrap_or("").trim();
                events.push(format!("UPDATE OF {cols}"));
                continue;
            }
            events.push(e.to_ascii_uppercase());
        }
    }
    let for_each_row = lower.get(on..).is_some_and(|rest| rest.contains("for each row"));
    // Body: from BEGIN (SQLite, MySQL) or EXECUTE (PostgreSQL).
    let body_at = find_words(create, "begin").into_iter().chain(find_words(create, "execute")).filter(|&p| p > on).min()?;
    let condition = find_words(create, "when").into_iter().find(|&w| w > on && w < body_at).and_then(|w| {
        let cond = create.get(w + 4..body_at)?.trim();
        let cond = cond.strip_prefix('(').and_then(|c| c.strip_suffix(')')).unwrap_or(cond);
        Some(cond.trim().to_string())
    });
    let body = create.get(body_at..)?.trim().trim_end_matches(';').trim().to_string();
    Some((timing.to_string(), events, for_each_row, condition, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_checks_outside_quotes() {
        let sql = "CREATE TABLE t (a int CHECK (a > 0), b text default 'check (no)', CONSTRAINT \"b_len\" CHECK (length(b) < 10 and (b <> ')')))";
        assert_eq!(checks(sql), vec![(None, "a > 0".to_string()), (Some("b_len".to_string()), "length(b) < 10 and (b <> ')')".to_string())]);
        assert!(checks("CREATE TABLE t (checked int)").is_empty());
    }

    #[test]
    fn splits_triggers() {
        let (timing, events, row, cond, body) =
            trigger_parts("CREATE TRIGGER trg AFTER UPDATE OF price, title ON film FOR EACH ROW WHEN new.price > 0 BEGIN UPDATE log SET n = n + 1; END")
                .unwrap();
        assert_eq!((timing.as_str(), events, row), ("AFTER", vec!["UPDATE OF price, title".to_string()], true));
        assert_eq!(cond.as_deref(), Some("new.price > 0"));
        assert_eq!(body, "BEGIN UPDATE log SET n = n + 1; END");
        let pg = trigger_parts("CREATE TRIGGER t BEFORE INSERT OR DELETE ON public.film FOR EACH ROW WHEN ((new.id > 0)) EXECUTE FUNCTION stamp()").unwrap();
        assert_eq!(
            (pg.0.as_str(), pg.1, pg.3.as_deref(), pg.4.as_str()),
            ("BEFORE", vec!["INSERT".to_string(), "DELETE".to_string()], Some("(new.id > 0)"), "EXECUTE FUNCTION stamp()")
        );
        assert!(trigger_parts("nonsense").is_none());
    }

    #[test]
    fn parens_and_words() {
        assert_eq!(parens_after("x (a (b) ')') y", 0), Some(("a (b) ')'".to_string(), 13)));
        assert_eq!(find_words("on 'on' -- on\n /* on */ ON", "on"), vec![0, 24]);
    }
}
