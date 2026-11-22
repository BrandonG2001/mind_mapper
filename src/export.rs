//! Exporting a map as a PNG image, an SVG drawing or a Markdown outline.
//!
//! SVG and PNG show what is on screen: folded branches stay folded, and colours come from
//! the current theme. Markdown is an outline of every node, folded or not.

use std::fmt::{self, Write as _};
use std::path::Path;
use std::sync::Arc;

use eframe::egui::{Color32, Rect, Visuals};
use resvg::{tiny_skia, usvg};
use thiserror::Error;

use crate::clipboard;
use crate::map_view::{BASE_FONT_SIZE, LINE_HEIGHT, branch_accent, estimated_rect, wrap_label};
use crate::model::MindMap;
use crate::storage::{StorageError, write_atomically};

/// Empty space around the map in exported images, in world units.
const MARGIN: f32 = 40.0;
/// Rounded-corner radius of node boxes.
const CORNER_RADIUS: f32 = 8.0;
/// Width of edges and node borders.
const STROKE_WIDTH: f32 = 2.0;
/// PNG pixels per world unit, for a sharp image on high-density screens.
const PNG_SCALE: f32 = 2.0;
/// Largest PNG width or height; bigger maps are scaled down to fit.
const MAX_PNG_SIDE: f32 = 8192.0;
/// Fonts listed in exported SVGs; the first is egui's built-in label font.
const FONT_FAMILY: &str = "Ubuntu, sans-serif";

/// Errors that can occur while exporting.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The chosen file name has no extension that names a known format.
    #[error("the file name must end in .png, .svg or .md")]
    UnknownFormat,
    /// The image could not be rendered or encoded.
    #[error("could not render the image: {0}")]
    Render(String),
    /// The file could not be written.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// A file format that maps can be exported to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// A raster image.
    Png,
    /// A vector drawing.
    Svg,
    /// A nested bullet outline.
    Markdown,
}

impl ExportFormat {
    /// The format named by `path`'s extension (case-insensitive), if any.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            "md" | "markdown" => Some(Self::Markdown),
            _ => None,
        }
    }
}

/// Colours used for exported drawings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportStyle {
    /// Page background.
    pub background: Color32,
    /// Inside of node boxes.
    pub node_fill: Color32,
    /// Labels, and the root's border.
    pub text: Color32,
}

impl ExportStyle {
    /// Take the colours the canvas uses with `visuals`.
    #[must_use]
    pub fn from_visuals(visuals: &Visuals) -> Self {
        Self {
            background: visuals.extreme_bg_color,
            node_fill: visuals.window_fill(),
            text: visuals.strong_text_color(),
        }
    }
}

/// Write `map` to `path` in the format given by the path's extension.
///
/// # Errors
///
/// * `ExportError::UnknownFormat` if the extension is not `.png`, `.svg` or `.md`
/// * `ExportError::Render` if the PNG cannot be produced
/// * `ExportError::Storage` if the file cannot be written
pub fn export(map: &MindMap, style: ExportStyle, path: &Path) -> Result<(), ExportError> {
    let bytes = match ExportFormat::from_path(path).ok_or(ExportError::UnknownFormat)? {
        ExportFormat::Png => to_png(map, style)?,
        ExportFormat::Svg => to_svg(map, style).into_bytes(),
        ExportFormat::Markdown => to_markdown(map).into_bytes(),
    };
    write_atomically(path, |writer| Ok(writer.write_all(&bytes)?))?;
    Ok(())
}

/// The map as a Markdown outline: the root as a heading, every other node as a nested
/// bullet in tree order.
#[must_use]
pub fn to_markdown(map: &MindMap) -> String {
    let root = map.root();
    let mut out = String::new();
    if let Some(root_node) = map.node(root) {
        out.push_str("# ");
        // A line break inside the title would end the heading early.
        out.push_str(&root_node.text().replace(['\r', '\n'], " "));
        out.push_str("\n\n");
        for &branch in root_node.children() {
            out.push_str(&clipboard::branch_outline(map, branch));
        }
    }
    out
}

