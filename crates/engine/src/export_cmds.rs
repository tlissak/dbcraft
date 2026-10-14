//! Export Wizard (only CSV of the current result so far; see ROADMAP.md).

use dbcraft_core::{Error, Result};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::CommandSpec;
use crate::params::str_req;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "export.resultCsv",
        label: "Export Result as CSV",
        menu: "Tools/Export Wizard",
        shortcut: None,
        params_doc: r#"{"path":str} -> {"rows":int} (exports the last result grid)"#,
        enabled: |s| s.last_result.is_some(),
        run: export_csv,
    }]
}

fn export_csv(s: &mut Session, p: &Json) -> Result<Json> {
    let path = str_req(p, "path")?;
    if path.trim().is_empty() {
        return Err(Error::InvalidParam("`path` must not be empty".into()));
    }
    let rs = s.last_result.as_ref().ok_or_else(|| Error::InvalidParam("no result to export".into()))?;
    std::fs::write(path, rs.to_csv())?;
    Ok(json!({ "rows": rs.rows.len() }))
}
