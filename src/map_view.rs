//! Drawing a [`MindMap`] through a [`Camera`]. Shared by the editor canvas and the
//! template thumbnails on the start screen.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{
    Align, Color32, Context, FontId, Galley, Painter, Pos2, Rect, Shape, Stroke, StrokeKind,
    TextureHandle, TextureId, TextureOptions, Vec2, Visuals,
    epaint::CubicBezierShape,
    text::{LayoutJob, TextFormat},
};

use crate::camera::Camera;
use crate::fonts;
use crate::model::{MindMap, Node, NodeColor, NodeId, NodeStyle, Point, Visit};
use crate::picture::Picture;

/// Space between a node's picture and its label, at 100% zoom.
const PICTURE_GAP: f32 = 6.0;
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
/// Width of cross-link lines at 100% zoom.
pub(crate) const LINK_WIDTH: f32 = 1.5;
/// Dash and gap lengths of cross-link lines at 100% zoom.
pub(crate) const LINK_DASH: [f32; 2] = [6.0, 4.0];
/// Opacity of cross-links, so they read as secondary to the tree.
pub(crate) const LINK_OPACITY: f32 = 0.6;
/// Length of a cross-link's arrowhead at 100% zoom.
pub(crate) const ARROW_SIZE: f32 = 9.0;
/// Cross-link labels' font size relative to node labels'.
pub(crate) const LINK_LABEL_SCALE: f32 = 0.85;
/// Space between a cross-link label and the edge of its backing, at 100% zoom.
pub(crate) const LINK_LABEL_PADDING: Vec2 = Vec2::new(6.0, 3.0);
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
    /// The node has notes or a link, shown by a folded corner.
    has_details: bool,
    /// The node's picture on screen: its texture and where it goes.
    picture: Option<(TextureId, Rect)>,
}

/// Screen copies of the pictures on the map, by [`Picture::key`], with the frame each was
/// last needed in. `None` marks a picture that could not be decoded, so it is not retried.
#[derive(Default)]
struct PictureTextures {
    textures: HashMap<u64, (Option<TextureHandle>, u64)>,
    frame: u64,
}

// Written by hand: egui's texture handles cannot be printed.
impl std::fmt::Debug for PictureTextures {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PictureTextures")
            .field("pictures", &self.textures.len())
            .field("frame", &self.frame)
            .finish()
    }
}

impl PictureTextures {
    /// Start a frame.
    fn begin(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// The texture for `picture`, decoding it the first time it is needed.
    fn texture(&mut self, ctx: &Context, picture: &Picture) -> Option<TextureId> {
        let frame = self.frame;
        let (texture, used) = self.textures.entry(picture.key()).or_insert_with(|| {
            let texture = picture.decode_for_screen().ok().map(|image| {
                ctx.load_texture(
                    format!("picture-{:x}", picture.key()),
                    image,
                    TextureOptions::LINEAR,
                )
            });
            (texture, frame)
        });
        *used = frame;
        texture.as_ref().map(TextureHandle::id)
    }

    /// Keep the texture of `picture` (on the map but off screen) for later frames.
    fn keep(&mut self, picture: &Picture) {
        if let Some((_, used)) = self.textures.get_mut(&picture.key()) {
            *used = self.frame;
        }
    }

    /// Free the textures of pictures no longer on the map (deleted, folded away).
    fn end(&mut self) {
        let frame = self.frame;
        self.textures.retain(|_, (_, used)| *used == frame);
    }
}

/// A camera plus the screen layout of the nodes it drew last.
#[derive(Debug, Default)]
pub struct MapView {
    camera: Camera,
    /// Reused every frame to avoid reallocating.
    layout: Vec<LaidOutNode>,
    /// Reused every frame to avoid reallocating.
    accents: Accents,
    /// The branch shown on its own in focus mode; `None` shows the whole map.
    focus: Option<NodeId>,
    /// Reused every frame: `shown[i]` is whether the node in slot `i` is on the map this
    /// frame (not folded away or outside the focused branch), drawn or scrolled off.
    shown: Vec<bool>,
    /// Screen copies of the pictures on the map.
    pictures: PictureTextures,
}

impl MapView {
    /// The branch shown on its own, if focus mode is on.
    #[must_use]
    pub const fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    /// Show only the branch of `id` (focus mode), or the whole map with `None`.
    pub const fn set_focus(&mut self, id: Option<NodeId>) {
        self.focus = id;
    }