/// The visible part of the map as a standalone SVG document.
///
/// # Panics
///
/// Never in practice: writing into a `String` cannot fail.
#[must_use]
pub fn to_svg(map: &MindMap, style: ExportStyle) -> String {
    let mut out = String::new();
    write_svg(&mut out, map, style).expect("formatting into a String cannot fail");
    out
}

fn write_svg(out: &mut String, map: &MindMap, style: ExportStyle) -> fmt::Result {
    let bounds = map
        .visible_preorder()
        .fold(Rect::NOTHING, |bounds, visit| {
            bounds.union(estimated_rect(visit.node))
        })
        .expand(MARGIN);
    let (width, height) = (bounds.width(), bounds.height());
    // Shift world coordinates so the drawing starts at (0, 0).
    let at = |x: f32, y: f32| (x - bounds.min.x, y - bounds.min.y);

    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.1} {height:.1}">"#
    )?;
    writeln!(
        out,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        hex(style.background)
    )?;
    // Edges first, so node boxes cover their ends.
    for visit in map.visible_preorder() {
        let Some(parent) = visit.node.parent().and_then(|id| map.node(id)) else {
            continue;
        };
        let (x1, y1) = at(parent.position().x, parent.position().y);
        let (x2, y2) = at(visit.node.position().x, visit.node.position().y);
        let bend = (x2 - x1) * 0.5;
        writeln!(
            out,
            r#"<path d="M {x1:.1} {y1:.1} C {:.1} {y1:.1}, {:.1} {y2:.1}, {x2:.1} {y2:.1}" fill="none" stroke="{}" stroke-width="{STROKE_WIDTH}"/>"#,
            x1 + bend,
            x2 - bend,
            hex(branch_accent(visit.branch, style.text)),
        )?;
    }
    for visit in map.visible_preorder() {
        let rect = estimated_rect(visit.node);
        let (x, y) = at(rect.min.x, rect.min.y);
        writeln!(
            out,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{:.1}" rx="{CORNER_RADIUS}" fill="{}" stroke="{}" stroke-width="{STROKE_WIDTH}"/>"#,
            rect.width(),
            rect.height(),
            hex(style.node_fill),
            hex(branch_accent(visit.branch, style.text)),
        )?;
        let (center_x, center_y) = at(rect.center().x, rect.center().y);
        let line_count =
            f32::from(u16::try_from(wrap_label(visit.node.text()).count()).unwrap_or(u16::MAX));
        // Baseline of the first line: lines are centred as a block, and a baseline sits
        // about 0.35 em below the middle of its line.
        let mut baseline =
            center_y - (line_count - 1.0) * LINE_HEIGHT * 0.5 + BASE_FONT_SIZE * 0.35;
        write!(
            out,
            r#"<text font-family="{FONT_FAMILY}" font-size="{BASE_FONT_SIZE}" fill="{}" text-anchor="middle">"#,
            hex(style.text)
        )?;
        for line in wrap_label(visit.node.text()) {
            write!(out, r#"<tspan x="{center_x:.1}" y="{baseline:.1}">"#)?;
            write_escaped(out, line)?;
            out.push_str("</tspan>");
            baseline += LINE_HEIGHT;
        }
        out.push_str("</text>\n");
    }
    out.push_str("</svg>\n");
    Ok(())
}

/// Render the visible part of the map to PNG bytes, using egui's built-in font so the
/// image looks like the app and needs no installed fonts.
///
/// # Errors
///
/// Returns `ExportError::Render` if the drawing cannot be rasterised or encoded.
pub fn to_png(map: &MindMap, style: ExportStyle) -> Result<Vec<u8>, ExportError> {
    let mut options = usvg::Options::default();
    // `Arc::make_mut` gives mutable access to the freshly created font database.
    let fonts = Arc::make_mut(&mut options.fontdb);
    fonts.load_font_data(epaint_default_fonts::UBUNTU_LIGHT.to_vec());
    // Make that font the one SVG's generic `sans-serif` resolves to.
    let family = fonts
        .faces()
        .next()
        .and_then(|face| face.families.first())
        .map(|(name, _)| name.clone());
    if let Some(family) = family {
        fonts.set_sans_serif_family(family);
    }

    let tree = usvg::Tree::from_str(&to_svg(map, style), &options)
        .map_err(|err| ExportError::Render(err.to_string()))?;
    let size = tree.size();
    let scale = PNG_SCALE.min(MAX_PNG_SIDE / size.width().max(size.height()));
    let mut pixmap = tiny_skia::Pixmap::new(
        pixel_count(size.width() * scale),
        pixel_count(size.height() * scale),
    )
    .ok_or_else(|| ExportError::Render("the image is too large".to_owned()))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|err| ExportError::Render(err.to_string()))
}

