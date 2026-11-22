//! Saving automatically: the map file itself shortly after each change (if switched on),
//! crash recovery copies, and the background jobs that write those and exports.

use std::time::Duration;

use eframe::egui;

use super::{AUTO_SAVE_DELAY, AUTOSAVE_SECONDS, MindMapApp, file_label};
use crate::background::{Job, JobState};
use crate::storage::StorageError;

/// Saving the map file itself shortly after each change.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct FileAutoSave {
    /// Whether it is switched on (a setting, off by default).
    pub(super) enabled: bool,
    /// When the map last changed, in egui's clock (seconds).
    pub(super) changed_at: f64,
    /// The map revision auto-save stopped at because the file changed elsewhere; it
    /// tries again after the next edit (and asks nothing: Save lets the user choose).
    blocked_at: Option<u64>,
}

impl MindMapApp {
    /// Save the map file if auto-save is on, the map has one and unsaved changes, and
    /// nothing has changed for [`AUTO_SAVE_DELAY`] seconds up to `now`.
    ///
    /// # Returns
    ///
    /// How long to wait before it is due, if it will be.
    pub(super) fn auto_save_file(&mut self, now: f64) -> Option<f64> {
        let busy = self.question.is_some() || self.editing.is_some() || self.label_edit.is_some();
        let state = &self.file_autosave;
        if !state.enabled || !self.dirty || busy || state.blocked_at == Some(self.revision) {
            return None;
        }
        // New maps and imports have no file yet: the first Save picks it.
        let path = self.file_path.clone()?;
        let due = state.changed_at + AUTO_SAVE_DELAY;
        if now < due {
            return Some(due - now);
        }
        if self.would_overwrite_changes(&path) {
            self.file_autosave.blocked_at = Some(self.revision);
            self.status = format!(
                "Auto-save paused: {} was changed elsewhere; Save to choose what to keep",
                file_label(&path)
            );
            return None;
        }
        self.write_to(path);
        None
    }

    /// Turn auto-save on or off (remembered between runs).
    pub(super) fn set_auto_save(&mut self, on: bool) {
        self.file_autosave.enabled = on;
        self.update_settings(|settings| settings.auto_save = on);
    }

    /// Write a recovery copy if there are new changes and enough time has passed, and
    /// save the map file if auto-save is due.
    pub(super) fn autosave(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        if let Some(wait) = self.auto_save_file(now) {
            // egui only redraws on input, so ask for a wake-up when the save is due.
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
        if !self.autosave_pending {
            return;
        }
        if self.recovery.is_none() {
            return;
        }
        let due = self.last_autosave + AUTOSAVE_SECONDS;
        if now < due {
            // egui only redraws on input, so ask for a wake-up when the save is due.
            ctx.request_repaint_after(Duration::from_secs_f64(due - now));
            return;
        }
        // One copy at a time: if the previous one is still being written, try again on
        // a later frame (changes stay pending).
        if self.recovery_job.is_some() {
            return;
        }
        let Some(store) = &self.recovery else {
            return;
        };
        // Copying the map is quick; turning it into JSON and writing it is the slow part
        // (about 40 ms at 100,000 nodes), so that happens on a background thread.
        let writer = store.writer();
        let map = self.map.clone();
        let source = self.file_path.clone();
        self.recovery_job = Some(Job::spawn(&self.ctx, "recovery-copy", move || {
            writer.save(&map, source.as_deref())
        }));
        self.autosave_pending = false;
        self.last_autosave = now;
    }

    /// Write this window's recovery copy now, on this thread.
    pub(super) fn save_recovery_copy(&mut self) {
        // A background write of the same file must not land after this one.
        self.finish_recovery_job();
        if let Some(store) = &self.recovery
            && let Err(err) = store.save(&self.map, self.file_path.as_deref())
        {
            self.status = format!("Auto-save failed: {err}");
        }
        self.autosave_pending = false;
    }

    /// Wait for a background recovery copy still being written, and report any error.
    pub(super) fn finish_recovery_job(&mut self) {
        if let Some(job) = self.recovery_job.take() {
            self.report_recovery_result(job.wait());
        }
    }

    pub(super) fn report_recovery_result(&mut self, result: Option<Result<(), StorageError>>) {
        match result {
            Some(Ok(())) => {}
            Some(Err(err)) => self.status = format!("Auto-save failed: {err}"),
            None => "Auto-save failed unexpectedly".clone_into(&mut self.status),
        }
    }

    /// Collect the results of background jobs that have finished.
    pub(super) fn poll_jobs(&mut self) {
        if let Some(job) = &self.recovery_job {
            match job.poll() {
                JobState::Running => {}
                JobState::Finished(result) => {
                    self.recovery_job = None;
                    self.report_recovery_result(Some(result));
                }
                JobState::Failed => {
                    self.recovery_job = None;
                    self.report_recovery_result(None);
                }
            }
        }
        let mut messages = Vec::new();
        self.exports.retain(|(name, job)| {
            let message = match job.poll() {
                JobState::Running => return true,
                JobState::Finished(Ok(())) => format!("Exported {name}"),
                JobState::Finished(Err(err)) => format!("Could not export {name}: {err}"),
                JobState::Failed => {
                    format!("Could not export {name}: the export stopped unexpectedly")
                }
            };
            messages.push(message);
            false
        });
        if let Some(last) = messages.pop() {
            self.status = last;
        }
        self.poll_update_check();
    }

    /// Look for unsaved work left by a window that is no longer running.
    pub(super) fn find_orphan(&mut self) {
        if let Some(store) = &self.recovery {
            match store.find_orphan() {
                Ok(found) => self.recovered = found,
                Err(err) => self.status = format!("Could not read recovered work: {err}"),
            }
        }
    }

    /// Delete the files of the recovered work on offer (it was restored or discarded).
    pub(super) fn discard_orphan(&mut self) {
        if let Some(recovered) = self.recovered.take()
            && let Err(err) = recovered.discard()
        {
            self.status = format!("Could not remove recovered work: {err}");
        }
    }

    /// Delete the recovery copy (after a save, or once the user discards their work).
    pub(super) fn clear_recovery(&mut self) {
        // A background write finishing after the delete would bring the file back.
        self.finish_recovery_job();
        if let Some(store) = &self.recovery
            && let Err(err) = store.clear()
        {
            self.status = format!("Could not remove the recovery file: {err}");
        }
    }
}
