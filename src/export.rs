//! Exporting a map as a PNG image, an SVG drawing, a PDF document, a Markdown outline, an
//! OPML outline or a FreeMind map.
//!
//! The pictures (PNG, SVG and PDF) show what is on screen: folded branches stay folded.
//! PNG and SVG use the current theme's colours; PDF, being for printing, always uses light
//! ones. The outline formats hold every node, folded or not.

use std::fmt::{self, Write as _};
use std::path::Path;
use std::sync::Arc;

use eframe::egui::{Color32, Pos2, Rect, Vec2, Visuals};
use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{Path as PdfPath, PathBuilder, Point as KrillaPoint, Transform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke, StrokeDash};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use resvg::{tiny_skia, usvg};
use thiserror::Error;

use crate::clipboard;
use crate::formats::{Format, freemind, opml, push_escaped};
use crate::map_view::{
    ARROW_SIZE, Accents, BASE_FONT_SIZE, LINE_HEIGHT, LINK_DASH, LINK_OPACITY, LINK_WIDTH,
    arrow_head, cross_link_curve, estimated_rect, wrap_label,
};
use crate::model::{MindMap, NodeId};
use crate::storage::{StorageError, write_atomically};
use crate::theme::ColorTheme;

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
/// Largest PDF page side, in points; PDF readers commonly refuse anything bigger.
const MAX_PDF_SIDE: f32 = 14_400.0;
/// Fonts listed in exported SVGs; the first is egui's built-in label font.
const FONT_FAMILY: &str = "Ubuntu, sans-serif";

/// Errors that can occur while exporting.
#[derive(Debug, Error)]
pub enum ExportError {
    /// The chosen file name has no extension that names a known format.
    #[error("the file name must end in .png, .svg, .pdf, .md, .opml or .mm")]
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
    /// A printable document.
    Pdf,
    /// A nested bullet outline.
    Markdown,
    /// A map in another app's format.
    Other(Format),
}

impl ExportFormat {
    /// The format named by `path`'s extension (case-insensitive), if any.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            "pdf" => Some(Self::Pdf),
            "md" | "markdown" => Some(Self::Markdown),
            _ => Format::from_path(path).map(Self::Other),
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
/// * `ExportError::UnknownFormat` if the extension is not `.png`, `.svg`, `.pdf`, `.md`,
///   `.opml` or `.mm`
/// * `ExportError::Render` if the PNG or PDF cannot be produced
/// * `ExportError::Storage` if the file cannot be written
pub fn export(map: &MindMap, style: ExportStyle, path: &Path) -> Result<(), ExportError> {
    let bytes = match ExportFormat::from_path(path).ok_or(ExportError::UnknownFormat)? {
        ExportFormat::Png => to_png(map, style)?,
        ExportFormat::Svg => to_svg(map, style).into_bytes(),
        ExportFormat::Pdf => to_pdf(map)?,
        ExportFormat::Markdown => to_markdown(map).into_bytes(),
        ExportFormat::Other(Format::Opml) => opml::write(map).into_bytes(),
        ExportFormat::Other(Format::FreeMind) => freemind::write(map).into_bytes(),
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
        out.push_str(&clipboard::branches_outline(map, root_node.children()));
    }
    out
}

/// What an SVG or PDF export draws, worked out once for both: page coordinates with the
/// origin at the top left and y growing downwards, one unit per world unit.
struct Scene<'a> {
    /// Page size, including the margin.
    size: Vec2,
    /// Curves from each parent to each child, drawn first so node boxes cover their ends.
    edges: Vec<SceneEdge>,
    /// Cross-links whose ends are both shown, drawn over the edges as dashed arrows.
    links: Vec<[Pos2; 4]>,
    /// Node boxes with their labels, drawn over the edges and links.
    nodes: Vec<SceneNode<'a>>,
}

