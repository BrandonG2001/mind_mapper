//! Mind Mapper: a desktop mind-mapping application built with egui.
//!
//! * [`model`] - the tree of ideas and its editing operations
//! * [`history`] - undo/redo
//! * [`camera`] - pan and zoom
//! * [`storage`] - JSON load/save
//! * [`export`] - PNG, SVG and Markdown export
//! * [`recovery`] - crash recovery of unsaved work
//! * [`recent`] - recently used files
//! * [`templates`] - built-in starting maps
//! * [`layout`] - automatic, overlap-free layout ("Tidy up")
//! * [`navigation`] - moving the selection with the arrow keys
//! * [`map_view`] - drawing a map (editor canvas and thumbnails)
//! * [`start_screen`] - the template gallery shown at launch
//! * [`theme`] - selectable colour themes
//! * [`app`] - the graphical user interface

pub mod app;
pub mod camera;
pub mod export;
pub mod history;
pub mod layout;
pub mod map_view;
pub mod model;
pub mod navigation;
pub mod recent;
pub mod recovery;
pub mod start_screen;
pub mod storage;
pub mod templates;
pub mod theme;

#[cfg(test)]
mod test_support;
