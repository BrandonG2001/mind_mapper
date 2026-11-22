//! The application icon, drawn from `assets/icon.svg`.

use eframe::egui::IconData;
use resvg::{tiny_skia, usvg};

use crate::export::ExportError;

/// The icon artwork, compiled into the program.
const ICON_SVG: &str = include_str!("../assets/icon.svg");
/// Width and height of the rendered window icon, in pixels.
const ICON_SIZE: u16 = 256;

/// Render the window icon.
///
/// # Errors
///
/// Returns `ExportError::Render` if the embedded artwork cannot be drawn (a bug, which the
/// unit test below guards against).
pub fn window_icon() -> Result<IconData, ExportError> {
    let tree = usvg::Tree::from_str(ICON_SVG, &usvg::Options::default())
        .map_err(|err| ExportError::Render(err.to_string()))?;
    let size = u32::from(ICON_SIZE);
    let mut pixmap = tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| ExportError::Render("invalid icon size".to_owned()))?;
    let scale = f32::from(ICON_SIZE) / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // The renderer stores colours pre-multiplied by alpha; window icons expect them
    // separate ("straight" alpha), so undo the multiplication for every pixel.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Ok(IconData {
        rgba,
        width: size,
        height: size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_renders_with_transparent_corners_and_opaque_centre() {
        let icon = window_icon().expect("embedded icon renders");
        let size = usize::from(ICON_SIZE);
        let alpha_at = |x: usize, y: usize| icon.rgba[(y * size + x) * 4 + 3];

        assert_eq!(icon.rgba.len(), size * size * 4);
        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(size / 2, size / 2), 255);
    }
}