/// A cubic curve from a parent's centre to a child's.
struct SceneEdge {
    /// Start, two control points, end.
    points: [Pos2; 4],
    color: Color32,
}

/// A node's box and the lines of its label.
struct SceneNode<'a> {
    rect: Rect,
    accent: Color32,
    /// Each line's horizontal centre and baseline, with its text.
    lines: Vec<(Pos2, &'a str)>,
}

impl<'a> Scene<'a> {
    /// Lay out the visible part of `map` (folded branches stay folded).
    fn new(map: &'a MindMap, root_color: Color32) -> Self {
        let bounds = map
            .visible_preorder()
            .fold(Rect::NOTHING, |bounds, visit| {
                bounds.union(estimated_rect(visit.node))
            })
            .expand(MARGIN);
        // Shift world coordinates so the drawing starts at (0, 0).
        let at = |point: Pos2| point - bounds.min.to_vec2();
        let mut accents = Accents::default();
        accents.reset(map);
        let mut edges = Vec::new();
        let mut nodes = Vec::with_capacity(map.len());
        // Each shown node's box by slot, to find where cross-links start and end.
        let mut boxes: Vec<Option<Rect>> = vec![None; map.slot_count()];
        for visit in map.visible_preorder() {
            let accent = accents.resolve(map, &visit, root_color);
            let position = visit.node.position();
            let center = at(Pos2::new(position.x, position.y));
            if let Some(parent) = visit.node.parent().and_then(|id| map.node(id)) {
                let start = at(Pos2::new(parent.position().x, parent.position().y));
                let bend = Vec2::new((center.x - start.x) * 0.5, 0.0);
                edges.push(SceneEdge {
                    points: [start, start + bend, center - bend, center],
                    color: accent,
                });
            }
            let world_rect = estimated_rect(visit.node);
            let rect = Rect::from_min_size(at(world_rect.min), world_rect.size());
            boxes[visit.id.index()] = Some(rect);
            let line_count =
                f32::from(u16::try_from(wrap_label(visit.node.text()).count()).unwrap_or(u16::MAX));
            // Baseline of the first line: lines are centred as a block, and a baseline
            // sits about 0.35 em below the middle of its line.
            let first_baseline =
                rect.center().y - (line_count - 1.0) * LINE_HEIGHT * 0.5 + BASE_FONT_SIZE * 0.35;
            let lines = wrap_label(visit.node.text())
                .scan(first_baseline, |baseline, line| {
                    let anchor = Pos2::new(rect.center().x, *baseline);
                    *baseline += LINE_HEIGHT;
                    Some((anchor, line))
                })
                .collect();
            nodes.push(SceneNode {
                rect,
                accent,
                lines,
            });
        }
        let box_of = |id: NodeId| boxes.get(id.index()).copied().flatten();
        let links = map
            .cross_links()
            .iter()
            .filter_map(|link| Some(cross_link_curve(box_of(link.from())?, box_of(link.to())?)))
            .collect();
        Self {
            size: bounds.size(),
            edges,
            links,
            nodes,
        }
    }
}

/// The visible part of the map as a standalone SVG document.
///
/// # Panics
///
/// Never in practice: writing into a `String` cannot fail.
#[must_use]
pub fn to_svg(map: &MindMap, style: ExportStyle) -> String {
    let mut out = String::new();
    write_svg(&mut out, &Scene::new(map, style.text), style)
        .expect("formatting into a String cannot fail");
    out
}

