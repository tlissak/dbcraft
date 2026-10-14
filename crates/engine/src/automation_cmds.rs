//! Automation: batch jobs (an ordered list of commands) run now or on a schedule. Scheduling uses
//! the Windows Task Scheduler, which runs `dbcraft-cli batchjob <id>` (Navicat: `navicat.exe
//! -batchjob <name>`).

use dbcraft_core::{Error, Result};
use dbcraft_model::{BatchJob, JobStep, Schedule, next_id};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::commands::{CommandSpec, always};
use crate::params::{str_opt, str_req};

/// Commands a batch job may run (Navicat job types we support).
pub const STEP_COMMANDS: &[&str] = &["query.run", "backup.create", "backup.restore", "tools.dataTransfer"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "automation.save",
            label: "New Batch Job",
            menu: "Tools/Automation",
            shortcut: None,
            params_doc: r#"{"name":str,"id"?:str,"steps":[{"command":"query.run|backup.create|backup.restore|tools.dataTransfer","params":{..},"label"?:str}],"continue_on_error"?:bool} -> {"id"}"#,
            enabled: always,
            run: save,
        },
        CommandSpec { id: "automation.list", label: "Batch Jobs", menu: "", shortcut: None, params_doc: "{} -> [BatchJob]", enabled: always, run: list },
        CommandSpec { id: "automation.delete", label: "Delete Batch Job", menu: "", shortcut: None, params_doc: r#"{"id":str}"#, enabled: always, run: delete },
        CommandSpec {
            id: "automation.run",
            label: "Start",
            menu: "",
            shortcut: None,
            params_doc: r#"{"id":str} -> {"ok":bool,"steps":[{"label","ok","error"?}],"summary":str}"#,
            enabled: always,
            run: run_job,
        },
        CommandSpec {
            id: "automation.schedule",
            label: "Set Task Schedule",
            menu: "",
            shortcut: None,
            params_doc: r#"{"id":str,"frequency":"once|hourly|daily|weekly","time":"HH:MM","day"?:"MON..SUN","cli_path":str,"dry_run"?:bool} -> {"task":str,"xml":str} (Windows Task Scheduler; runs on battery and catches up missed starts)"#,
            enabled: always,
            run: schedule,
        },
        CommandSpec {
            id: "automation.unschedule",
            label: "Delete Task Schedule",
            menu: "",
            shortcut: None,
            params_doc: r#"{"id":str,"dry_run"?:bool}"#,
            enabled: always,
            run: unschedule,
        },
    ]
}

fn job<'a>(s: &'a Session, id: &str) -> Result<&'a BatchJob> {
    s.workspace.automations.iter().find(|j| j.id == id).ok_or_else(|| Error::NotFound(format!("batch job `{id}`")))
}

fn save(s: &mut Session, p: &Json) -> Result<Json> {
    let name = str_req(p, "name")?.trim().to_string();
    if name.is_empty() {
        return Err(Error::InvalidParam("job name must not be empty".into()));
    }
    let steps: Vec<JobStep> = match p.get("steps") {
        None | Some(Json::Null) => Vec::new(),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| Error::InvalidParam(format!("`steps`: {e}")))?,
    };
    if let Some(bad) = steps.iter().find(|st| !STEP_COMMANDS.contains(&st.command.as_str())) {
        return Err(Error::InvalidParam(format!("`{}` can't be a batch job step (allowed: {})", bad.command, STEP_COMMANDS.join(", "))));
    }
    let id = match str_opt(p, "id")? {
        Some(id) => id.to_string(),
        None => next_id("job", s.workspace.automations.iter().map(|j| j.id.as_str())),
    };
    let continue_on_error = p.get("continue_on_error").and_then(Json::as_bool).unwrap_or(true);
    match s.workspace.automations.iter_mut().find(|j| j.id == id) {
        Some(j) => {
            j.name = name;
            j.steps = steps;
            j.continue_on_error = continue_on_error;
        }
        None => s.workspace.automations.push(BatchJob { id: id.clone(), name, steps, continue_on_error, schedule: None, last_run: None }),
    }
    s.save_workspace()?;
    Ok(json!({ "id": id }))
}

fn list(s: &mut Session, _: &Json) -> Result<Json> {
    serde_json::to_value(&s.workspace.automations).map_err(|e| Error::InvalidParam(e.to_string()))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?.to_string();
    let j = job(s, &id)?.clone();
    if j.schedule.is_some() {
        // Best effort: a task that's already gone shouldn't block deleting the job.
        let _ = run_schtasks(&delete_args(&j));
    }
    s.workspace.automations.retain(|j| j.id != id);
    s.save_workspace()?;
    Ok(json!({}))
}

