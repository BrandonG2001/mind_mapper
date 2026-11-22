//! The fonts labels are drawn in: egui's built-in Ubuntu Light, plus Ubuntu Bold (bundled
//! in `assets/fonts`, under the same Ubuntu Font Licence) for bold labels.

use eframe::egui::{
    Context, FontData, FontFamily,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
};

/// The bold font file.
pub const UBUNTU_BOLD: &[u8] = include_bytes!("../assets/fonts/Ubuntu-Bold.ttf");
/// The egui font family name bold labels use.
const BOLD_FAMILY: &str = "bold";

/// Make the bold font available to `ctx`. Cheap after the first call, so it can run at
/// the start of every frame (it must run before any bold text is laid out).
pub fn install(ctx: &Context) {
    ctx.add_font(FontInsert {
        name: "Ubuntu-Bold".to_owned(),
        data: FontData::from_static(UBUNTU_BOLD),
        families: vec![InsertFontFamily {
            family: FontFamily::Name(BOLD_FAMILY.into()),
            priority: FontPriority::Highest,
        }],
    });
}

/// The font family for a label: bold, or the regular one.
#[must_use]
pub fn family(bold: bool) -> FontFamily {
    if bold {
        FontFamily::Name(BOLD_FAMILY.into())
    } else {
        FontFamily::Proportional
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bold_font_is_a_bold_ubuntu() {
        let face = ttf_parser::Face::parse(UBUNTU_BOLD, 0).expect("a valid font");
        assert_eq!(face.weight(), ttf_parser::Weight::Bold);
        assert_eq!(family(false), FontFamily::Proportional);
    }
}
