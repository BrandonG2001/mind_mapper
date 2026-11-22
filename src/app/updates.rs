//! The background check for a newer release, and its setting.

use super::MindMapApp;
use crate::background::{Job, JobState};
use crate::update::{self, GitHub, Release, UpdateError, Version};

/// The update check's setting, progress and result.
pub(super) struct Updates {
    /// Whether to ask GitHub for newer releases (a setting).
    pub(super) enabled: bool,
    /// The background check, while it runs.
    pub(super) job: Option<Job<Result<Option<Release>, UpdateError>>>,
    /// A newer release it found, offered in the toolbar and on the start screen.
    pub(super) available: Option<Release>,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            enabled: true,
            job: None,
            available: None,
        }
    }
}

impl MindMapApp {
    /// Start a background check for a newer release, if checks are on, none is running or
    /// already found, and the last check that found the app up to date was `last_check`
    /// at least a day ago.
    pub(super) fn check_for_update_if_due(&mut self, last_check: Option<u64>) {
        if !self.updates.enabled
            || self.updates.job.is_some()
            || self.updates.available.is_some()
            || !update::is_due(last_check, update::now_seconds())
        {
            return;
        }
        self.updates.job = Some(Job::spawn(&self.ctx, "update check", || {
            update::newer_release(&GitHub, Version::current())
        }));
    }

    /// Collect the result of the update check once it is done.
    pub(super) fn poll_update_check(&mut self) {
        let Some(job) = &self.updates.job else {
            return;
        };
        match job.poll() {
            JobState::Running => {}
            JobState::Finished(result) => {
                self.updates.job = None;
                self.finish_update_check(result);
            }
            JobState::Failed => self.updates.job = None,
        }
    }

    /// Act on what the update check found.
    pub(super) fn finish_update_check(&mut self, result: Result<Option<Release>, UpdateError>) {
        match result {
            Ok(Some(release)) => {
                self.status = format!("Mind Mapper {} is available", release.version);
                self.updates.available = Some(release);
            }
            // Up to date: wait a day before asking again. (A newer release found is not
            // recorded, so the next launch offers it again.)
            Ok(None) => {
                let now = update::now_seconds();
                self.update_settings(|settings| settings.last_update_check = Some(now));
            }
            // Offline, or GitHub is unavailable: try again next launch, without bothering
            // the user.
            Err(_) => {}
        }
    }

    /// Turn update checks on or off (remembered between runs). Turning them on checks
    /// straight away if a check is due.
    pub(super) fn set_check_updates(&mut self, on: bool) {
        self.updates.enabled = on;
        self.update_settings(|settings| settings.check_updates = on);
        if on {
            let last_check = self
                .settings_file
                .as_ref()
                .and_then(|file| file.load().last_update_check);
            self.check_for_update_if_due(last_check);
        } else {
            self.updates.available = None;
        }
    }
}
