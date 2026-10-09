//! GTK integration for background work.
//!
//! Git operations run on worker threads; results and progress are delivered
//! on the GTK main thread so that widgets are only ever touched locally.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::git::{Cancellation, GitError, GitResult, StreamKind};

/// Run a blocking operation on a worker thread, delivering the result on the
/// main thread. The completion callback does not need to be `Send`.
pub fn run_blocking<F, T, D>(work: F, on_done: D)
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (tx, rx) = async_channel::bounded::<T>(1);
    std::thread::Builder::new()
        .name("gittrellis-task".into())
        .spawn(move || {
            let result = work();
            let _ = tx.send_blocking(result);
        })
        .expect("spawn worker");
    glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            on_done(value);
        }
    });
}

/// A long running, streaming, cancellable operation.
pub struct StreamingOp {
    cancel: Cancellation,
    finished: Arc<AtomicBool>,
}

impl StreamingOp {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

/// Run a git operation that reports progress and can be cancelled.
///
/// `work` receives the cancellation token and a progress callback. `on_line`
/// receives progress lines on the main thread while the operation runs and
/// `on_done` receives the final result.
pub fn run_streaming<T, F, P, D>(work: F, on_line: P, on_done: D) -> StreamingOp
where
    T: Send + 'static,
    F: FnOnce(
            Cancellation,
            Box<dyn FnMut(StreamKind, &str) + Send>,
        ) -> GitResult<T>
        + Send
        + 'static,
    P: Fn(String) + 'static,
    D: FnOnce(GitResult<T>) + 'static,
{
    let cancel = Cancellation::new();
    let finished = Arc::new(AtomicBool::new(false));
    let result_slot: Arc<Mutex<Option<GitResult<T>>>> = Arc::new(Mutex::new(None));
    let (line_tx, line_rx) = async_channel::unbounded::<String>();

    let cancel_worker = cancel.clone();
    let result_slot_worker = Arc::clone(&result_slot);
    std::thread::Builder::new()
        .name("gittrellis-stream".into())
        .spawn(move || {
            let progress = move |_: StreamKind, line: &str| {
                let _ = line_tx.send_blocking(line.to_string());
            };
            let result = work(cancel_worker, Box::new(move |kind, line| {
                let _ = kind;
                progress(StreamKind::Stderr, line);
            }));
            *result_slot_worker.lock().unwrap() = Some(result);
        })
        .expect("spawn worker");

    let mut on_line = Some(on_line);
    let mut on_done = Some(on_done);
    let finished_ui = Arc::clone(&finished);
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let mut had_lines = false;
        while let Ok(line) = line_rx.try_recv() {
            if let Some(cb) = on_line.as_mut() {
                cb(line);
            }
            had_lines = true;
        }
        let _ = had_lines;
        if let Some(result) = result_slot.lock().unwrap().take() {
            // Drain remaining progress before delivering the result.
            while let Ok(line) = line_rx.try_recv() {
                if let Some(cb) = on_line.as_mut() {
                    cb(line);
                }
            }
            finished_ui.store(true, Ordering::SeqCst);
            if let Some(cb) = on_done.take() {
                cb(result);
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });

    StreamingOp { cancel, finished }
}

/// Convenience: turn a git error into a logged, user visible report.
pub fn report_error(context: &str, error: &GitError) {
    crate::log_error!("{context}: {error}\n{}", error.technical_details());
}
