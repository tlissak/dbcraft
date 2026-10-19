//! How cells read in the grids (Tools › Options › Display formats): the NULL text, and strftime
//! formats for date, time and date-time columns. Values themselves never change (editing and
//! copying use the raw value); only what the grid shows.

use std::cell::RefCell;
use std::fmt::Write as _;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use dbcraft_core::Value;
use dbcraft_model::Options;

#[derive(Clone, Default)]
struct Formats {
    null_text: String,
    date: String,
    time: String,
    datetime: String,
}

thread_local! {
    static FORMATS: RefCell<Formats> = RefCell::new(Formats { null_text: "(NULL)".into(), ..Formats::default() });
}

/// Use these options' display formats on this (the UI) thread.
pub fn apply(o: &Options) {
    FORMATS.with(|f| {
        *f.borrow_mut() =
            Formats { null_text: o.null_text.clone(), date: o.date_format.clone(), time: o.time_format.clone(), datetime: o.datetime_format.clone() };
    });
}

/// What NULL reads as.
pub fn null_text() -> String {
    FORMATS.with(|f| f.borrow().null_text.clone())
}

/// A cell as the grid shows it, and whether it's a placeholder (NULL) drawn weak.
pub fn cell(v: &Value, type_name: &str) -> (String, bool) {
    match v {
        Value::Null => (null_text(), true),
        Value::Text(t) => (FORMATS.with(|f| formatted(&f.borrow(), t, type_name)).unwrap_or_else(|| t.clone()), false),
        other => (other.display(), false),
    }
}

/// `text` rendered with the format for its column type, when one is set and the text parses.
fn formatted(f: &Formats, text: &str, type_name: &str) -> Option<String> {
    let t = type_name.to_ascii_uppercase();
    let is_datetime = t.contains("TIMESTAMP") || t.contains("DATETIME");
    let is_date = !is_datetime && t.contains("DATE");
    let is_time = !is_datetime && t.contains("TIME");
    let s = text.trim();
    let mut out = String::new();
    // `write!` reports a bad format as an error instead of panicking like `to_string` would.
    let ok = if is_datetime && !f.datetime.is_empty() {
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            write!(out, "{}", dt.format(&f.datetime)).is_ok()
        } else {
            let dt = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")).ok()?;
            write!(out, "{}", dt.format(&f.datetime)).is_ok()
        }
    } else if is_date && !f.date.is_empty() {
        let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
        write!(out, "{}", d.format(&f.date)).is_ok()
    } else if is_time && !f.time.is_empty() {
        let tm = NaiveTime::parse_from_str(s, "%H:%M:%S%.f").ok()?;
        write!(out, "{}", tm.format(&f.time)).is_ok()
    } else {
        return None;
    };
    ok.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmts(date: &str, time: &str, datetime: &str) -> Formats {
        Formats { null_text: "∅".into(), date: date.into(), time: time.into(), datetime: datetime.into() }
    }

    #[test]
    fn formats_by_column_type() {
        let f = fmts("%d/%m/%Y", "%H:%M", "%d/%m/%Y %H:%M");
        assert_eq!(formatted(&f, "2026-10-19", "DATE").as_deref(), Some("19/10/2026"));
        assert_eq!(formatted(&f, "13:45:10.5", "TIME").as_deref(), Some("13:45"));
        assert_eq!(formatted(&f, "2026-10-19 13:45:10", "DATETIME").as_deref(), Some("19/10/2026 13:45"));
        assert_eq!(formatted(&f, "2026-10-19T13:45:10+02:00", "TIMESTAMPTZ").as_deref(), Some("19/10/2026 13:45"));
        // Not a date column, not parseable, or no format: left as is.
        assert_eq!(formatted(&f, "2026-10-19", "TEXT"), None);
        assert_eq!(formatted(&f, "soon", "DATE"), None);
        assert_eq!(formatted(&fmts("", "", ""), "2026-10-19", "DATE"), None);
    }

    #[test]
    fn a_bad_format_never_panics() {
        let f = fmts("%Q%", "%", "%Y-%");
        assert_eq!(formatted(&f, "2026-10-19", "DATE"), None);
        assert_eq!(formatted(&f, "10:00:00", "TIME"), None);
        assert_eq!(formatted(&f, "2026-10-19 10:00:00", "TIMESTAMP"), None);
    }

    #[test]
    fn null_text_follows_the_options() {
        let mut o = Options::default();
        assert_eq!(cell(&Value::Null, "INT"), ("(NULL)".to_string(), true));
        o.null_text = "<null>".into();
        apply(&o);
        assert_eq!(cell(&Value::Null, "INT"), ("<null>".to_string(), true));
        assert_eq!(cell(&Value::Int(3), "INT"), ("3".to_string(), false));
    }
}
