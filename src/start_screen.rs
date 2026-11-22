//! The start screen: a gallery of templates to begin a new mind map from.

use std::path::PathBuf;

use eframe::egui::{
    self, Align, Align2, CursorIcon, FontId, Key, Layout, Modifiers, Rect, RichText, Sense,
    StrokeKind, Ui, Vec2,
};

use crate::map_view::MapView;
use crate::model::MindMap;
use crate::templates::{TEMPLATES, Template};
use crate::theme::{ColorTheme, theme_picker, ui_size_control};

/// Size of one template card.
const CARD_SIZE: Vec2 = Vec2::new(240.0, 210.0);
/// Height of the thumbnail at the top of a card.
const PREVIEW_HEIGHT: f32 = 130.0;
/// Gap between cards and around their contents.
const GAP: f32 = 16.0;
/// Width of the recent-files list.
const RECENT_WIDTH: f32 = 520.0;
/// Rounded-corner radius of cards.
const CARD_RADIUS: f32 = 10.0;

/// What the user picked on the start screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartChoice {
    /// Start a new map from `TEMPLATES[index]`.
    Template(usize),
    /// Open an existing file instead.
    Open,
    /// Return to the map that was open before the start screen was shown.
    Back,
    /// Reopen the work recovered from a crashed session.
    Restore,
    /// Throw the recovered work away.
    DiscardRecovery,
    /// Open the file at this index of [`StartContext::recent`].
    OpenRecent(usize),
}

/// What the start screen shows besides the templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartContext<'a> {
    /// Whether a map is already open, enabling "Back" and Escape.
    pub can_return: bool,
    /// Description of work recovered from a crash, if any; shows a Restore/Discard banner.
    pub recovery: Option<&'a str>,
    /// Recently used files, newest first.
    pub recent: &'a [PathBuf],
}

/// Template gallery with pre-built thumbnail maps.
#[derive(Debug)]
pub struct StartScreen {
    /// `maps[i]` is `TEMPLATES[i]` already built, so thumbnails don't rebuild per frame.
    maps: Vec<MindMap>,
    /// Scratch view reused to draw every thumbnail.
    preview: MapView,
}

impl Default for StartScreen {
    fn default() -> Self {
        Self {
            maps: TEMPLATES.iter().map(Template::build).collect(),
            preview: MapView::default(),
        }
    }
}

impl StartScreen {
    /// A fresh copy of the map for template `index`.
    ///
    /// # Returns
    ///
    /// `None` if `index` is out of range.
    #[must_use]
    pub fn template_map(&self, index: usize) -> Option<MindMap> {
        self.maps.get(index).cloned()
    }

