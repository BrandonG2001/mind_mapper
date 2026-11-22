//! Drawing a [`MindMap`] through a [`Camera`]. Shared by the editor canvas and the
//! template thumbnails on the start screen.

use std::sync::Arc;

use eframe::egui::{
    Color32, FontId, Galley, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, Visuals,
    epaint::CubicBezierShape,
};

use crate::camera::Camera;
use crate::model::{MindMap, Node, NodeId};

/// Font size of node labels at 100% zoom.
pub const BASE_FONT_SIZE: f32 = 16.0;
/// Space between a node's label and its border at 100% zoom.
const NODE_PADDING: Vec2 = Vec2::new(14.0, 8.0);
/// Labels wider than this (at 100% zoom) wrap onto several lines.
const MAX_LABEL_WIDTH: f32 = 240.0;
/// Roughly how many characters fill one wrapped line; used to estimate node sizes
/// before their text is laid out.
const MAX_LINE_CHARS: u8 = 28;
/// Radius of the collapse/expand handle at 100% zoom.
const HANDLE_RADIUS: f32 = 8.0;
/// Rounded-corner radius of node boxes at 100% zoom.
const CORNER_RADIUS: f32 = 8.0;
/// Accent colours cycled through for the root's branches.
const BRANCH_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x4e, 0x79, 0xa7),
    Color32::from_rgb(0xf2, 0x8e, 0x2b),
    Color32::from_rgb(0x59, 0xa1, 0x4f),
    Color32::from_rgb(0xe1, 0x57, 0x59),
    Color32::from_rgb(0xb0, 0x7a, 0xa1),
    Color32::from_rgb(0x76, 0xb7, 0xb2),
];

/// A node's on-screen geometry for the current frame.
#[derive(Debug, Clone)]
struct LaidOutNode {
    id: NodeId,
    rect: Rect,
    galley: Arc<Galley>,
    accent: Color32,
    /// Centre of the collapse/expand handle, if one is drawn for this node.
    handle: Option<Pos2>,
    collapsed: bool,
}

/// A camera plus the screen layout of the nodes it drew last.
#[derive(Debug, Default)]
pub struct MapView {
    camera: Camera,
    /// Reused every frame to avoid reallocating.
    layout: Vec<LaidOutNode>,
}

impl MapView {
    /// The view's pan/zoom transform.
    #[must_use]
    pub const fn camera(&self) -> &Camera {
        &self.camera
    }

    /// Mutable access to the pan/zoom transform.
    pub const fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Zoom and pan so the whole map fits inside `screen` (never above 100%).
    pub fn fit(&mut self, map: &MindMap, screen: Rect) {
        self.camera.fit(world_bounds(map), screen);
    }

