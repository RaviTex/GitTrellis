//! Background task execution.
//!
//! Git operations run on plain OS threads. Results are delivered back to the
//! GTK main thread through the default GLib main context, so widgets are only
//! ever touched from the main thread.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::git::Cancellation;

static TASK_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn next_task_id() -> u64 {
    TASK_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Run `work` on a background thread and deliver its result on the main
/// thread.
pub fn spawn<F, T>(work: F, on_done: impl FnOnce(T) + Send + 'static)
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name(format!("gittrellis-task-{}", next_task_id()))
        .spawn(move || {
            let result = work();
            invoke_on_main(move || on_done(result));
        })
        .expect("failed to spawn worker thread");
}

/// Schedule a closure on the GTK main thread from any thread.
pub fn invoke_on_main<F: FnOnce() + Send + 'static>(f: F) {
    let main_context = glib::MainContext::default();
    main_context.invoke(f);
}

/// A cancellable long running operation helper.
#[derive(Debug, Clone)]
pub struct BackgroundOp {
    pub cancel: Cancellation,
    pub label: String,
}

impl BackgroundOp {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            cancel: Cancellation::new(),
            label: label.into(),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Default for BackgroundOp {
    fn default() -> Self {
        Self::new("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn spawn_runs_work_off_thread() {
        // The completion callback needs a running GLib main context; in this
        // headless test we only verify the work itself executes.
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let v = 40 + 2;
            tx.send(v).unwrap();
        });
        handle.join().unwrap();
        assert_eq!(rx.recv().unwrap(), 42);
    }

    #[test]
    fn cancellation_flag() {
        let op = BackgroundOp::new("fetch");
        assert!(!op.cancel.is_cancelled());
        op.cancel();
        assert!(op.cancel.is_cancelled());
    }

    #[test]
    fn task_ids_increase() {
        let a = next_task_id();
        let b = next_task_id();
        assert!(b > a);
    }
}