/// Connection ids a step's params refer to.
fn step_connections(p: &Json) -> Vec<String> {
    [p.get("connection"), p.pointer("/source/connection"), p.pointer("/target/connection")]
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn run_job(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?.to_string();
    let j = job(s, &id)?.clone();
    if j.steps.is_empty() {
        return Err(Error::InvalidParam(format!("batch job `{}` has no steps", j.name)));
    }
    s.report(&format!("Batch job `{}`: {} steps", j.name, j.steps.len()));
    let mut results = Vec::new();
    let mut ok = true;
    for (i, st) in j.steps.iter().enumerate() {
        let label = if st.label.is_empty() { st.command.clone() } else { st.label.clone() };
        s.report(&format!("Step {}/{}: {label}", i + 1, j.steps.len()));
        let r = if STEP_COMMANDS.contains(&st.command.as_str()) {
            step_connections(&st.params).iter().try_for_each(|c| s.conn_auto(c).map(|_| ())).and_then(|()| s.dispatch(&st.command, &st.params))
        } else {
            Err(Error::InvalidParam(format!("`{}` can't be a batch job step", st.command)))
        };
        match r {
            Ok(_) => results.push(json!({ "label": label, "ok": true })),
            Err(e) => {
                ok = false;
                s.report(&format!("  failed: {e}"));
                results.push(json!({ "label": label, "ok": false, "error": e.to_string() }));
                if !j.continue_on_error {
                    break;
                }
            }
        }
    }
    let failed = results.iter().filter(|r| r["ok"] == false).count();
    let summary = format!(
        "{} {}, {} of {} steps ok",
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
        if ok { "ok" } else { "FAILED" },
        results.len() - failed,
        j.steps.len()
    );
    s.report(&format!("Finished: {summary}"));
    if let Some(slot) = s.workspace.automations.iter_mut().find(|x| x.id == id) {
        slot.last_run = Some(summary.clone());
    }
    s.save_workspace()?;
    Ok(json!({ "ok": ok, "steps": results, "summary": summary }))
}

fn task_name(j: &BatchJob) -> String {
    let clean: String = j.name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    format!("DbCraft\\{} ({})", clean.trim(), j.id)
}

fn validate_time(t: &str) -> Result<(u32, u32)> {
    let ok = t.len() == 5 && t.as_bytes().get(2) == Some(&b':');
    let h = t.get(..2).and_then(|h| h.parse::<u32>().ok()).filter(|h| *h < 24);
    let m = t.get(3..).and_then(|m| m.parse::<u32>().ok()).filter(|m| *m < 60);
    match (ok, h, m) {
        (true, Some(h), Some(m)) => Ok((h, m)),
        _ => Err(Error::InvalidParam(format!("time `{t}` must be HH:MM"))),
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Task Scheduler XML for `job`. Pure, so it is tested without touching the system. Unlike plain
/// `schtasks /SC`, the XML also sets: start even on battery, keep running when unplugged, and run
/// as soon as possible after a missed start (the PC was off or asleep).
pub fn task_xml(j: &BatchJob, sch: &Schedule, cli_path: &str, workspace: Option<&str>, today: chrono::NaiveDate) -> Result<String> {
    let (h, m) = validate_time(&sch.time)?;
    if cli_path.trim().is_empty() || cli_path.contains('"') || workspace.is_some_and(|w| w.contains('"')) {
        return Err(Error::InvalidParam("paths must not be empty or contain quotes".into()));
    }
    let start = format!("{}T{h:02}:{m:02}:00", today.format("%Y-%m-%d"));
    let trigger = match sch.frequency.as_str() {
        "once" => format!("<TimeTrigger><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled></TimeTrigger>"),
        "hourly" => format!(
            "<TimeTrigger><Repetition><Interval>PT1H</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled></TimeTrigger>"
        ),
        "daily" => format!(
            "<CalendarTrigger><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>"
        ),
        "weekly" => {
            let day = match sch.day.clone().unwrap_or_else(|| "MON".into()).to_ascii_uppercase().as_str() {
                "MON" => "Monday",
                "TUE" => "Tuesday",
                "WED" => "Wednesday",
                "THU" => "Thursday",
                "FRI" => "Friday",
                "SAT" => "Saturday",
                "SUN" => "Sunday",
                other => return Err(Error::InvalidParam(format!("day `{other}` must be MON..SUN"))),
            };
            format!(
                "<CalendarTrigger><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled><ScheduleByWeek><DaysOfWeek><{day} /></DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek></CalendarTrigger>"
            )
        }
        other => return Err(Error::InvalidParam(format!("frequency `{other}` must be once, hourly, daily or weekly"))),
    };
    let mut args = String::new();
    if let Some(ws) = workspace {
        args.push_str(&format!("--workspace \"{ws}\" "));
    }
    args.push_str(&format!("batchjob {}", j.id));
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>{desc}</Description></RegistrationInfo>
  <Triggers>{trigger}</Triggers>
  <Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT72H</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author"><Exec><Command>{cmd}</Command><Arguments>{args}</Arguments></Exec></Actions>
</Task>
"#,
        desc = xml_escape(&format!("DbCraft batch job: {}", j.name)),
        cmd = xml_escape(cli_path),
        args = xml_escape(&args),
    ))
}

/// Register the task from XML (written as UTF-16, as Task Scheduler expects).
fn create_task(name: &str, xml: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("dbcraft-task-{}.xml", std::process::id()));
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&path, bytes)?;
    let r = run_schtasks(&["/Create".into(), "/F".into(), "/TN".into(), name.to_string(), "/XML".into(), path.to_string_lossy().into_owned()]);
    let _ = std::fs::remove_file(&path);
    r
}