    /// Lay out the visible nodes of `map` and paint its edges and nodes.
    ///
    /// Nodes and edges outside `painter`'s clip rectangle are skipped, so the cost is
    /// proportional to what is on screen.
    ///
    /// # Arguments
    ///
    /// * `painter` - Where to draw; its clip rectangle limits what is laid out
    /// * `visuals` - Current theme colours
    /// * `map` - The map to draw
    /// * `selected` - Node to highlight, if any
    pub fn paint(
        &mut self,
        painter: &Painter,
        visuals: &Visuals,
        map: &MindMap,
        selected: Option<NodeId>,
    ) {
        let zoom = self.camera.zoom();
        let font = FontId::proportional(BASE_FONT_SIZE * zoom);
        let padding = NODE_PADDING * zoom;
        let text_color = visuals.strong_text_color();
        let edge_width = (2.0 * zoom).max(1.0);
        let clip = painter.clip_rect();
        // Nodes are positioned by their centre, so widen the clip test by the largest
        // likely half-size to keep partly visible nodes.
        let cull_rect = clip.expand(MAX_LABEL_WIDTH * zoom);

        self.layout.clear();
        let handle_radius = HANDLE_RADIUS * zoom;
        for visit in map.visible_preorder() {
            let accent = branch_accent(visit.branch, text_color);
            let center = self.camera.world_to_screen(visit.node.position());
            // Which way the branch grows: away from the parent (the root has no handle).
            let mut outward = None;

            if let Some(parent) = visit.node.parent().and_then(|id| map.node(id)) {
                let start = self.camera.world_to_screen(parent.position());
                outward = Some(if center.x >= start.x { 1.0 } else { -1.0 });
                // The curve never leaves the box spanned by its end points.
                if clip.intersects(Rect::from_two_pos(start, center)) {
                    let bend = Vec2::new((center.x - start.x) * 0.5, 0.0);
                    painter.add(CubicBezierShape::from_points_stroke(
                        [start, start + bend, center - bend, center],
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(edge_width, accent),
                    ));
                }
            }

            if cull_rect.contains(center) {
                let galley = painter.layout(
                    visit.node.text().to_owned(),
                    font.clone(),
                    text_color,
                    MAX_LABEL_WIDTH * zoom,
                );
                let rect = Rect::from_center_size(center, galley.size() + padding * 2.0);
                let collapsed = visit.node.is_collapsed();
                // Collapsed nodes always show "+"; the selected node shows "-" so it can
                // be folded with the mouse.
                let wants_handle =
                    !visit.node.children().is_empty() && (collapsed || selected == Some(visit.id));
                let handle = outward.filter(|_| wants_handle).map(|dir| {
                    center + Vec2::new(dir * (rect.width() * 0.5 + handle_radius * 1.4), 0.0)
                });
                self.layout.push(LaidOutNode {
                    id: visit.id,
                    rect,
                    galley,
                    accent,
                    handle,
                    collapsed,
                });
            }
        }

        let radius = CORNER_RADIUS * zoom;
        let fill = visuals.window_fill();
        let selection = visuals.selection.stroke.color;
        for node in &self.layout {
            let (border, width) = if selected == Some(node.id) {
                (selection, edge_width * 2.0)
            } else {
                (node.accent, edge_width)
            };
            painter.rect_filled(node.rect, radius, fill);
            painter.rect_stroke(
                node.rect,
                radius,
                Stroke::new(width, border),
                StrokeKind::Inside,
            );
            painter.galley(
                node.rect.center() - node.galley.size() * 0.5,
                Arc::clone(&node.galley),
                text_color,
            );
            if let Some(handle) = node.handle {
                paint_handle(painter, node, handle, handle_radius, fill);
            }
        }
    }

    /// The node whose collapse/expand handle is at `screen`, from the last paint call.
    #[must_use]
    pub fn hit_test_handle(&self, screen: Pos2) -> Option<NodeId> {
        // Slightly larger than drawn, so the small circle is easy to click.
        let reach = HANDLE_RADIUS * self.camera.zoom() * 1.5;
        self.layout
            .iter()
            .rev()
            .find(|node| node.handle.is_some_and(|h| h.distance(screen) <= reach))
            .map(|node| node.id)
    }

    /// The topmost node drawn at `screen` in the last [`MapView::paint`] call.
    #[must_use]
    pub fn hit_test(&self, screen: Pos2) -> Option<NodeId> {
        self.hit_test_where(screen, |_| true)
    }

    /// Like [`MapView::hit_test`], but only considers nodes for which `accept` is true.
    ///
    /// # Arguments
    ///
    /// * `screen` - Point to test, in screen pixels
    /// * `accept` - Filter, e.g. to ignore the node currently being dragged
    #[must_use]
    pub fn hit_test_where(&self, screen: Pos2, accept: impl Fn(NodeId) -> bool) -> Option<NodeId> {
        // Reverse order: nodes painted last are on top.
        self.layout
            .iter()
            .rev()
            .find(|node| node.rect.contains(screen) && accept(node.id))
            .map(|node| node.id)
    }

    /// Draw a highlight ring around `id` if it was drawn in the last paint call.
    ///
    /// # Arguments
    ///
    /// * `painter` - Where to draw
    /// * `id` - Node to outline
    /// * `color` - Ring colour
    pub fn outline(&self, painter: &Painter, id: NodeId, color: Color32) {
        if let Some(rect) = self.node_rect(id) {
            let zoom = self.camera.zoom();
            painter.rect_stroke(
                rect.expand(3.0 * zoom),
                CORNER_RADIUS * zoom * 1.4,
                Stroke::new((3.0 * zoom).max(1.5), color),
                StrokeKind::Outside,
            );
        }
    }

    /// Where `id` was drawn in the last [`MapView::paint`] call, or `None` if it was not
    /// visible.
    #[must_use]
    pub fn node_rect(&self, id: NodeId) -> Option<Rect> {
        self.layout
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.rect)
    }
}