    /// Where drawing starts: the focused branch if it still exists, else the root.
    fn start(&self, map: &MindMap) -> NodeId {
        self.focus
            .filter(|&id| map.node(id).is_some())
            .unwrap_or_else(|| map.root())
    }

    /// The view's pan/zoom transform.
    #[must_use]
    pub const fn camera(&self) -> &Camera {
        &self.camera
    }

    /// Mutable access to the pan/zoom transform.
    pub const fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Zoom and pan so the whole map (or the focused branch) fits inside `screen`, never
    /// above 100%.
    pub fn fit(&mut self, map: &MindMap, screen: Rect) {
        self.camera.fit(world_bounds(map, self.start(map)), screen);
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
    /// * `selected` - Whether a node is selected (highlighted, with a fold handle)
    pub fn paint(
        &mut self,
        painter: &Painter,
        visuals: &Visuals,
        map: &MindMap,
        selected: impl Fn(NodeId) -> bool,
    ) {
        let zoom = self.camera.zoom();
        let padding = NODE_PADDING * zoom;
        let text_color = visuals.strong_text_color();
        let edge_width = (2.0 * zoom).max(1.0);
        let clip = painter.clip_rect();
        // Nodes are positioned by their centre, so widen the clip test by the largest
        // likely half-size to keep partly visible nodes.
        let cull_rect = clip.expand(MAX_LABEL_WIDTH * zoom);

        self.layout.clear();
        let start = self.start(map);
        self.accents.reset(map);
        self.accents.prime(map, start, text_color);
        self.shown.clear();
        self.shown.resize(map.slot_count(), false);
        self.pictures.begin();
        let handle_radius = HANDLE_RADIUS * zoom;
        for visit in map.visible_subtree_preorder(start) {
            self.shown[visit.id.index()] = true;
            let accent = self.accents.resolve(map, &visit, text_color);
            let center = self.camera.world_to_screen(visit.node.position());
            // Which way the branch grows: away from the parent (the root has no handle).
            let mut outward = None;

            if let Some(parent) = visit.node.parent().and_then(|id| map.node(id)) {
                let from = self.camera.world_to_screen(parent.position());
                outward = Some(if center.x >= from.x { 1.0 } else { -1.0 });
                // The curve never leaves the box spanned by its end points. The focused
                // branch's own edge is left out, since its parent is not drawn.
                if visit.id != start && clip.intersects(Rect::from_two_pos(from, center)) {
                    let bend = Vec2::new((center.x - from.x) * 0.5, 0.0);
                    painter.add(CubicBezierShape::from_points_stroke(
                        [from, from + bend, center - bend, center],
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(edge_width, accent),
                    ));
                }
            }

            if cull_rect.contains(center) {
                let galley = painter.layout_job(label_job(visit.node, text_color, zoom));
                let picture_size = visit
                    .node
                    .picture()
                    .map_or(Vec2::ZERO, |picture| picture.display_size() * zoom);
                let rect = Rect::from_center_size(
                    center,
                    content_size(galley.size(), picture_size, zoom) + padding * 2.0,
                );
                let picture = visit.node.picture().and_then(|picture| {
                    let texture = self.pictures.texture(painter.ctx(), picture)?;
                    let top = Pos2::new(center.x - picture_size.x * 0.5, rect.min.y + padding.y);
                    Some((texture, Rect::from_min_size(top, picture_size)))
                });
                let collapsed = visit.node.is_collapsed();
                // Collapsed nodes always show "+"; selected nodes show "-" so they can be
                // folded with the mouse.
                let wants_handle =
                    !visit.node.children().is_empty() && (collapsed || selected(visit.id));
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
                    has_details: !visit.node.note().is_empty() || visit.node.link().is_some(),
                    picture,
                });
            } else if let Some(picture) = visit.node.picture() {
                // Off screen but on the map: keep its screen copy for when it scrolls back.
                self.pictures.keep(picture);
            }
        }
        self.pictures.end();

        self.paint_cross_links(painter, map, visuals);

        self.paint_nodes(painter, visuals, &selected);
    }

