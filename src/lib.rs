//! Mind Mapper: a desktop mind-mapping application built with egui.
//!
//! * [`model`] - the tree of ideas and its editing operations
//! * [`history`] - undo/redo
//! * [`camera`] - pan and zoom
//! * [`storage`] - JSON load/save
//! * [`app`] - the graphical user interface

pub mod app;
pub mod camera;
pub mod history;
pub mod model;
pub mod storage;