    /// Draw the start screen.
    ///
    /// # Arguments
    ///
    /// * `ui` - Area to fill
    /// * `context` - Back button, recovery banner and recent files to show
    /// * `theme` - Colour theme, changeable from the picker at the top
    ///
    /// # Returns
    ///
    /// The user's choice this frame, if any.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        context: &StartContext<'_>,
        theme: &mut ColorTheme,
    ) -> Option<StartChoice> {
        let can_return = context.can_return;
        let mut choice = None;
        if can_return && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            choice = Some(StartChoice::Back);
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                // Right-to-left layout pins the theme picker to the top-right corner.
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    theme_picker(ui, theme);
                    ui.label("Theme");
                    // Plain spacing: a separator here would stretch to the full height.
                    ui.add_space(GAP);
                    ui_size_control(ui);
                });
                ui.vertical_centered(|ui| {
                    ui.heading(RichText::new("Mind Mapper").size(30.0).strong());
                    ui.label("Choose a template to start a new mind map");
                    ui.add_space(GAP);
                    if ui
                        .button("Open existing map...")
                        .on_hover_text("Ctrl+O")
                        .clicked()
                    {
                        choice = Some(StartChoice::Open);
                    }
                    if can_return
                        && ui
                            .button("Back to current map")
                            .on_hover_text("Esc")
                            .clicked()
                    {
                        choice = Some(StartChoice::Back);
                    }
                    ui.add_space(GAP);
                    if let Some(description) = context.recovery {
                        choice = recovery_banner(ui, description).or(choice);
                        ui.add_space(GAP);
                    }
                    if !context.recent.is_empty() {
                        choice = recent_list(ui, context.recent).or(choice);
                        ui.add_space(GAP);
                    }
                });
                if let Some(index) = self.gallery(ui) {
                    choice = Some(StartChoice::Template(index));
                }
                ui.add_space(GAP);
            });
        choice
    }

    /// Lay the cards out in as many centred columns as fit.
    fn gallery(&mut self, ui: &mut Ui) -> Option<usize> {
        let mut clicked = None;
        let available = ui.available_width();
        // Find the largest column count whose total width fits (at least one).
        let columns = (1..=TEMPLATES.len())
            .take_while(|&count| grid_width(count) <= available)
            .last()
            .unwrap_or(1);
        let indent = ((available - grid_width(columns)) * 0.5).max(0.0);
        ui.spacing_mut().item_spacing = Vec2::splat(GAP);
        for row_start in (0..TEMPLATES.len()).step_by(columns) {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                for index in row_start..(row_start + columns).min(TEMPLATES.len()) {
                    if self.card(ui, index) {
                        clicked = Some(index);
                    }
                }
            });
        }
        clicked
    }

    /// Draw one template card.
    ///
    /// # Returns
    ///
    /// `true` if it was clicked.
    fn card(&mut self, ui: &mut Ui, index: usize) -> bool {
        let (rect, response) = ui.allocate_exact_size(CARD_SIZE, Sense::click());
        // Skip drawing cards scrolled out of view.
        if ui.is_rect_visible(rect)
            && let (Some(template), Some(map)) = (TEMPLATES.get(index), self.maps.get(index))
        {
            let visuals = ui.visuals();
            let painter = ui.painter();
            let border = if response.hovered() {
                visuals.selection.stroke
            } else {
                visuals.widgets.noninteractive.bg_stroke
            };
            painter.rect(
                rect,
                CARD_RADIUS,
                visuals.window_fill(),
                border,
                StrokeKind::Inside,
            );

            let preview = Rect::from_min_size(rect.min, Vec2::new(CARD_SIZE.x, PREVIEW_HEIGHT))
                .shrink(GAP * 0.5);
            painter.rect_filled(preview, CARD_RADIUS * 0.6, visuals.extreme_bg_color);
            self.preview.fit(map, preview.shrink(GAP * 0.5));
            self.preview
                .paint(&painter.with_clip_rect(preview), visuals, map, None);

            let text_left = rect.min.x + GAP;
            let title_top = preview.max.y + GAP * 0.5;
            painter.text(
                egui::pos2(text_left, title_top),
                Align2::LEFT_TOP,
                template.name,
                FontId::proportional(17.0),
                visuals.strong_text_color(),
            );
            let description = painter.layout(
                template.description.to_owned(),
                FontId::proportional(13.0),
                visuals.weak_text_color(),
                CARD_SIZE.x - GAP * 2.0,
            );
            painter.galley(
                egui::pos2(text_left, title_top + 24.0),
                description,
                visuals.weak_text_color(),
            );
        }
        response.on_hover_cursor(CursorIcon::PointingHand).clicked()
    }
}

/// A highlighted box offering to restore or discard work recovered after a crash.
///
/// # Returns
///
/// `Restore` or `DiscardRecovery` if one of its buttons was clicked.
fn recovery_banner(ui: &mut Ui, description: &str) -> Option<StartChoice> {
    let mut choice = None;
    let accent = ui.visuals().selection.stroke.color;
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(2.0, accent))
        .inner_margin(GAP * 0.75)
        .show(ui, |ui| {
            ui.label(RichText::new("Unsaved work was recovered from your last session").strong());
            ui.label(description);
            ui.add_space(GAP * 0.5);
            if ui.button("Restore").clicked() {
                choice = Some(StartChoice::Restore);
            }
            if ui.button("Discard").clicked() {
                choice = Some(StartChoice::DiscardRecovery);
            }
        });
    choice
}

/// The recently used files, each a button showing the file name and its folder.
///
/// # Returns
///
/// `OpenRecent` with the index of the clicked file, if any.
fn recent_list(ui: &mut Ui, recent: &[PathBuf]) -> Option<StartChoice> {
    let mut choice = None;
    ui.label(RichText::new("Recent").strong());
    for (index, path) in recent.iter().enumerate() {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let folder = path
            .parent()
            .map_or_else(String::new, |folder| folder.display().to_string());
        let button = egui::Button::new(RichText::new(name).strong())
            .shortcut_text(folder)
            .min_size(Vec2::new(RECENT_WIDTH, 0.0));
        if ui
            .add(button)
            .on_hover_text(path.display().to_string())
            .clicked()
        {
            choice = Some(StartChoice::OpenRecent(index));
        }
    }
    choice
}

/// Total width of `columns` cards with gaps between them.
fn grid_width(columns: usize) -> f32 {
    let columns = f32::from(u8::try_from(columns).unwrap_or(u8::MAX));
    columns * CARD_SIZE.x + (columns - 1.0) * GAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_maps_match_templates() {
        let screen = StartScreen::default();

        for (index, template) in TEMPLATES.iter().enumerate() {
            let map = screen.template_map(index).expect("index in range");
            let root = map.node(map.root()).expect("root exists");
            assert_eq!(root.text(), template.root.text);
        }
        assert!(screen.template_map(TEMPLATES.len()).is_none());
    }

    #[test]
    fn grid_width_includes_gaps() {
        assert_eq!(grid_width(1), CARD_SIZE.x);
        assert_eq!(grid_width(3), CARD_SIZE.x * 3.0 + GAP * 2.0);
    }
}