fn write_svg(out: &mut String, scene: &Scene<'_>, style: ExportStyle) -> fmt::Result {
    let (width, height) = (scene.size.x, scene.size.y);
    writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.1} {height:.1}">"#
    )?;
    writeln!(
        out,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        hex(style.background)
    )?;
    for edge in &scene.edges {
        writeln!(
            out,
            r#"<path d="{}" fill="none" stroke="{}" stroke-width="{STROKE_WIDTH}"/>"#,
            curve_data(edge.points),
            hex(edge.color),
        )?;
    }
    let link_color = hex(style.text);
    for &curve in &scene.links {
        writeln!(
            out,
            r#"<path d="{}" fill="none" stroke="{link_color}" stroke-opacity="{LINK_OPACITY}" stroke-width="{LINK_WIDTH}" stroke-dasharray="{} {}"/>"#,
            curve_data(curve),
            LINK_DASH[0],
            LINK_DASH[1],
        )?;
        let [tip, left, right] = arrow_head(curve, ARROW_SIZE);
        writeln!(
            out,
            r#"<polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}" fill="{link_color}" fill-opacity="{LINK_OPACITY}"/>"#,
            tip.x, tip.y, left.x, left.y, right.x, right.y,
        )?;
    }
    for node in &scene.nodes {
        let rect = node.rect;
        writeln!(
            out,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{CORNER_RADIUS}" fill="{}" stroke="{}" stroke-width="{STROKE_WIDTH}"/>"#,
            rect.min.x,
            rect.min.y,
            rect.width(),
            rect.height(),
            hex(style.node_fill),
            hex(node.accent),
        )?;
        write!(
            out,
            r#"<text font-family="{FONT_FAMILY}" font-size="{BASE_FONT_SIZE}" fill="{}" text-anchor="middle">"#,
            hex(style.text)
        )?;
        for &(anchor, line) in &node.lines {
            write!(out, r#"<tspan x="{:.1}" y="{:.1}">"#, anchor.x, anchor.y)?;
            push_escaped(out, line);
            out.push_str("</tspan>");
        }
        out.push_str("</text>\n");
    }
    out.push_str("</svg>\n");
    Ok(())
}

/// SVG path data for a cubic curve.
fn curve_data([start, first, second, end]: [Pos2; 4]) -> String {
    format!(
        "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
        start.x, start.y, first.x, first.y, second.x, second.y, end.x, end.y
    )
}

/// The visible part of the map as a PDF document of one page, in light colours whatever
/// the theme (it is meant for printing), with egui's built-in font embedded so the text
/// stays selectable and needs no installed fonts.
///
/// # Errors
///
/// Returns `ExportError::Render` if the font cannot be read or the document not written.
pub fn to_pdf(map: &MindMap) -> Result<Vec<u8>, ExportError> {
    let style = ExportStyle::from_visuals(&ColorTheme::Light.visuals());
    let scene = Scene::new(map, style.text);
    let font_bytes = epaint_default_fonts::UBUNTU_LIGHT;
    let font = Font::new(font_bytes.into(), 0)
        .ok_or_else(|| ExportError::Render("the built-in font could not be read".to_owned()))?;
    let face = ttf_parser::Face::parse(font_bytes, 0)
        .map_err(|err| ExportError::Render(err.to_string()))?;
    // Many PDF readers refuse pages larger than this, so huge maps are scaled down.
    let scale = (MAX_PDF_SIDE / scene.size.x.max(scene.size.y)).min(1.0);
    let page_settings = PageSettings::from_wh(scene.size.x * scale, scene.size.y * scale)
        .ok_or_else(|| ExportError::Render("the page has no size".to_owned()))?;

    let mut document = Document::new();
    let mut page = document.start_page_with(page_settings);
    let mut surface = page.surface();
    surface.push_transform(&Transform::from_scale(scale, scale));
    let draw = |surface: &mut krilla::surface::Surface<'_>,
                path: Option<krilla::geom::Path>,
                fill: Option<Color32>,
                stroke: Option<Color32>| {
        if let Some(path) = path {
            surface.set_fill(fill.map(pdf_fill));
            surface.set_stroke(stroke.map(pdf_stroke));
            surface.draw_path(&path);
        }
    };

    let mut background = PathBuilder::new();
    if let Some(page_rect) = krilla::geom::Rect::from_xywh(0.0, 0.0, scene.size.x, scene.size.y) {
        background.push_rect(page_rect);
    }
    draw(
        &mut surface,
        background.finish(),
        Some(style.background),
        None,
    );
    for edge in &scene.edges {
        draw(
            &mut surface,
            curve_path(edge.points),
            None,
            Some(edge.color),
        );
    }
    for &curve in &scene.links {
        draw_cross_link(&mut surface, curve, style.text);
    }
    for node in &scene.nodes {
        draw(
            &mut surface,
            rounded_rect(node.rect, CORNER_RADIUS),
            Some(style.node_fill),
            Some(node.accent),
        );
        surface.set_stroke(None);
        surface.set_fill(Some(pdf_fill(style.text)));
        for &(anchor, line) in &node.lines {
            let glyphs = line_glyphs(&face, line);
            let width: f32 = glyphs.iter().map(|glyph| glyph.x_advance).sum::<f32>();
            let left = anchor.x - width * BASE_FONT_SIZE * 0.5;
            surface.draw_glyphs(
                KrillaPoint::from_xy(left, anchor.y),
                &glyphs,
                font.clone(),
                line,
                BASE_FONT_SIZE,
                false,
            );
        }
    }
    surface.pop();
    surface.finish();
    page.finish();
    document
        .finish()
        .map_err(|err| ExportError::Render(format!("{err:?}")))
}