/// Draw a collapse/expand handle: a filled "+" disc when collapsed, an outlined "-"
/// disc when expanded.
fn paint_handle(
    painter: &Painter,
    node: &LaidOutNode,
    center: Pos2,
    radius: f32,
    background: Color32,
) {
    let (accent, collapsed) = (node.accent, node.collapsed);
    let (disc, mark) = if collapsed {
        (accent, background)
    } else {
        (background, accent)
    };
    let line = Stroke::new((radius * 0.25).max(1.0), mark);
    painter.circle(center, radius, disc, Stroke::new(line.width, accent));
    let arm = radius * 0.5;
    painter.line_segment([center - Vec2::X * arm, center + Vec2::X * arm], line);
    if collapsed {
        painter.line_segment([center - Vec2::Y * arm, center + Vec2::Y * arm], line);
    }
}

/// Colour of a branch's edges and borders: `root_color` for the root itself, otherwise
/// one of the branch colours, cycling by the index of the root's child it descends from.
pub(crate) fn branch_accent(branch: Option<usize>, root_color: Color32) -> Color32 {
    branch.map_or(root_color, |branch| {
        BRANCH_COLORS[branch % BRANCH_COLORS.len()]
    })
}

/// Height of one line of label text at 100% zoom.
pub(crate) const LINE_HEIGHT: f32 = BASE_FONT_SIZE * 1.3;

/// Split a label into lines of at most `MAX_LINE_CHARS` characters, breaking at spaces
/// where possible (a single word longer than a line is split). Approximates how the
/// canvas wraps labels, without needing the font system.
pub(crate) fn wrap_label(text: &str) -> impl Iterator<Item = &str> {
    let max = usize::from(MAX_LINE_CHARS);
    let mut rest = text.trim();
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        // Byte offset just past `max` characters (labels may contain multi-byte text).
        let limit = rest
            .char_indices()
            .nth(max)
            .map_or(rest.len(), |(at, _)| at);
        let cut = if limit == rest.len() || rest[limit..].starts_with(' ') {
            limit
        } else {
            rest[..limit]
                .rfind(' ')
                .filter(|&at| at > 0)
                .unwrap_or(limit)
        };
        let (line, tail) = rest.split_at(cut);
        rest = tail.trim_start();
        Some(line.trim_end())
    })
}

/// Estimated world-space size of a node box with label `text`, without laying out the
/// text (which needs the font system). Used for fitting the view, auto-layout and export.
pub(crate) fn estimated_size(text: &str) -> Vec2 {
    let (lines, widest) = wrap_label(text).fold((0_u16, 0_u8), |(lines, widest), line| {
        // Wrapped lines hold at most MAX_LINE_CHARS characters, so this fits in a u8.
        let chars = u8::try_from(line.chars().count()).unwrap_or(MAX_LINE_CHARS);
        (lines.saturating_add(1), widest.max(chars))
    });
    let text_width = f32::from(widest) * MAX_LABEL_WIDTH / f32::from(MAX_LINE_CHARS);
    let text_height = f32::from(lines.max(1)) * LINE_HEIGHT;
    Vec2::new(text_width, text_height) + NODE_PADDING * 2.0
}

/// Estimated world-space box of a node.
pub(crate) fn estimated_rect(node: &Node) -> Rect {
    let position = node.position();
    Rect::from_center_size(
        Pos2::new(position.x, position.y),
        estimated_size(node.text()),
    )
}