fn delete_args(j: &BatchJob) -> Vec<String> {
    vec!["/Delete".into(), "/F".into(), "/TN".into(), task_name(j)]
}

fn run_schtasks(args: &[String]) -> Result<String> {
    if !cfg!(windows) {
        return Err(Error::Unsupported("scheduling uses the Windows Task Scheduler; on other systems add `dbcraft-cli batchjob <id>` to cron".into()));
    }
    let out = std::process::Command::new("schtasks").args(args).output()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string() + String::from_utf8_lossy(&out.stderr).trim();
    if out.status.success() { Ok(text) } else { Err(Error::Database(format!("schtasks: {text}"))) }
}

fn schedule(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?.to_string();
    let sch = Schedule { frequency: str_req(p, "frequency")?.to_string(), time: str_req(p, "time")?.to_string(), day: str_opt(p, "day")?.map(str::to_string) };
    let cli = str_req(p, "cli_path")?;
    let ws = s.workspace_path.as_ref().map(|w| w.to_string_lossy().into_owned());
    let j = job(s, &id)?.clone();
    let xml = task_xml(&j, &sch, cli, ws.as_deref(), chrono::Local::now().date_naive())?;
    let dry = p.get("dry_run").and_then(Json::as_bool).unwrap_or(false);
    if !dry {
        create_task(&task_name(&j), &xml)?;
        if let Some(slot) = s.workspace.automations.iter_mut().find(|x| x.id == id) {
            slot.schedule = Some(sch);
        }
        s.save_workspace()?;
    }
    Ok(json!({ "task": task_name(&j), "xml": xml }))
}

fn unschedule(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_req(p, "id")?.to_string();
    let j = job(s, &id)?.clone();
    let args = delete_args(&j);
    if !p.get("dry_run").and_then(Json::as_bool).unwrap_or(false) {
        run_schtasks(&args)?;
        if let Some(slot) = s.workspace.automations.iter_mut().find(|x| x.id == id) {
            slot.schedule = None;
        }
        s.save_workspace()?;
    }
    Ok(json!({ "task": task_name(&j), "args": args }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> BatchJob {
        BatchJob { id: "job-1".into(), name: "Nightly \\ backup".into(), steps: vec![], continue_on_error: true, schedule: None, last_run: None }
    }

    #[test]
    fn builds_task_xml() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        let s = Schedule { frequency: "weekly".into(), time: "02:30".into(), day: Some("sun".into()) };
        let xml = task_xml(&job(), &s, "C:\\Program Files\\DbCraft\\dbcraft-cli.exe", Some("C:\\ws & co\\workspace.json"), today).unwrap();
        assert!(xml.contains("<StartBoundary>2026-10-14T02:30:00</StartBoundary>"));
        assert!(xml.contains("<DaysOfWeek><Sunday /></DaysOfWeek>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<Command>C:\\Program Files\\DbCraft\\dbcraft-cli.exe</Command>"));
        assert!(xml.contains("<Arguments>--workspace &quot;C:\\ws &amp; co\\workspace.json&quot; batchjob job-1</Arguments>"), "{xml}");
        assert_eq!(task_name(&job()), "DbCraft\\Nightly _ backup (job-1)");
        let hourly = task_xml(&job(), &Schedule { frequency: "hourly".into(), time: "00:05".into(), day: None }, "x.exe", None, today).unwrap();
        assert!(hourly.contains("<Interval>PT1H</Interval>"));
        for bad in [("daily", "25:00"), ("daily", "2:30"), ("monthly", "02:00"), ("daily", "ab:cd")] {
            let s = Schedule { frequency: bad.0.into(), time: bad.1.into(), day: None };
            assert!(task_xml(&job(), &s, "x.exe", None, today).is_err(), "{bad:?}");
        }
        let s = Schedule { frequency: "daily".into(), time: "02:00".into(), day: None };
        assert!(task_xml(&job(), &s, "x\" & calc.exe", None, today).is_err());
        assert!(task_xml(&job(), &s, "  ", None, today).is_err());
    }
}
