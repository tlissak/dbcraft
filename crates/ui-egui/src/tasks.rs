//! Interactive background work (ROADMAP 1.1): opening a connection, running a query, loading a
//! table page or a database's objects. A [`Task`] runs engine commands on a worker thread over a
//! session that shares the app's live connections ([`Session::fork_shared`]), so the window never
//! freezes on a slow query or an unreachable host. Stop (Ctrl+T) cancels it; the server is asked
//! to stop the statement too.
//!
//! Unlike [`crate::jobs::Job`] (long tools with a progress log, own connections), a task hands
//! its session back when it finishes so the app can [`Session::absorb`] what it learned.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dbcraft_engine::{Cancel, Session};
use serde_json::Value as Json;

/// What a finished task hands back.
pub struct Done {
    /// The last command's result (the first error stops the task).
    pub result: Result<Json, String>,
    /// The worker session, to [`Session::absorb`] on the UI thread.
    pub session: Session,
    pub cancelled: bool,
}

/// Handle to a background task. Dropping it doesn't stop the worker; call [`Task::cancel`].
#[derive(Clone)]
pub struct Task {
    done: Arc<Mutex<Option<Done>>>,
    cancel: Cancel,
    started: Instant,
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task").field("elapsed", &self.started.elapsed()).finish()
    }
}

impl Task {
    /// Run `steps` in order on a worker; a step whose connection isn't open yet can be preceded
    /// by `connection.open`. Repaints `ctx` when done.
    pub fn spawn(ctx: &egui::Context, session: &Session, steps: Vec<(&'static str, Json)>) -> Task {
        let done = Arc::new(Mutex::new(None));
        let cancel = Cancel::new();
        let mut worker = session.fork_shared();
        let (out, c, ctx) = (done.clone(), cancel.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("dbcraft-task".into()).spawn(move || {
            let mut result = Ok(Json::Null);
            dbcraft_engine::with_cancel(&c, || {
                for (id, params) in &steps {
                    result = worker.dispatch(id, params).map_err(|e| e.to_string());
                    if result.is_err() {
                        break;
                    }
                }
            });
            let cancelled = c.is_cancelled();
            if cancelled {
                result = Err("Stopped".into());
            }
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(Done { result, session: worker, cancelled });
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            // No thread: report it as a failed task rather than hanging.
            *done.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(Done { result: Err(format!("cannot start a worker thread: {e}")), session: Session::new(), cancelled: false });
        }
        Task { done, cancel, started: Instant::now() }
    }

    /// Stop: the worker returns at once; PostgreSQL/MySQL also cancel the statement server-side.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn finished(&self) -> bool {
        self.done.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    /// The outcome, once (later calls return `None`).
    pub fn take(&self) -> Option<Done> {
        self.done.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// Block until the task finishes (tests and the CLI; never from the UI thread).
pub fn wait(task: &Task, timeout: Duration) -> bool {
    let start = Instant::now();
    while !task.finished() {
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// "Running… 3.2 s" with a spinner and a Stop button; returns true when Stop was clicked.
pub fn running_bar(ui: &mut egui::Ui, task: &Task, what: &str) -> bool {
    let mut stop = false;
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(format!("{what} {:.1} s", task.elapsed().as_secs_f32()));
        if ui.button(format!("{} Stop", crate::icons::STOP)).on_hover_text("Stop (Ctrl+T)").clicked() {
            stop = true;
        }
    });
    // Keep the timer ticking while the worker runs.
    ui.ctx().request_repaint_after(Duration::from_millis(100));
    stop
}