/// World-space rectangle that (approximately) contains every visible node of `map`.
fn world_bounds(map: &MindMap) -> Rect {
    map.visible_preorder().fold(Rect::NOTHING, |bounds, visit| {
        bounds.union(estimated_rect(visit.node))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;
    use eframe::egui;

    /// Run one headless egui frame of `size` pixels and hand `draw` its painter.
    fn headless_frame(size: Vec2, draw: impl FnOnce(&Painter, &Visuals)) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        // `Option::take` lets the FnMut closure that egui expects call `draw` once.
        let mut draw = Some(draw);
        let mut output = ctx.run_ui(input, |ui| {
            if let Some(draw) = draw.take() {
                draw(ui.painter(), ui.visuals());
            }
        });
        // Nothing renders the frame, so discard its texture uploads explicitly.
        output.textures_delta.clear();
    }

    #[test]
    fn paint_lays_out_only_visible_nodes() {
        let mut map = MindMap::new("root");
        let far = map
            .add_child_at(map.root(), "far", Point::new(100_000.0, 0.0))
            .expect("root exists");
        let mut view = MapView::default();
        view.camera_mut().pan(Vec2::new(200.0, 200.0));

        headless_frame(Vec2::splat(400.0), |painter, visuals| {
            view.paint(painter, visuals, &map, None);
        });

        assert!(view.node_rect(map.root()).is_some());
        assert!(view.node_rect(far).is_none());
        let center = view.node_rect(map.root()).expect("root drawn").center();
        assert_eq!(view.hit_test(center), Some(map.root()));
        assert_eq!(view.hit_test(Pos2::new(399.0, 399.0)), None);
    }

    #[test]
    fn hit_test_prefers_topmost_node() {
        let mut map = MindMap::new("root");
        let child = map
            .add_child_at(map.root(), "child", Point::new(4.0, 4.0))
            .expect("root exists");
        let mut view = MapView::default();
        view.camera_mut().pan(Vec2::new(200.0, 200.0));

        headless_frame(Vec2::splat(400.0), |painter, visuals| {
            view.paint(painter, visuals, &map, None);
        });

        // The child is drawn after (on top of) the root where they overlap.
        assert_eq!(view.hit_test(Pos2::new(205.0, 205.0)), Some(child));
        // Filtering the child out exposes the root underneath.
        assert_eq!(
            view.hit_test_where(Pos2::new(205.0, 205.0), |id| id != child),
            Some(map.root())
        );
    }

    #[test]
    fn collapsed_children_are_hidden_and_handle_is_clickable() {
        let mut map = MindMap::new("root");
        let branch = map
            .add_child_at(map.root(), "branch", Point::new(150.0, 0.0))
            .expect("root exists");
        let leaf = map
            .add_child_at(branch, "leaf", Point::new(300.0, 0.0))
            .expect("branch exists");
        map.set_collapsed(branch, true).expect("branch exists");
        let mut view = MapView::default();
        view.camera_mut().pan(Vec2::new(50.0, 200.0));

        headless_frame(Vec2::splat(600.0), |painter, visuals| {
            view.paint(painter, visuals, &map, None);
        });

        assert!(view.node_rect(leaf).is_none());
        let rect = view.node_rect(branch).expect("branch drawn");
        // The handle sits just outside the node's outer (right-hand) edge.
        let handle = Pos2::new(rect.max.x + HANDLE_RADIUS * 1.4, rect.center().y);
        assert_eq!(view.hit_test_handle(handle), Some(branch));
        assert_eq!(view.hit_test_handle(rect.center()), None);
    }

    #[test]
    fn wrap_label_breaks_at_spaces() {
        let text = "a fairly long label that certainly needs more than one line";

        let lines: Vec<&str> = wrap_label(text).collect();

        assert!(lines.len() > 1);
        assert!(
            lines
                .iter()
                .all(|line| line.chars().count() <= usize::from(MAX_LINE_CHARS))
        );
        assert_eq!(lines.join(" "), text);
    }

    #[test]
    fn wrap_label_splits_long_words_and_handles_multibyte_text() {
        let word = "\u{e9}".repeat(usize::from(MAX_LINE_CHARS) + 5);

        let lines: Vec<&str> = wrap_label(&word).collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines.concat(), word);
        assert_eq!(wrap_label("   ").count(), 0);
    }

    #[test]
    fn longer_labels_are_estimated_wider_then_taller() {
        let short = estimated_size("hi");
        let long = estimated_size(&"word ".repeat(20));

        assert!(long.x > short.x);
        assert!(long.y > short.y);
    }

    #[test]
    fn world_bounds_contains_every_node() {
        let mut map = MindMap::new("root");
        map.add_child_at(map.root(), "left", Point::new(-500.0, 30.0))
            .expect("root exists");
        map.add_child_at(map.root(), "a much longer label", Point::new(400.0, -80.0))
            .expect("root exists");

        let bounds = world_bounds(&map);

        for (_, node) in map.iter() {
            assert!(bounds.contains_rect(estimated_rect(node)));
        }
    }

    #[test]
    fn fit_shows_whole_map() {
        let mut map = MindMap::new("root");
        map.add_child_at(map.root(), "far", Point::new(2_000.0, 0.0))
            .expect("root exists");
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
        let mut view = MapView::default();

        view.fit(&map, screen);

        for (_, node) in map.iter() {
            let on_screen = view.camera().world_to_screen(node.position());
            assert!(
                screen.contains(on_screen),
                "{on_screen:?} outside {screen:?}"
            );
        }
    }
}
