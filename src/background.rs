//! Running slow work (exports, recovery copies) on a background thread, so the window
//! stays responsive.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;

/// Where a background job is up to.
#[derive(Debug, PartialEq, Eq)]
pub enum JobState<T> {
    /// Still working.
    Running,
    /// Done, with its result.
    Finished(T),
    /// The thread could not be started or panicked, so there is no result.
    Failed,
}

/// A piece of work running on its own thread.
#[derive(Debug)]
pub struct Job<T> {
    receiver: Receiver<T>,
}

impl<T: Send + 'static> Job<T> {
    /// Run `work` on a new thread. When it finishes, `ctx` is asked to repaint so the
    /// window notices the result promptly even if the user is idle.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The egui context to wake when done
    /// * `name` - Thread name, shown in debuggers and crash reports
    /// * `work` - The work; it must own everything it uses (`move` closure)
    pub fn spawn(
        ctx: &egui::Context,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Self {
        // A channel carries the result back. If the thread cannot be started, the
        // closure (and with it the sender) is dropped, which `poll` reports as `Failed`.
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                // The receiver may be gone if the app is closing; nothing to do then.
                sender.send(work()).ok();
                ctx.request_repaint();
            })
            .ok();
        Self { receiver }
    }

    /// Check on the job without waiting.
    #[must_use]
    pub fn poll(&self) -> JobState<T> {
        match self.receiver.try_recv() {
            Ok(result) => JobState::Finished(result),
            Err(TryRecvError::Empty) => JobState::Running,
            Err(TryRecvError::Disconnected) => JobState::Failed,
        }
    }

    /// Wait for the job to finish.
    ///
    /// # Returns
    ///
    /// Its result, or `None` if it failed.
    #[must_use]
    pub fn wait(self) -> Option<T> {
        self.receiver.recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_arrives_and_wait_returns_it() {
        let ctx = egui::Context::default();
        let job = Job::spawn(&ctx, "test-job", || 6 * 7);

        assert_eq!(job.wait(), Some(42));
    }

    #[test]
    fn poll_reports_running_then_finished() {
        let ctx = egui::Context::default();
        let (go, wait_for_go) = mpsc::channel::<()>();
        let job = Job::spawn(&ctx, "test-job", move || {
            wait_for_go.recv().ok();
            "done"
        });

        assert_eq!(job.poll(), JobState::Running);
        go.send(()).expect("job is waiting");
        let state = loop {
            match job.poll() {
                JobState::Running => std::thread::yield_now(),
                other => break other,
            }
        };

        assert_eq!(state, JobState::Finished("done"));
    }

    #[test]
    fn panicking_job_is_reported_as_failed() {
        let ctx = egui::Context::default();
        let job: Job<u8> = Job::spawn(&ctx, "test-job", || panic!("deliberate test panic"));

        assert_eq!(job.wait(), None);
    }
}