/// Round a pixel length up to a whole number within `1..=MAX_PNG_SIDE`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
// The value is clamped to a small positive range first, so the cast is exact.
fn pixel_count(length: f32) -> u32 {
    length.ceil().clamp(1.0, MAX_PNG_SIDE) as u32
}

/// `#rrggbb` for an opaque colour.
fn hex(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

/// Write `text` with the characters that are special in XML escaped.
fn write_escaped(out: &mut String, text: &str) -> fmt::Result {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.write_char(other)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;
    use crate::test_support::TempDir;

    fn sample() -> MindMap {
        let mut map = MindMap::new("Plan");
        let goals = map
            .add_child_at(map.root(), "Goals & <aims>", Point::new(200.0, 0.0))
            .expect("root exists");
        map.add_child_at(goals, "Ship it", Point::new(400.0, 0.0))
            .expect("goals exist");
        map.add_child_at(map.root(), "Risks", Point::new(-200.0, 0.0))
            .expect("root exists");
        map
    }

    fn style() -> ExportStyle {
        ExportStyle::from_visuals(&Visuals::dark())
    }

    #[test]
    fn format_comes_from_extension() {
        assert_eq!(
            ExportFormat::from_path(Path::new("a.PNG")),
            Some(ExportFormat::Png)
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("a.svg")),
            Some(ExportFormat::Svg)
        );
        assert_eq!(
            ExportFormat::from_path(Path::new("a.markdown")),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(ExportFormat::from_path(Path::new("a.txt")), None);
        assert_eq!(ExportFormat::from_path(Path::new("a")), None);
    }

    #[test]
    fn markdown_is_a_nested_outline_of_every_node() {
        let mut map = sample();
        let goals = map.node(map.root()).expect("root").children()[0];
        map.set_collapsed(goals, true).expect("goals exist");

        assert_eq!(
            to_markdown(&map),
            "# Plan\n\n- Goals & <aims>\n  - Ship it\n- Risks\n"
        );
    }

    #[test]
    fn svg_draws_visible_nodes_with_escaped_labels() {
        let mut map = sample();

        let svg = to_svg(&map, style());

        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("Goals &amp; &lt;aims&gt;"));
        assert!(svg.contains(">Ship it<"));
        assert_eq!(svg.matches("<path ").count(), 3);
        let goals = map.node(map.root()).expect("root").children()[0];
        map.set_collapsed(goals, true).expect("goals exist");
        let folded = to_svg(&map, style());
        assert!(!folded.contains(">Ship it<"));
        assert_eq!(folded.matches("<path ").count(), 2);
    }

    #[test]
    fn png_renders_a_valid_image() {
        let png = to_png(&sample(), style()).expect("renders");

        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        // Width and height are big-endian u32s at bytes 16..24 of a PNG.
        let width = u32::from_be_bytes(png[16..20].try_into().expect("4 bytes"));
        let height = u32::from_be_bytes(png[20..24].try_into().expect("4 bytes"));
        assert!(width > height && height > 0, "{width}x{height}");
    }

    #[test]
    fn export_writes_each_format() {
        let dir = TempDir::new("export");
        let map = sample();

        for name in ["map.png", "map.svg", "map.md"] {
            let path = dir.path().join(name);
            export(&map, style(), &path).expect("export succeeds");
            assert!(std::fs::metadata(&path).expect("file exists").len() > 0);
        }
        assert!(matches!(
            export(&map, style(), &dir.path().join("map.doc")),
            Err(ExportError::UnknownFormat)
        ));
    }
}
