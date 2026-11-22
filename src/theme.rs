//! Colour themes the user can pick from.

use eframe::Storage;
use eframe::egui::{self, Color32, Theme, Ui, Visuals};
use serde::{Deserialize, Serialize};

/// Key under which the chosen theme is kept in eframe's settings storage.
const STORAGE_KEY: &str = "color_theme";
/// Key under which the interface size (egui's zoom factor) is kept.
const UI_ZOOM_KEY: &str = "ui_zoom";
/// Interface sizes accepted when loading, matching the range egui's zoom keys allow.
const UI_ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.2..=5.0;

/// A selectable colour scheme for the whole window.
///
/// Serializable so the choice can be remembered between runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorTheme {
    /// egui's standard dark theme.
    #[default]
    Dark,
    /// egui's standard light theme.
    Light,
    /// Soft light-grey surfaces.
    LightGrey,
    /// Charcoal surfaces.
    DarkGrey,
    /// Navy surfaces with a bright blue accent.
    DarkBlue,
    /// Lavender surfaces with a violet accent.
    LightPurple,
    /// Deep plum surfaces with a lilac accent.
    DarkPurple,
}

/// The colours that distinguish a custom theme from egui's stock dark/light themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Palette {
    /// Map canvas and text-field background.
    canvas: Color32,
    /// Panels, windows and node fills.
    surface: Color32,
    /// Buttons at rest.
    widget: Color32,
    /// Buttons under the mouse.
    widget_hover: Color32,
    /// Selection highlight and links.
    accent: Color32,
}

impl ColorTheme {
    /// Every theme, in the order shown in the picker.
    pub const ALL: [Self; 7] = [
        Self::Dark,
        Self::Light,
        Self::LightGrey,
        Self::DarkGrey,
        Self::DarkBlue,
        Self::LightPurple,
        Self::DarkPurple,
    ];

