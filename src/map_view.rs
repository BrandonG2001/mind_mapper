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
        for visit in map.preorder() {
            let accent = visit.branch.map_or(text_color, |branch| {
                BRANCH_COLORS[branch % BRANCH_COLORS.len()]
            });
            let center = self.camera.world_to_screen(visit.node.position());

            if let Some(parent) = visit.node.parent().and_then(|id| map.node(id)) {
                let start = self.camera.world_to_screen(parent.position());
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
                self.layout.push(LaidOutNode {
                    id: visit.id,
                    rect,
                    galley,
                    accent,
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
        }
    }

    /// The topmost node drawn at `screen` in the last [`MapView::paint`] call.
    #[must_use]
    pub fn hit_test(&self, screen: Pos2) -> Option<NodeId> {
        // Reverse order: nodes painted last are on top.
        self.layout
            .iter()
            .rev()
            .find(|node| node.rect.contains(screen))
            .map(|node| node.id)
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

/// Estimated world-space box of a node, without laying out its text.
fn estimated_rect(node: &Node) -> Rect {
    // Labels wrap at MAX_LABEL_WIDTH, so counting past one line changes nothing.
    let chars = node
        .text()
        .chars()
        .take(usize::from(MAX_LINE_CHARS))
        .count();
    // `take` caps the count, so this conversion cannot actually fail.
    let chars = u8::try_from(chars).unwrap_or(MAX_LINE_CHARS);
    let text_width = f32::from(chars) * MAX_LABEL_WIDTH / f32::from(MAX_LINE_CHARS);
    let size = Vec2::new(text_width, BASE_FONT_SIZE * 1.3) + NODE_PADDING * 2.0;
    let position = node.position();
    Rect::from_center_size(Pos2::new(position.x, position.y), size)
}

/// World-space rectangle that (approximately) contains every node of `map`.
fn world_bounds(map: &MindMap) -> Rect {
    map.iter().fold(Rect::NOTHING, |bounds, (_, node)| {
        bounds.union(estimated_rect(node))
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
