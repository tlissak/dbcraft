//! Background jobs: run one engine command in a forked session on a worker thread, streaming
//! progress lines to the UI (Data Transfer, Backup, Restore, Automation). The UI stays responsive.

use std::sync::{Arc, Mutex, PoisonError};

use dbcraft_engine::Session;
use serde_json::Value as Json;

#[derive(Default)]
struct Inner {
    log: Mutex<Vec<String>>,
    result: Mutex<Option<Result<Json, String>>>,
}

/// Handle to a running or finished job. Cheap to clone; not serialized.
#[derive(Clone, Default)]
pub struct Job {
    inner: Arc<Inner>,
    started: bool,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job").field("running", &self.running()).finish()
    }
}

impl Job {
    pub fn spawn(ctx: &egui::Context, session: &Session, command: &str, params: Json) -> Job {
        let inner = Arc::new(Inner::default());
        let (sink_inner, sink_ctx) = (inner.clone(), ctx.clone());
        let sink: dbcraft_engine::ProgressSink = Arc::new(move |line: &str| {
            sink_inner.log.lock().unwrap_or_else(PoisonError::into_inner).push(line.to_string());
            sink_ctx.request_repaint();
        });
        let mut worker = session.fork(Some(sink));
        let (out, ctx, command) = (inner.clone(), ctx.clone(), command.to_string());
        std::thread::spawn(move || {
            let r = worker.dispatch(&command, &params).map_err(|e| e.to_string());
            *out.result.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
            ctx.request_repaint();
        });
        Job { inner, started: true }
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn running(&self) -> bool {
        self.started && self.inner.result.lock().unwrap_or_else(PoisonError::into_inner).is_none()
    }

    pub fn log(&self) -> Vec<String> {
        self.inner.log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn result(&self) -> Option<Result<Json, String>> {
        self.inner.result.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// A log panel for a job: spinner while running, then the outcome.
pub fn show_log(ui: &mut egui::Ui, job: &Job, height: f32) {
    if !job.started() {
        return;
    }
    ui.horizontal(|ui| match job.result() {
        None => {
            ui.spinner();
            ui.label("Running…");
        }
        Some(Ok(_)) => {
            ui.colored_label(crate::theme::Tokens::success(), format!("{} Finished", crate::icons::CHECK));
        }
        Some(Err(e)) => {
            ui.colored_label(crate::theme::Tokens::error(), format!("{} {e}", crate::icons::X));
        }
    });
    egui::Frame::new()
        .fill(crate::theme::Tokens::panel_bg())
        .stroke(egui::Stroke::new(1.0, crate::theme::Tokens::border()))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(height).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
                for line in job.log() {
                    ui.monospace(line);
                }
            });
        });
}