    /// Human-readable name for the picker.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::LightGrey => "Light grey",
            Self::DarkGrey => "Dark grey",
            Self::DarkBlue => "Dark blue",
            Self::LightPurple => "Light purple",
            Self::DarkPurple => "Dark purple",
        }
    }

    /// Whether the theme uses light text on dark backgrounds.
    #[must_use]
    pub const fn is_dark(self) -> bool {
        match self {
            Self::Dark | Self::DarkGrey | Self::DarkBlue | Self::DarkPurple => true,
            Self::Light | Self::LightGrey | Self::LightPurple => false,
        }
    }

    /// The custom colours, or `None` for egui's stock themes.
    const fn palette(self) -> Option<Palette> {
        // `const fn` cannot call `Color32::from_hex`, so colours are written as RGB.
        let rgb = Color32::from_rgb;
        match self {
            Self::Dark | Self::Light => None,
            Self::LightGrey => Some(Palette {
                canvas: rgb(0xd2, 0xd4, 0xd7),
                surface: rgb(0xe6, 0xe7, 0xe9),
                widget: rgb(0xc6, 0xc9, 0xcd),
                widget_hover: rgb(0xb7, 0xbb, 0xc0),
                accent: rgb(0x4a, 0x5d, 0x73),
            }),
            Self::DarkGrey => Some(Palette {
                canvas: rgb(0x2b, 0x2b, 0x2d),
                surface: rgb(0x3a, 0x3a, 0x3d),
                widget: rgb(0x4a, 0x4a, 0x4e),
                widget_hover: rgb(0x5a, 0x5a, 0x5f),
                accent: rgb(0xa4, 0xb6, 0xcc),
            }),
            Self::DarkBlue => Some(Palette {
                canvas: rgb(0x0e, 0x16, 0x2b),
                surface: rgb(0x16, 0x22, 0x40),
                widget: rgb(0x1f, 0x31, 0x5c),
                widget_hover: rgb(0x2a, 0x41, 0x7a),
                accent: rgb(0x5c, 0x9d, 0xff),
            }),
            Self::LightPurple => Some(Palette {
                canvas: rgb(0xe3, 0xd7, 0xf3),
                surface: rgb(0xf2, 0xeb, 0xfa),
                widget: rgb(0xd6, 0xc5, 0xee),
                widget_hover: rgb(0xc8, 0xb1, 0xe6),
                accent: rgb(0x6e, 0x40, 0xb8),
            }),
            Self::DarkPurple => Some(Palette {
                canvas: rgb(0x1b, 0x13, 0x2c),
                surface: rgb(0x29, 0x1e, 0x3e),
                widget: rgb(0x3b, 0x2b, 0x58),
                widget_hover: rgb(0x4c, 0x37, 0x70),
                accent: rgb(0xb8, 0x8f, 0xff),
            }),
        }
    }

    /// Build the egui style for this theme.
    #[must_use]
    pub fn visuals(self) -> Visuals {
        let mut visuals = if self.is_dark() {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let Some(palette) = self.palette() else {
            return visuals;
        };
        visuals.panel_fill = palette.surface;
        visuals.window_fill = palette.surface;
        visuals.faint_bg_color = palette.surface;
        visuals.extreme_bg_color = palette.canvas;
        visuals.code_bg_color = palette.widget;
        visuals.hyperlink_color = palette.accent;
        visuals.selection.bg_fill = palette.accent.gamma_multiply(0.45);
        visuals.selection.stroke.color = palette.accent;
        let widgets = &mut visuals.widgets;
        for (state, fill) in [
            (&mut widgets.inactive, palette.widget),
            (&mut widgets.hovered, palette.widget_hover),
            (&mut widgets.active, palette.widget_hover),
            (&mut widgets.open, palette.widget),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
        }
        widgets.noninteractive.bg_fill = palette.surface;
        widgets.noninteractive.weak_bg_fill = palette.surface;
        visuals
    }

    /// Read the theme that versions before 0.5 kept in eframe's storage (used once, to
    /// carry it over into the shared settings file).
    ///
    /// # Arguments
    ///
    /// * `storage` - eframe's settings storage; `None` when persistence is unavailable
    ///
    /// # Returns
    ///
    /// The saved theme, or the default if nothing (or something unreadable, such as a
    /// theme removed in a later version) was stored.
    #[must_use]
    pub fn load(storage: Option<&dyn Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default()
    }

    /// Install this theme's style in `ctx` and switch egui to it.
    pub fn apply(self, ctx: &egui::Context) {
        let base = if self.is_dark() {
            Theme::Dark
        } else {
            Theme::Light
        };
        ctx.set_visuals_of(base, self.visuals());
        ctx.set_theme(base);
    }
}

/// Read the interface size that versions before 0.5 kept in eframe's storage (used once,
/// to carry it over into the shared settings file).
///
/// # Returns
///
/// The saved zoom factor (1.0 = 100%), or `None` if nothing valid was stored.
#[must_use]
pub fn load_ui_zoom(storage: Option<&dyn Storage>) -> Option<f32> {
    storage
        .and_then(|storage| eframe::get_value(storage, UI_ZOOM_KEY))
        .filter(|zoom| UI_ZOOM_RANGE.contains(zoom))
}

/// "UI size  -  100%  +" buttons that scale the whole interface (also Ctrl +, Ctrl - and
/// Ctrl 0, which egui handles itself).
///
/// Reads correctly in both left-to-right and right-to-left layouts.
pub fn ui_size_control(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let zoom_out_ctx = ctx.clone();
    let zoom_in_ctx = ctx.clone();
    let mut label = |ui: &mut Ui| {
        ui.label("UI size");
    };
    let mut smaller = |ui: &mut Ui| {
        if ui
            .small_button("-")
            .on_hover_text("Smaller (Ctrl -)")
            .clicked()
        {
            egui::gui_zoom::zoom_out(&zoom_out_ctx);
        }
    };
    let mut reset = |ui: &mut Ui| {
        let percent = format!("{:.0}%", ctx.zoom_factor() * 100.0);
        if ui
            .small_button(percent)
            .on_hover_text("Reset to 100% (Ctrl 0)")
            .clicked()
        {
            ctx.set_zoom_factor(1.0);
        }
    };
    let mut larger = |ui: &mut Ui| {
        if ui
            .small_button("+")
            .on_hover_text("Larger (Ctrl +)")
            .clicked()
        {
            egui::gui_zoom::zoom_in(&zoom_in_ctx);
        }
    };
    // `&mut dyn FnMut` lets closures of different types share one array.
    let mut parts: [&mut dyn FnMut(&mut Ui); 4] =
        [&mut label, &mut smaller, &mut reset, &mut larger];
    // A right-to-left layout places each new widget to the left of the previous one, so
    // add them in reverse to keep the reading order.
    if ui.layout().prefer_right_to_left() {
        parts.reverse();
    }
    for part in parts {
        part(ui);
    }
}

