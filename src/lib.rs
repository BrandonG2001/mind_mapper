//! Mind Mapper: a desktop mind-mapping application built with egui.
//!
//! * [`model`] - the tree of ideas and its editing operations
//! * [`history`] - undo/redo
//! * [`camera`] - pan and zoom
//! * [`storage`] - JSON load/save
//! * [`templates`] - built-in starting maps
//! * [`map_view`] - drawing a map (editor canvas and thumbnails)
//! * [`start_screen`] - the template gallery shown at launch
//! * [`theme`] - selectable colour themes
//! * [`app`] - the graphical user interface

pub mod app;
pub mod camera;
pub mod history;
pub mod map_view;
pub mod model;
pub mod start_screen;
pub mod storage;
pub mod templates;
pub mod theme;