    /// Draw the node boxes laid out by [`MapView::paint`]: border, picture, label, the
    /// folded corner of nodes with details and the fold handle.
    fn paint_nodes(
        &self,
        painter: &Painter,
        visuals: &Visuals,
        selected: &impl Fn(NodeId) -> bool,
    ) {
        let zoom = self.camera.zoom();
        let padding = NODE_PADDING * zoom;
        let text_color = visuals.strong_text_color();
        let edge_width = (2.0 * zoom).max(1.0);
        let handle_radius = HANDLE_RADIUS * zoom;
        let radius = CORNER_RADIUS * zoom;
        let fill = visuals.window_fill();
        let selection = visuals.selection.stroke.color;
        for node in &self.layout {
            let (border, width) = if selected(node.id) {
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
            if let Some((texture, place)) = node.picture {
                let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture, place, whole, Color32::WHITE);
            }
            // The label sits at the bottom of the box (below any picture). A centred
            // galley's lines are laid out around x = 0, so align its own bounding box (not
            // its origin) with that spot.
            let label_center = Pos2::new(
                node.rect.center().x,
                node.rect.max.y - padding.y - node.galley.size().y * 0.5,
            );
            painter.galley(
                label_center - node.galley.rect.center().to_vec2(),
                Arc::clone(&node.galley),
                text_color,
            );
            if node.has_details {
                // A small folded corner at the top right, like a dog-eared page.
                let corner = node.rect.right_top();
                let size = CORNER_RADIUS * zoom * 1.3;
                painter.add(Shape::convex_polygon(
                    vec![
                        corner + Vec2::new(-size, 0.0),
                        corner,
                        corner + Vec2::new(0.0, size),
                    ],
                    node.accent,
                    Stroke::NONE,
                ));
            }
            if let Some(handle) = node.handle {
                paint_handle(painter, node, handle, handle_radius, fill);
            }
        }
    }

    /// Draw the cross-links whose ends are both on the map, under the node boxes.
    fn paint_cross_links(&self, painter: &Painter, map: &MindMap, visuals: &Visuals) {
        let zoom = self.camera.zoom();
        let text_color = visuals.strong_text_color();
        let stroke = Stroke::new(
            (LINK_WIDTH * zoom).max(1.0),
            text_color.gamma_multiply(LINK_OPACITY),
        );
        let label_font = FontId::proportional(BASE_FONT_SIZE * LINK_LABEL_SCALE * zoom);
        let shown = |id: NodeId| self.shown.get(id.index()).copied().unwrap_or(false);
        for link in map.cross_links() {
            let (Some(from), Some(to)) = (map.node(link.from()), map.node(link.to())) else {
                continue;
            };
            if !shown(link.from()) || !shown(link.to()) {
                continue;
            }
            let curve = cross_link_curve(estimated_rect(from), estimated_rect(to))
                .map(|point| self.camera.world_to_screen(Point::new(point.x, point.y)));
            if !painter.clip_rect().intersects(Rect::from_points(&curve)) {
                continue;
            }
            paint_cross_link(painter, curve, stroke, zoom);
            if !link.label().is_empty() {
                let mut job = LayoutJob::simple(
                    link.label().to_owned(),
                    label_font.clone(),
                    text_color,
                    MAX_LABEL_WIDTH * zoom,
                );
                job.halign = Align::Center;
                let galley = painter.layout_job(job);
                let backing = Rect::from_center_size(
                    curve_midpoint(curve),
                    galley.size() + LINK_LABEL_PADDING * 2.0 * zoom,
                );
                painter.rect(
                    backing,
                    CORNER_RADIUS * zoom,
                    visuals.window_fill(),
                    stroke,
                    StrokeKind::Inside,
                );
                painter.galley(
                    backing.center() - galley.rect.center().to_vec2(),
                    galley,
                    text_color,
                );
            }
        }
    }

    /// Draw the link being made from node `from` (if it was drawn) to the mouse pointer at
    /// `cursor`, in `color`.
    pub fn paint_pending_link(
        &self,
        painter: &Painter,
        from: NodeId,
        cursor: Pos2,
        color: Color32,
    ) {
        let Some(start) = self.node_rect(from) else {
            return;
        };
        let zoom = self.camera.zoom();
        let curve = cross_link_curve(start, Rect::from_center_size(cursor, Vec2::ZERO));
        paint_cross_link(
            painter,
            curve,
            Stroke::new((LINK_WIDTH * zoom).max(1.0), color),
            zoom,
        );
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

    /// The nodes drawn in the last [`MapView::paint`] call that overlap `area`.
    pub fn nodes_in(&self, area: Rect) -> impl Iterator<Item = NodeId> + '_ {
        self.layout
            .iter()
            .filter(move |node| node.rect.intersects(area))
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
        self.outline_where(painter, |candidate| candidate == id, color);
    }