/// The glyphs of one label line: one per character, each advancing by its width in
/// `face` (in ems, as krilla expects). No shaping engine is needed for the scripts the
/// label font covers; characters it lacks show as its "missing glyph" box, as in PNGs.
fn line_glyphs(face: &ttf_parser::Face<'_>, line: &str) -> Vec<KrillaGlyph> {
    let units_per_em = f32::from(face.units_per_em());
    line.char_indices()
        .map(|(at, character)| {
            // Glyph 0 is the font's "missing glyph".
            let glyph = face
                .glyph_index(character)
                .unwrap_or(ttf_parser::GlyphId(0));
            let advance = face.glyph_hor_advance(glyph).map_or(0.0, f32::from) / units_per_em;
            KrillaGlyph::new(
                GlyphId::new(u32::from(glyph.0)),
                advance,
                0.0,
                0.0,
                0.0,
                // Which bytes of `line` the glyph stands for, so copied text is right.
                at..at + character.len_utf8(),
                None,
            )
        })
        .collect()
}

/// A cubic curve as a PDF path.
fn curve_path([start, first, second, end]: [Pos2; 4]) -> Option<PdfPath> {
    let mut path = PathBuilder::new();
    path.move_to(start.x, start.y);
    path.cubic_to(first.x, first.y, second.x, second.y, end.x, end.y);
    path.finish()
}

/// Draw a cross-link as a translucent dashed curve with an arrowhead, as on screen.
fn draw_cross_link(surface: &mut krilla::surface::Surface<'_>, curve: [Pos2; 4], color: Color32) {
    let opacity = NormalizedF32::new(LINK_OPACITY).unwrap_or(NormalizedF32::ONE);
    if let Some(path) = curve_path(curve) {
        surface.set_fill(None);
        surface.set_stroke(Some(Stroke {
            width: LINK_WIDTH,
            opacity,
            dash: Some(StrokeDash {
                array: LINK_DASH.to_vec(),
                offset: 0.0,
            }),
            ..pdf_stroke(color)
        }));
        surface.draw_path(&path);
    }
    let [tip, left, right] = arrow_head(curve, ARROW_SIZE);
    let mut head = PathBuilder::new();
    head.move_to(tip.x, tip.y);
    head.line_to(left.x, left.y);
    head.line_to(right.x, right.y);
    head.close();
    if let Some(path) = head.finish() {
        surface.set_stroke(None);
        surface.set_fill(Some(Fill {
            opacity,
            ..pdf_fill(color)
        }));
        surface.draw_path(&path);
    }
}

