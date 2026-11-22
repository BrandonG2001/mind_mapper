//! Yes/no questions drawn inside the window (unsaved changes, a file changed on disk).
//!
//! They are egui pop-ups rather than system message boxes because those need a helper
//! program on Linux (`zenity`). Without it, a message box silently answers "no", which
//! once made a window with unsaved work impossible to close.
//!
//! A question cannot block and return the answer the way a system message box does,
//! because egui draws the window one frame at a time. Instead, the question stores what
//! to do next, and that runs when the user answers.

use std::borrow::Cow;
use std::path::PathBuf;

use eframe::egui::{self, ViewportCommand};

use super::{MindMapApp, file_label};

/// Widest a question's text gets before it wraps, in points.
const MAX_WIDTH: f32 = 380.0;

/// A question waiting for the user's answer, and what to do if they agree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Question {
    /// Unsaved work would be lost by `then`.
    Discard(AfterDiscard),
    /// Saving to this path would replace changes made there by another window or program.
    Overwrite(PathBuf),
}

/// What to do once the user agrees to throw away unsaved work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AfterDiscard {
    /// Close the window.
    Close,
    /// Ask which map file to open, then open it.
    PickFile,
    /// Open this map file.
    OpenPath(PathBuf),
    /// Start from the start screen's template at this index.
    Template(usize),
}

impl Question {
    /// The question's title, text and the label of the button that agrees.
    fn wording(&self) -> (&'static str, Cow<'static, str>, &'static str) {
        match self {
            Self::Discard(_) => (
                "Unsaved changes",
                Cow::Borrowed("Discard the unsaved changes to this mind map?"),
                "Discard changes",
            ),
            Self::Overwrite(path) => (
                "File changed on disk",
                Cow::Owned(format!(
                    "{} was changed by another window or program after you opened it.\n\n\
                     Replace those changes with this version? (Choose Cancel, then \
                     Save As, to keep both.)",
                    file_label(path)
                )),
                "Replace",
            ),
        }
    }
}

impl MindMapApp {
    /// Whether some work exists only in this window: unsaved edits, or work recovered
    /// after a crash that has not been dealt with yet.
    pub(super) fn has_unsaved_work(&self) -> bool {
        self.dirty || self.recovered.is_some()
    }

    /// Do `then`, first asking (in a pop-up) whether to throw away unsaved work, if any.
    pub(super) fn discard_then(&mut self, then: AfterDiscard) {
        if self.has_unsaved_work() {
            self.question = Some(Question::Discard(then));
        } else {
            self.continue_after(then);
        }
    }

    /// Act on the user's answer to the open question (if any): `agreed` is true for the
    /// button that goes ahead, false for Cancel.
    pub(super) fn answer(&mut self, agreed: bool) {
        // `take` moves the question out of `self` (leaving `None`), like `dict.pop`.
        let Some(question) = self.question.take() else {
            return;
        };
        match (question, agreed) {
            (Question::Discard(then), true) => {
                self.discard_orphan();
                self.clear_recovery();
                // If the current map stays open (e.g. the file dialog is cancelled), keep
                // protecting it.
                self.autosave_pending = self.dirty;
                self.continue_after(then);
            }
            (Question::Overwrite(path), true) => self.write_to(path),
            (Question::Overwrite(path), false) => {
                self.status = format!(
                    "Not saved: {} was changed elsewhere; use Save As to keep both versions",
                    file_label(&path)
                );
            }
            (Question::Discard(_), false) => {}
        }
    }

    /// Carry out an action that no longer needs to protect unsaved work.
    fn continue_after(&mut self, then: AfterDiscard) {
        match then {
            AfterDiscard::Close => {
                self.close_confirmed = true;
                self.ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            AfterDiscard::PickFile => {
                if let Some(path) = super::files::file_dialog().pick_file() {
                    self.open_path(&path);
                }
            }
            AfterDiscard::OpenPath(path) => self.open_path(&path),
            AfterDiscard::Template(index) => {
                if let Some(map) = self.start.template_map(index) {
                    self.replace_map(map, None);
                }
            }
        }
    }

    /// Draw the open question, if any, over the whole window, and act on the answer.
    ///
    /// Escape or a click outside the pop-up counts as Cancel.
    pub(super) fn show_question(&mut self, ctx: &egui::Context) {
        let Some(question) = &self.question else {
            return;
        };
        let (title, text, agree) = question.wording();
        let response = egui::Modal::new(egui::Id::unique("question")).show(ctx, |ui| {
            ui.set_max_width(MAX_WIDTH);
            ui.heading(title);
            ui.label(text);
            ui.add_space(ui.spacing().item_spacing.y * 2.0);
            ui.horizontal(|ui| {
                if ui.button(agree).clicked() {
                    Some(true)
                } else if ui.button("Cancel").clicked() {
                    Some(false)
                } else {
                    None
                }
            })
            .inner
        });
        let answer = response
            .inner
            .or_else(|| response.should_close().then_some(false));
        if let Some(agreed) = answer {
            self.answer(agreed);
        }
    }
}