    /// Draw a highlight ring around every drawn node for which `wanted` is true, in one
    /// pass over the layout.
    pub fn outline_where(
        &self,
        painter: &Painter,
        wanted: impl Fn(NodeId) -> bool,
        color: Color32,
    ) {
        let zoom = self.camera.zoom();
        let stroke = Stroke::new((3.0 * zoom).max(1.5), color);
        for node in self.layout.iter().filter(|node| wanted(node.id)) {
            painter.rect_stroke(
                node.rect.expand(3.0 * zoom),
                CORNER_RADIUS * zoom * 1.4,
                stroke,
                StrokeKind::Outside,
            );
        }
    }

    /// Every node drawn in the last [`MapView::paint`] call and where, parents before
    /// children.
    pub fn drawn(&self) -> impl Iterator<Item = (NodeId, Rect)> + '_ {
        self.layout.iter().map(|node| (node.id, node.rect))
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

/// The curve of a cross-link between the boxes `from` and `to` (in any one coordinate
/// space): a cubic from edge to edge that bows to one side, so it stands out from the
/// tree's own lines.
pub(crate) fn cross_link_curve(from: Rect, to: Rect) -> [Pos2; 4] {
    let (start, end) = (from.center(), to.center());
    let span = end - start;
    if span.length() < f32::EPSILON {
        return [start; 4];
    }
    // A quarter of the distance to the side, at a third and two thirds of the way.
    let bow = span.rot90() * 0.25;
    let first = start + span / 3.0 + bow;
    let second = start + span * (2.0 / 3.0) + bow;
    [
        edge_toward(from, first),
        first,
        second,
        edge_toward(to, second),
    ]
}

/// The point halfway along a cubic `curve`, where a cross-link's label goes.
pub(crate) fn curve_midpoint([start, first, second, end]: [Pos2; 4]) -> Pos2 {
    // The cubic Bezier formula at t = 1/2: weights 1, 3, 3, 1 over 8.
    let mix = |a: f32, b: f32, c: f32, d: f32| (a + 3.0 * b + 3.0 * c + d) / 8.0;
    Pos2::new(
        mix(start.x, first.x, second.x, end.x),
        mix(start.y, first.y, second.y, end.y),
    )
}

/// Estimated size of a cross-link label's backing, like [`estimated_size`] for nodes.
pub(crate) fn estimated_link_label_size(label: &str) -> Vec2 {
    (estimated_size(label) - NODE_PADDING * 2.0) * LINK_LABEL_SCALE + LINK_LABEL_PADDING * 2.0
}

/// Where the line from the centre of `rect` towards `target` leaves the rectangle (or
/// `target` itself if it is inside).
fn edge_toward(rect: Rect, target: Pos2) -> Pos2 {
    let center = rect.center();
    let direction = target - center;
    let half = rect.size() * 0.5;
    let reach = |half: f32, along: f32| {
        if along.abs() > f32::EPSILON {
            half / along.abs()
        } else {
            f32::INFINITY
        }
    };
    let scale = reach(half.x, direction.x)
        .min(reach(half.y, direction.y))
        .min(1.0);
    center + direction * scale
}

/// The triangle at the end of a cross-link `curve`: its tip on the curve's end, pointing
/// the way the curve arrives, `size` long.
pub(crate) fn arrow_head(curve: [Pos2; 4], size: f32) -> [Pos2; 3] {
    let tip = curve[3];
    let arriving = [curve[2], curve[1], curve[0]]
        .into_iter()
        .map(|point| tip - point)
        .find(|direction| direction.length() > f32::EPSILON)
        .map_or(Vec2::X, Vec2::normalized);
    let half_width = arriving.rot90() * size * 0.5;
    let base = tip - arriving * size;
    [tip, base + half_width, base - half_width]
}

/// Draw one cross-link: a dashed `curve` (in screen pixels) with an arrowhead.
fn paint_cross_link(painter: &Painter, curve: [Pos2; 4], stroke: Stroke, zoom: f32) {
    let points = CubicBezierShape::from_points_stroke(curve, false, Color32::TRANSPARENT, stroke)
        .flatten(Some(0.5));
    painter.extend(Shape::dashed_line(
        &points,
        stroke,
        LINK_DASH[0] * zoom,
        LINK_DASH[1] * zoom,
    ));
    painter.add(Shape::convex_polygon(
        arrow_head(curve, ARROW_SIZE * zoom).to_vec(),
        stroke.color,
        Stroke::NONE,
    ));
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

/// Works out the accent colour (border and incoming edge) of each node during a
/// pre-order walk: a node's own colour if it has one, otherwise its parent's accent, and
/// for the root's children the automatic branch colour.
#[derive(Debug, Default)]
pub(crate) struct Accents(Vec<Color32>);

impl Accents {
    /// Prepare for a walk over `map`.
    pub(crate) fn reset(&mut self, map: &MindMap) {
        self.0.clear();
        self.0.resize(map.slot_count(), Color32::TRANSPARENT);
    }

    /// Resolve the ancestors of `start`, so a walk that begins at `start` gets the same
    /// colours as a walk over the whole map.
    pub(crate) fn prime(&mut self, map: &MindMap, start: NodeId, root_color: Color32) {
        let branch = map.branch_index(start);
        let ancestors: Vec<NodeId> = map.ancestors(start).collect();
        // Root first, so each parent is resolved before its child.
        for &id in ancestors.iter().rev() {
            if let Some(node) = map.node(id) {
                self.resolve(map, &Visit { id, node, branch }, root_color);
            }
        }
    }

    /// The accent of `visit`. Its parent must have been resolved first, which a pre-order
    /// walk guarantees.
    pub(crate) fn resolve(
        &mut self,
        map: &MindMap,
        visit: &Visit<'_>,
        root_color: Color32,
    ) -> Color32 {
        let inherited = match visit.node.parent() {
            None => root_color,
            Some(parent) if parent == map.root() => {
                let branch = visit.branch.unwrap_or(0);
                BRANCH_COLORS[branch % BRANCH_COLORS.len()]
            }
            Some(parent) => self.0[parent.index()],
        };
        let accent = visit.node.color().map_or(inherited, to_color32);
        self.0[visit.id.index()] = accent;
        accent
    }
}

/// The egui colour for a node colour.
pub(crate) fn to_color32(color: NodeColor) -> Color32 {
    let [r, g, b] = color.components();
    Color32::from_rgb(r, g, b)
}

/// Height of one line of label text at 100% zoom.
pub(crate) const LINE_HEIGHT: f32 = BASE_FONT_SIZE * 1.3;

/// Split a label into display lines: every typed line break starts a new line, and lines
/// longer than `MAX_LINE_CHARS` characters wrap, at spaces where possible (a single word
/// longer than a line is split). Approximates how the canvas wraps labels, without
/// needing the font system. Blank lines are skipped.
pub(crate) fn wrap_label(text: &str) -> impl Iterator<Item = &str> {
    text.lines().flat_map(wrap_line)
}

/// Wrap one typed line (see [`wrap_label`]).
fn wrap_line(line: &str) -> impl Iterator<Item = &str> {
    let max = usize::from(MAX_LINE_CHARS);
    let mut rest = line.trim();
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

/// The estimated world-space size of a label (without padding) of `text` in `style`.
pub(crate) fn estimated_label_size(text: &str, style: NodeStyle) -> Vec2 {
    let plain = estimated_size(text) - NODE_PADDING * 2.0;
    Vec2::new(plain.x * style.width_scale(), plain.y * style.font_scale())
}

/// How to lay out `node`'s label at `zoom`: in its style, wrapped as estimated (a bigger or
/// bold label wraps at a proportionally wider width, so lines hold the same text), each
/// line centred like the editor and exports.
pub(crate) fn label_job(node: &Node, color: Color32, zoom: f32) -> LayoutJob {
    let style = node.style();
    let format = TextFormat {
        font_id: FontId::new(
            BASE_FONT_SIZE * style.font_scale() * zoom,
            fonts::family(style.bold),
        ),
        color,
        italics: style.italic,
        ..TextFormat::default()
    };
    let mut job = LayoutJob::single_section(node.text().to_owned(), format);
    job.wrap.max_width = MAX_LABEL_WIDTH * style.width_scale() * zoom;
    job.halign = Align::Center;
    job
}

/// The size of a node's contents: its label of size `label`, with a picture of size
/// `picture` (zero if none) above it, `zoom` setting the gap between them.
fn content_size(label: Vec2, picture: Vec2, zoom: f32) -> Vec2 {
    if picture == Vec2::ZERO {
        label
    } else {
        Vec2::new(
            label.x.max(picture.x),
            label.y + picture.y + PICTURE_GAP * zoom,
        )
    }
}

/// Estimated world-space size of `node`'s box: its label (see [`estimated_size`]) and
/// any picture above it.
pub(crate) fn estimated_node_size(node: &Node) -> Vec2 {
    let picture = node.picture().map_or(Vec2::ZERO, Picture::display_size);
    content_size(
        estimated_label_size(node.text(), node.style()),
        picture,
        1.0,
    ) + NODE_PADDING * 2.0
}

/// Where a node's picture and label go inside its world-space box `rect`, for exports:
/// the picture's rectangle (if it has one) and the centre of the label.
pub(crate) fn node_contents(node: &Node, rect: Rect) -> (Option<Rect>, Pos2) {
    let label_height = estimated_label_size(node.text(), node.style()).y;
    let label_center = Pos2::new(
        rect.center().x,
        rect.max.y - NODE_PADDING.y - label_height * 0.5,
    );
    let picture = node.picture().map(|picture| {
        let size = picture.display_size();
        Rect::from_min_size(
            Pos2::new(rect.center().x - size.x * 0.5, rect.min.y + NODE_PADDING.y),
            size,
        )
    });
    (picture, label_center)
}

/// Estimated world-space box of a node.
pub(crate) fn estimated_rect(node: &Node) -> Rect {
    let position = node.position();
    Rect::from_center_size(Pos2::new(position.x, position.y), estimated_node_size(node))
}

/// World-space rectangle that (approximately) contains every visible node of `start`'s
/// branch of `map` (the whole map when `start` is the root).
fn world_bounds(map: &MindMap, start: NodeId) -> Rect {
    map.visible_subtree_preorder(start)
        .fold(Rect::NOTHING, |bounds, visit| {
            bounds.union(estimated_rect(visit.node))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
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
            view.paint(painter, visuals, &map, |_| false);
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
            view.paint(painter, visuals, &map, |_| false);
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
    fn focus_draws_only_one_branch_in_its_usual_colours() {
        let mut map = MindMap::new("root");
        let root = map.root();
        let a = map
            .add_child_at(root, "a", Point::new(150.0, 0.0))
            .expect("root");
        let b = map
            .add_child_at(root, "b", Point::new(150.0, 60.0))
            .expect("root");
        let b1 = map
            .add_child_at(b, "b1", Point::new(300.0, 60.0))
            .expect("b");
        let mut view = MapView::default();
        view.camera_mut().pan(Vec2::new(50.0, 100.0));
        let accent_of = |view: &MapView, id| {
            view.layout
                .iter()
                .find(|node| node.id == id)
                .map(|node| node.accent)
        };
        headless_frame(Vec2::splat(500.0), |painter, visuals| {
            view.paint(painter, visuals, &map, |_| false);
        });
        let usual = accent_of(&view, b1);

        view.set_focus(Some(b));
        headless_frame(Vec2::splat(500.0), |painter, visuals| {
            view.paint(painter, visuals, &map, |_| false);
        });

        assert!(view.node_rect(root).is_none() && view.node_rect(a).is_none());
        assert!(view.node_rect(b).is_some());
        assert_eq!(accent_of(&view, b1), usual);
        assert!(usual.is_some());
    }

    #[test]
    fn cross_link_curves_run_edge_to_edge_and_end_in_an_arrow() {
        let from = Rect::from_center_size(Pos2::new(0.0, 0.0), Vec2::new(100.0, 40.0));
        let to = Rect::from_center_size(Pos2::new(300.0, 0.0), Vec2::new(60.0, 40.0));

        let curve = cross_link_curve(from, to);

        // Both ends sit on the boxes' outlines, not inside them.
        let on_outline = |rect: Rect, point: Pos2| {
            rect.expand(0.01).contains(point) && !rect.shrink(0.01).contains(point)
        };
        assert!(on_outline(from, curve[0]), "{:?}", curve[0]);
        assert!(on_outline(to, curve[3]), "{:?}", curve[3]);
        // It bows to one side of the straight line between them.
        assert!(curve[1].y.abs() > 10.0 && (curve[1].y > 0.0) == (curve[2].y > 0.0));
        let head = arrow_head(curve, 9.0);
        assert_eq!(head[0], curve[3]);
        assert!((head[1].distance(head[2]) - 9.0).abs() < 1e-3);
        let middle = curve_midpoint(curve);
        assert!(
            middle.x > 50.0 && middle.x < 270.0 && middle.y.abs() > 10.0,
            "{middle:?}"
        );
        assert!(estimated_link_label_size("depends on").x < estimated_size("depends on").x);
        // Boxes in the same place give a dot, not NaNs.
        assert!(
            cross_link_curve(from, from)
                .iter()
                .all(|point| point.is_finite())
        );
    }

    #[test]
    fn nodes_in_finds_nodes_touching_an_area() {
        let mut map = MindMap::new("root");
        let near = map
            .add_child_at(map.root(), "near", Point::new(150.0, 0.0))
            .expect("root exists");
        map.add_child_at(map.root(), "far", Point::new(150.0, 150.0))
            .expect("root exists");
        let mut view = MapView::default();
        view.camera_mut().pan(Vec2::new(50.0, 100.0));

        headless_frame(Vec2::splat(400.0), |painter, visuals| {
            view.paint(painter, visuals, &map, |_| false);
        });

        // A box from the root's centre to "near"'s centre touches exactly those two.
        let root = view.node_rect(map.root()).expect("drawn").center();
        let near_center = view.node_rect(near).expect("drawn").center();
        let mut found: Vec<NodeId> = view
            .nodes_in(Rect::from_two_pos(root, near_center))
            .collect();
        found.sort();
        assert_eq!(found, vec![map.root(), near]);
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
            view.paint(painter, visuals, &map, |_| false);
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
    fn typed_line_breaks_start_new_lines() {
        let lines: Vec<&str> = wrap_label("first line\nsecond\r\n\nthird").collect();

        assert_eq!(lines, ["first line", "second", "third"]);
        assert!(estimated_size("a\nb").y > estimated_size("a b").y);
    }

    #[test]
    fn longer_labels_are_estimated_wider_then_taller() {
        let short = estimated_size("hi");
        let long = estimated_size(&"word ".repeat(20));

        assert!(long.x > short.x);
        assert!(long.y > short.y);
    }

    #[test]
    fn own_colours_override_and_are_inherited() {
        let mut map = MindMap::new("root");
        let branch = map.add_child(map.root(), "branch").expect("root");
        let child = map.add_child(branch, "child").expect("branch");
        let grandchild = map.add_child(child, "grandchild").expect("child");
        let other = map.add_child(map.root(), "other").expect("root");
        let red = NodeColor::rgb(255, 0, 0);
        let blue = NodeColor::rgb(0, 0, 255);
        map.set_color(child, Some(red)).expect("child");
        map.set_color(grandchild, Some(blue)).expect("grandchild");
        let root_color = Color32::WHITE;

        let mut accents = Accents::default();
        accents.reset(&map);
        let resolved: Vec<(NodeId, Color32)> = map
            .preorder()
            .map(|visit| (visit.id, accents.resolve(&map, &visit, root_color)))
            .collect();

        let accent = |id| resolved.iter().find(|(n, _)| *n == id).expect("visited").1;
        assert_eq!(accent(map.root()), root_color);
        assert_eq!(accent(branch), BRANCH_COLORS[0]);
        assert_eq!(accent(child), to_color32(red));
        assert_eq!(accent(grandchild), to_color32(blue));
        assert_eq!(accent(other), BRANCH_COLORS[1]);
    }

    #[test]
    fn world_bounds_contains_every_node() {
        let mut map = MindMap::new("root");
        map.add_child_at(map.root(), "left", Point::new(-500.0, 30.0))
            .expect("root exists");
        map.add_child_at(map.root(), "a much longer label", Point::new(400.0, -80.0))
            .expect("root exists");

        let bounds = world_bounds(&map, map.root());

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
