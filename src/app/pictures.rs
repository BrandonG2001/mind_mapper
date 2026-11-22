//! Adding and removing pictures on nodes: from a file (the right-click menu), by pasting
//! an image (Ctrl+V), or by dropping an image file onto a node.

use std::path::Path;

use eframe::egui::{ColorImage, Pos2};
use rfd::FileDialog;

use super::{MindMapApp, node_label};
use crate::model::NodeId;
use crate::picture::{Picture, PictureError};

/// File extensions of the picture formats that can be added.
const PICTURE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

impl MindMapApp {
    /// Show `picture` on node `id` (one undo step), or report why it could not be read.
    pub(super) fn attach_picture(&mut self, id: NodeId, picture: Result<Picture, PictureError>) {
        match picture {
            Ok(picture) => {
                if self
                    .edit_map(|map| map.set_picture(id, Some(picture)))
                    .is_some()
                {
                    self.status = format!("Added a picture to \"{}\"", node_label(&self.map, id));
                }
            }
            Err(err) => self.status = format!("Could not add the picture: {err}"),
        }
    }

    /// Ask for a PNG or JPEG file and show it on node `id`.
    pub(super) fn choose_picture(&mut self, id: NodeId) {
        if let Some(path) = FileDialog::new()
            .add_filter("Picture (PNG or JPEG)", &PICTURE_EXTENSIONS)
            .pick_file()
        {
            self.attach_picture(id, read_picture(&path));
        }
    }

    /// Remove node `id`'s picture (one undo step).
    pub(super) fn remove_picture(&mut self, id: NodeId) {
        if self.edit_map(|map| map.set_picture(id, None)).is_some() {
            self.status = format!("Removed the picture from \"{}\"", node_label(&self.map, id));
        }
    }

    /// Put a pasted image on the selected node.
    pub(super) fn paste_picture(&mut self, image: &ColorImage) {
        match self.selected {
            Some(id) => self.attach_picture(id, Picture::from_pixels(image)),
            None => "Select a node to paste the picture onto".clone_into(&mut self.status),
        }
    }

    /// Put an image file dropped on the canvas onto the node under `pointer` (or the
    /// selected node if it was dropped on empty canvas). Other files are ignored.
    pub(super) fn drop_picture(
        &mut self,
        path: &Path,
        bytes: Result<Vec<u8>, String>,
        pointer: Option<Pos2>,
    ) {
        let is_picture = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                PICTURE_EXTENSIONS
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            });
        if !is_picture {
            return;
        }
        let target = pointer
            .and_then(|pos| self.view.hit_test(pos))
            .or(self.selected);
        let Some(target) = target else {
            "Drop the picture onto a node".clone_into(&mut self.status);
            return;
        };
        match bytes {
            Ok(bytes) => self.attach_picture(target, Picture::from_bytes(bytes)),
            Err(err) => self.status = format!("Could not read the dropped file: {err}"),
        }
    }
}

/// Read the picture file at `path`.
fn read_picture(path: &Path) -> Result<Picture, PictureError> {
    let bytes = std::fs::read(path).map_err(|err| PictureError::Image(err.into()))?;
    Picture::from_bytes(bytes)
}