/// A drop-down for choosing the colour theme.
///
/// # Arguments
///
/// * `ui` - Where to draw the drop-down
/// * `theme` - Current theme; updated in place when the user picks another
pub fn theme_picker(ui: &mut Ui, theme: &mut ColorTheme) {
    egui::ComboBox::from_id_salt("color_theme")
        .selected_text(theme.name())
        .show_ui(ui, |ui| {
            for option in ColorTheme::ALL {
                ui.selectable_value(theme, option, option.name());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MemoryStorage;

    #[test]
    fn saved_theme_is_loaded_back() {
        for theme in ColorTheme::ALL {
            let mut storage = MemoryStorage::default();

            eframe::set_value(&mut storage, STORAGE_KEY, &theme);

            assert_eq!(ColorTheme::load(Some(&storage)), theme);
        }
    }

    #[test]
    fn ui_zoom_round_trips_and_rejects_nonsense() {
        let mut storage = MemoryStorage::default();
        assert_eq!(load_ui_zoom(Some(&storage)), None);

        eframe::set_value(&mut storage, UI_ZOOM_KEY, &1.5_f32);
        assert_eq!(load_ui_zoom(Some(&storage)), Some(1.5));
        eframe::set_value(&mut storage, UI_ZOOM_KEY, &50.0_f32);

        assert_eq!(load_ui_zoom(Some(&storage)), None);
        assert_eq!(load_ui_zoom(None), None);
    }

    #[test]
    fn missing_or_corrupt_value_falls_back_to_default() {
        let mut storage = MemoryStorage::default();
        assert_eq!(ColorTheme::load(None), ColorTheme::Dark);
        assert_eq!(ColorTheme::load(Some(&storage)), ColorTheme::Dark);

        storage.set_string(STORAGE_KEY, "NoSuchTheme".to_owned());

        assert_eq!(ColorTheme::load(Some(&storage)), ColorTheme::Dark);
    }

    #[test]
    fn all_lists_every_theme_once_with_unique_names() {
        for (index, theme) in ColorTheme::ALL.iter().enumerate() {
            assert!(
                ColorTheme::ALL[index + 1..]
                    .iter()
                    .all(|other| other != theme && other.name() != theme.name())
            );
        }
    }

    #[test]
    fn visuals_match_dark_or_light_base() {
        for theme in ColorTheme::ALL {
            assert_eq!(
                theme.visuals().dark_mode,
                theme.is_dark(),
                "{}",
                theme.name()
            );
        }
    }

    #[test]
    fn stock_themes_are_unchanged() {
        assert_eq!(ColorTheme::Dark.visuals(), Visuals::dark());
        assert_eq!(ColorTheme::Light.visuals(), Visuals::light());
    }

    #[test]
    fn custom_themes_apply_their_palette() {
        for theme in ColorTheme::ALL {
            if let Some(palette) = theme.palette() {
                let visuals = theme.visuals();
                assert_eq!(visuals.extreme_bg_color, palette.canvas);
                assert_eq!(visuals.window_fill, palette.surface);
                assert_eq!(visuals.selection.stroke.color, palette.accent);
                assert_ne!(palette.canvas, palette.surface, "{}", theme.name());
            }
        }
    }

    #[test]
    fn default_is_dark() {
        assert_eq!(ColorTheme::default(), ColorTheme::Dark);
    }
}
