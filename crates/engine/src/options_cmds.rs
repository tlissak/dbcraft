//! Tools › Options: read and change the app's options (saved in the workspace).

use chrono::format::{Item, StrftimeItems};
use dbcraft_core::{Error, Result};
use dbcraft_model::Options;
use serde_json::Value as Json;

use crate::Session;
use crate::commands::{CommandSpec, always};

/// The engine's maximum page (see `data_cmds`): what "no limit" loads.
const MAX_PAGE: u64 = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "tools.options",
        label: "Options",
        menu: "Tools/Options",
        shortcut: None,
        params_doc: r#"{"set"?:{"theme"?:"light|dark","big_icons"?:bool,"show_caption"?:bool,"editor_font_size"?:8..40,"word_wrap"?:bool,"limit_records"?:bool,"page_size"?:1..100000,"confirm_close"?:bool,"null_text"?:str,"date_format"?:strftime,"time_format"?:strftime,"datetime_format"?:strftime},"reset"?:bool} -> Options"#,
        enabled: always,
        run: options,
    }]
}

fn options(s: &mut Session, p: &Json) -> Result<Json> {
    let reset = match p.get("reset") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(_) => return Err(Error::InvalidParam("`reset` must be a boolean".into())),
    };
    let mut changed = false;
    if reset {
        s.workspace.options = Options::default();
        changed = true;
    }
    match p.get("set") {
        None | Some(Json::Null) => {}
        Some(Json::Object(set)) => {
            let mut current = serde_json::to_value(&s.workspace.options).map_err(|e| Error::InvalidParam(e.to_string()))?;
            let Some(fields) = current.as_object_mut() else { return Err(Error::InvalidParam("options are not an object".into())) };
            for (k, v) in set {
                if !fields.contains_key(k) {
                    return Err(Error::InvalidParam(format!("unknown option `{k}`")));
                }
                fields.insert(k.clone(), v.clone());
            }
            let next: Options = serde_json::from_value(current).map_err(|e| Error::InvalidParam(format!("bad option value: {e}")))?;
            validate(&next)?;
            s.workspace.options = next;
            changed = true;
        }
        Some(_) => return Err(Error::InvalidParam("`set` must be an object".into())),
    }
    if changed {
        s.save_workspace()?;
    }
    serde_json::to_value(&s.workspace.options).map_err(|e| Error::InvalidParam(e.to_string()))
}

fn validate(o: &Options) -> Result<()> {
    if !matches!(o.theme.as_str(), "light" | "dark") {
        return Err(Error::InvalidParam(format!("theme must be `light` or `dark`, not `{}`", o.theme)));
    }
    if !(8.0..=40.0).contains(&o.editor_font_size) {
        return Err(Error::InvalidParam(format!("editor_font_size must be 8–40, not {}", o.editor_font_size)));
    }
    if !(1..=MAX_PAGE).contains(&o.page_size) {
        return Err(Error::InvalidParam(format!("page_size must be 1–{MAX_PAGE}, not {}", o.page_size)));
    }
    if o.null_text.chars().count() > 32 {
        return Err(Error::InvalidParam("null_text is limited to 32 characters".into()));
    }
    for (name, f) in [("date_format", &o.date_format), ("time_format", &o.time_format), ("datetime_format", &o.datetime_format)] {
        check_format(name, f)?;
    }
    Ok(())
}

/// Reject strftime strings chrono can't render (rendering them would fail at draw time).
pub fn check_format(name: &str, f: &str) -> Result<()> {
    if f.chars().count() > 64 {
        return Err(Error::InvalidParam(format!("{name} is limited to 64 characters")));
    }
    if StrftimeItems::new(f).any(|i| matches!(i, Item::Error)) {
        return Err(Error::InvalidParam(format!("{name} `{f}` is not a valid date/time format (use strftime codes like %Y-%m-%d)")));
    }
    Ok(())
}

/// Rows per page the table viewer should load.
pub fn page_size(o: &Options) -> u64 {
    if o.limit_records { o.page_size.clamp(1, MAX_PAGE) } else { MAX_PAGE }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_and_merges() {
        let mut s = Session::new();
        let v = options(&mut s, &json!({"set": {"theme": "dark", "page_size": 250}})).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(s.workspace.options.page_size, 250);
        assert!(s.workspace.options.big_icons, "unset fields keep their value");
        for bad in [
            json!({"set": {"theme": "neon"}}),
            json!({"set": {"page_size": 0}}),
            json!({"set": {"page_size": 10_000_000}}),
            json!({"set": {"editor_font_size": 400}}),
            json!({"set": {"date_format": "%Q"}}),
            json!({"set": {"nope": 1}}),
            json!({"set": {"big_icons": "yes"}}),
            json!({"set": 3}),
            json!({"reset": "x"}),
        ] {
            assert!(options(&mut s, &bad).is_err(), "{bad}");
        }
        assert_eq!(s.workspace.options.theme, "dark", "a rejected change leaves options alone");
        options(&mut s, &json!({"reset": true})).unwrap();
        assert_eq!(s.workspace.options, Options::default());
    }

    #[test]
    fn page_size_honours_the_limit_switch() {
        let mut o = Options::default();
        assert_eq!(page_size(&o), 1000);
        o.limit_records = false;
        assert_eq!(page_size(&o), MAX_PAGE);
    }
}