/// A rectangle with corners rounded by `radius`, as a path of lines and quarter-circle
/// curves.
fn rounded_rect(rect: Rect, radius: f32) -> Option<PdfPath> {
    // How far along the tangent a cubic curve's control points sit to approximate a
    // quarter circle.
    const KAPPA: f32 = 0.552_284_8;
    let radius = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let handle = radius * (1.0 - KAPPA);
    let (left, top, right, bottom) = (rect.min.x, rect.min.y, rect.max.x, rect.max.y);
    let mut path = PathBuilder::new();
    path.move_to(left + radius, top);
    path.line_to(right - radius, top);
    path.cubic_to(
        right - handle,
        top,
        right,
        top + handle,
        right,
        top + radius,
    );
    path.line_to(right, bottom - radius);
    path.cubic_to(
        right,
        bottom - handle,
        right - handle,
        bottom,
        right - radius,
        bottom,
    );
    path.line_to(left + radius, bottom);
    path.cubic_to(
        left + handle,
        bottom,
        left,
        bottom - handle,
        left,
        bottom - radius,
    );
    path.line_to(left, top + radius);
    path.cubic_to(left, top + handle, left + handle, top, left + radius, top);
    path.close();
    path.finish()
}

/// A solid PDF fill of `color`.
fn pdf_fill(color: Color32) -> Fill {
    Fill {
        paint: rgb::Color::new(color.r(), color.g(), color.b()).into(),
        ..Fill::default()
    }
}

/// A solid PDF line of `color`, as wide as the SVG export's.
fn pdf_stroke(color: Color32) -> Stroke {
    Stroke {
        paint: rgb::Color::new(color.r(), color.g(), color.b()).into(),
        width: STROKE_WIDTH,
        ..Stroke::default()
    }
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
        assert_eq!(
            ExportFormat::from_path(Path::new("a.mm")),
            Some(ExportFormat::Other(Format::FreeMind))
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
    fn pdf_is_a_one_page_document_with_its_font_embedded() {
        let pdf = to_pdf(&sample()).expect("renders");

        assert!(pdf.starts_with(b"%PDF-"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Type/Pages/Count 1"), "one page");
        assert!(text.contains("/FontFile2"), "the font is embedded");
    }

    #[test]
    fn huge_maps_fit_on_a_page_readers_accept() {
        let mut map = MindMap::new("wide");
        map.add_child_at(map.root(), "far", Point::new(100_000.0, 0.0))
            .expect("root exists");

        let pdf = to_pdf(&map).expect("renders");

        let text = String::from_utf8_lossy(&pdf);
        let media_box = text
            .split("/MediaBox[")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("page has a media box");
        let width: f32 = media_box
            .split_whitespace()
            .nth(2)
            .and_then(|number| number.parse().ok())
            .expect("numeric width");
        assert!(width <= MAX_PDF_SIDE + 1.0 && width > 1_000.0, "{width}");
    }

    #[test]
    fn each_character_gets_a_glyph_spanning_its_bytes() {
        let face = ttf_parser::Face::parse(epaint_default_fonts::UBUNTU_LIGHT, 0).expect("font");
        let width = |line| {
            line_glyphs(&face, line)
                .iter()
                .map(|glyph| glyph.x_advance)
                .sum::<f32>()
        };

        let glyphs = line_glyphs(&face, "Gö");
        assert_eq!(glyphs.len(), 2);
        assert_eq!(glyphs[1].text_range, 1..3, "two-byte character");
        // About 2.5 ems for five characters, and longer text is wider.
        assert!((1.5..3.5).contains(&width("Goals")), "{}", width("Goals"));
        assert!(width("Goals and more goals") > width("Goals") * 3.0);
    }

    #[test]
    fn export_writes_each_format() {
        let dir = TempDir::new("export");
        let map = sample();

        for name in [
            "map.png", "map.svg", "map.pdf", "map.md", "map.opml", "map.mm",
        ] {
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
