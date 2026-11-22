//! Automatic, overlap-free layout ("Tidy up").
//!
//! Every visible leaf gets its own row, each parent is centred on the rows of its
//! children, and each child sits a fixed gap beyond its parent's edge. Branches keep the
//! side of the root they are already on, and siblings keep their current top-to-bottom
//! order, so tidying respects how the user arranged things.

use crate::map_view::{estimated_node_size, estimated_size};
use crate::model::{MapError, MindMap, Node, NodeId, Point};

/// Horizontal gap between a node's edge and its children's edges, in world units.
pub(crate) const COLUMN_GAP: f32 = 70.0;
/// Vertical space reserved for each leaf row, in world units.
pub(crate) const ROW_HEIGHT: f32 = 56.0;

/// Builds a new map one node at a time in reading order, then arranges it with [`tidy`].
///
/// Nodes get placeholder positions that only tell the layout which side of the root each
/// top-level branch goes on, and that siblings run top to bottom in the order added. Used
/// for templates and imported files, which have no positions of their own.
#[derive(Debug, Clone)]
pub struct TreeBuilder {
    map: MindMap,
    /// Grows with every node, so later siblings sort below earlier ones.
    order: f32,
}

impl TreeBuilder {
    /// Start a map whose central idea is labelled `root_text`.
    #[must_use]
    pub fn new(root_text: &str) -> Self {
        Self {
            map: MindMap::new(root_text),
            order: 0.0,
        }
    }

    /// The central idea.
    #[must_use]
    pub fn root(&self) -> NodeId {
        self.map.root()
    }

    /// The map built so far, e.g. to give a new node notes or a colour.
    pub const fn map_mut(&mut self) -> &mut MindMap {
        &mut self.map
    }

    /// Add a node labelled `text` as the last child of `parent`.
    ///
    /// # Arguments
    ///
    /// * `parent` - A node already in this map
    /// * `text` - Label of the new node
    /// * `left` - For a branch of the root: put it on the left. Ignored further out, where
    ///   branches keep growing away from their parent.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `parent` is not in the map.
    pub fn add(&mut self, parent: NodeId, text: &str, left: bool) -> Result<NodeId, MapError> {
        self.order += 1.0;
        let side = if left { -1.0 } else { 1.0 };
        self.map
            .add_child_at(parent, text, Point::new(side, self.order))
    }

    /// Lay the map out without overlaps and hand it over.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` only if the map is internally inconsistent.
    pub fn finish(mut self) -> Result<MindMap, MapError> {
        tidy(&mut self.map)?;
        Ok(self.map)
    }
}

/// Whether branch `index` of the root's `count` branches goes on the left when nothing
/// says otherwise: the first half (rounded up) go right, the rest left.
#[must_use]
pub const fn balanced_left(index: usize, count: usize) -> bool {
    index >= count.div_ceil(2)
}

/// Re-arrange every node of `map` so nothing overlaps. The root stays where it is.
///
/// A collapsed node takes a single row; its hidden descendants are still arranged
/// (centred on it), so expanding it later shows a tidy branch.
///
/// Runs in O(n log n) for n nodes (the log comes from ordering siblings).
///
/// # Errors
///
/// Returns `MapError::NodeNotFound` only if the map is internally inconsistent.
pub fn tidy(map: &mut MindMap) -> Result<(), MapError> {
    arrange(map, map.root())
}

/// Like [`tidy`], but only re-arranges the descendants of `branch`; the branch's own node
/// and everything outside it stay where they are. Used to lay out pasted branches.
///
/// # Errors
///
/// Returns `MapError::NodeNotFound` if `branch` does not exist.
pub fn tidy_branch(map: &mut MindMap, branch: NodeId) -> Result<(), MapError> {
    arrange(map, branch)
}

/// Lay out everything below `start` (both sides of the map when `start` is the root).
fn arrange(map: &mut MindMap, start: NodeId) -> Result<(), MapError> {
    let start_node = map.node(start).ok_or(MapError::NodeNotFound(start))?;
    let order: Vec<NodeId> = map.subtree_preorder(start).map(|visit| visit.id).collect();
    let mut tidier = Tidier::new(map.slot_count());
    tidier.count_rows(map, &order)?;
    // A branch keeps growing away from its parent.
    if let Some(parent) = start_node.parent().and_then(|id| map.node(id)) {
        tidier.direction[start.index()] = if start_node.position().x < parent.position().x {
            -1.0
        } else {
            1.0
        };
    }

    let root = map.root();
    let root_x = map
        .node(root)
        .ok_or(MapError::NodeNotFound(root))?
        .position()
        .x;
    let mut children = Vec::new();
    for &id in &order {
        children.clear();
        children.extend_from_slice(map.node(id).ok_or(MapError::NodeNotFound(id))?.children());
        if id == root {
            // The root's branches fan out to both sides; keep each on its current side.
            let (left, right): (Vec<NodeId>, Vec<NodeId>) = children
                .iter()
                .partition(|&&child| map.node(child).is_some_and(|c| c.position().x < root_x));
            tidier.place_column(map, id, &right, 1.0)?;
            tidier.place_column(map, id, &left, -1.0)?;
        } else {
            let dir = tidier.direction[id.index()];
            tidier.place_column(map, id, &children, dir)?;
        }
    }
    Ok(())
}

/// Working tables for one [`tidy`] pass, indexed by [`NodeId::index`].
struct Tidier {
    /// Rows a node occupies in its parent's column: 1 when collapsed or a leaf, else the
    /// sum over its children. Counted in f32 to avoid integer-to-float casts.
    rows: Vec<f32>,
    /// +1 for branches growing right, -1 for left.
    direction: Vec<f32>,
    /// Scratch space for sorting one column, reused to avoid an allocation per node.
    column: Vec<(f32, NodeId)>,
}

impl Tidier {
    fn new(slots: usize) -> Self {
        Self {
            rows: vec![1.0; slots],
            direction: vec![1.0; slots],
            column: Vec::new(),
        }
    }

    /// Fill `rows`, given `order` in pre-order.
    fn count_rows(&mut self, map: &MindMap, order: &[NodeId]) -> Result<(), MapError> {
        // Reverse pre-order visits every child before its parent (a post-order).
        for &id in order.iter().rev() {
            let node = map.node(id).ok_or(MapError::NodeNotFound(id))?;
            let own = own_rows(node);
            self.rows[id.index()] = if node.is_collapsed() || node.children().is_empty() {
                own
            } else {
                own.max(self.rows_of(node.children()))
            };
        }
        Ok(())
    }

    fn rows_of(&self, nodes: &[NodeId]) -> f32 {
        nodes.iter().map(|node| self.rows[node.index()]).sum()
    }

    /// Stack `children` in one column on the `dir` side of `parent`, centred on it
    /// vertically, keeping their current top-to-bottom order.
    fn place_column(
        &mut self,
        map: &mut MindMap,
        parent: NodeId,
        children: &[NodeId],
        dir: f32,
    ) -> Result<(), MapError> {
        if children.is_empty() {
            return Ok(());
        }
        let parent_node = map.node(parent).ok_or(MapError::NodeNotFound(parent))?;
        let parent_pos = parent_node.position();
        let parent_half_width = estimated_node_size(parent_node).x * 0.5;

        self.column.clear();
        for &child in children {
            let y = map
                .node(child)
                .ok_or(MapError::NodeNotFound(child))?
                .position()
                .y;
            self.column.push((y, child));
        }
        // `total_cmp` gives floats a total order, so sorting cannot panic on NaN.
        self.column.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut top = parent_pos.y - self.rows_of(children) * ROW_HEIGHT * 0.5;
        for &(_, child) in &self.column {
            self.direction[child.index()] = dir;
            let height = self.rows[child.index()] * ROW_HEIGHT;
            let child_node = map.node(child).ok_or(MapError::NodeNotFound(child))?;
            let half_width = estimated_node_size(child_node).x * 0.5;
            let x = parent_pos.x + dir * (parent_half_width + COLUMN_GAP + half_width);
            map.set_position(child, Point::new(x, top + height * 0.5))?;
            top += height;
        }
        Ok(())
    }
}

/// Rows a node needs for its own box: one for a single-line label, more for a label
/// that wraps, keeping the same spacing between boxes either way.
fn own_rows(node: &Node) -> f32 {
    let spacing = ROW_HEIGHT - estimated_size("").y;
    // The small epsilon stops float rounding from turning exactly 1.0 into 2 rows.
    ((estimated_node_size(node).y + spacing) / ROW_HEIGHT - 1e-3)
        .ceil()
        .max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_view::estimated_rect;

    /// Fail if any two visible nodes' boxes overlap.
    fn assert_no_overlaps(map: &MindMap) {
        let rects: Vec<_> = map
            .visible_preorder()
            .map(|visit| (visit.node.text().to_owned(), estimated_rect(visit.node)))
            .collect();
        for (index, (name, rect)) in rects.iter().enumerate() {
            for (other_name, other) in &rects[index + 1..] {
                assert!(
                    !rect.shrink(0.5).intersects(other.shrink(0.5)),
                    "{name} overlaps {other_name}"
                );
            }
        }
    }

    #[test]
    fn built_trees_keep_their_order_and_sides() {
        let mut builder = TreeBuilder::new("root");
        let root = builder.root();
        let right = builder.add(root, "right", false).expect("root");
        let first = builder.add(right, "first", true).expect("right");
        let second = builder.add(right, "second", true).expect("right");
        let left = builder.add(root, "left", true).expect("root");

        let map = builder.finish().expect("consistent");

        let position = |id| map.node(id).expect("live").position();
        assert!(position(right).x > 0.0 && position(left).x < 0.0);
        // Deeper nodes follow their branch, whatever `left` said.
        assert!(position(first).x > position(right).x);
        assert!(position(first).y < position(second).y);
        assert_no_overlaps(&map);
    }

    #[test]
    fn balanced_sides_put_the_extra_branch_on_the_right() {
        let sides: Vec<bool> = (0..5).map(|index| balanced_left(index, 5)).collect();
        assert_eq!(sides, [false, false, false, true, true]);
    }

    /// A map where every node was dropped on top of the root.
    fn piled_up_map() -> (MindMap, NodeId, NodeId) {
        let mut map = MindMap::new("root");
        let origin = Point::default();
        let a = map.add_child_at(map.root(), "alpha", origin).expect("root");
        let b = map
            .add_child_at(map.root(), "beta with a longer label", origin)
            .expect("root");
        for label in ["one", "two", "three"] {
            map.add_child_at(a, label, origin).expect("a");
            map.add_child_at(b, label, origin).expect("b");
        }
        (map, a, b)
    }

    #[test]
    fn tidy_removes_all_overlaps() {
        let (mut map, _, _) = piled_up_map();

        tidy(&mut map).expect("consistent map");

        assert_no_overlaps(&map);
    }

    #[test]
    fn multi_line_labels_get_extra_rows() {
        let mut map = MindMap::new("root");
        let origin = Point::default();
        let long = "a long label that wraps onto several lines in the canvas ".repeat(2);
        let long = map.add_child_at(map.root(), &long, origin).expect("root");
        let short = map
            .add_child_at(map.root(), "short", Point::new(0.0, 1.0))
            .expect("root");
        let pictured = map
            .add_child_at(map.root(), "with a picture", Point::new(0.0, 2.0))
            .expect("root");
        map.set_picture(pictured, Some(crate::picture::test_picture(160, 120)))
            .expect("exists");

        tidy(&mut map).expect("consistent map");

        let rows = |id| own_rows(map.node(id).expect("live"));
        assert!(rows(long) > 1.0);
        assert!((rows(short) - 1.0).abs() < f32::EPSILON);
        assert!(rows(pictured) > 2.0, "a picture takes the room it needs");
        assert_no_overlaps(&map);
    }

    #[test]
    fn tidy_branch_only_moves_that_branch() {
        let (mut map, a, b) = piled_up_map();
        map.set_position(a, Point::new(-300.0, 0.0)).expect("a");
        let b_before = map.node(b).expect("b").position();
        let a_before = map.node(a).expect("a").position();

        tidy_branch(&mut map, a).expect("a exists");

        assert_eq!(map.node(b).expect("b").position(), b_before);
        assert_eq!(map.node(a).expect("a").position(), a_before);
        // `a` is on the left of the root, so its children grow further left.
        for &child in map.node(a).expect("a").children() {
            assert!(map.node(child).expect("child").position().x < a_before.x);
        }
    }

    #[test]
    fn root_stays_put_and_branches_keep_their_side() {
        let mut map = MindMap::new("root");
        map.set_position(map.root(), Point::new(10.0, 20.0))
            .expect("root");
        let left = map
            .add_child_at(map.root(), "left", Point::new(-50.0, 0.0))
            .expect("root");
        let right = map
            .add_child_at(map.root(), "right", Point::new(80.0, 0.0))
            .expect("root");

        tidy(&mut map).expect("consistent map");

        assert_eq!(
            map.node(map.root()).expect("root").position(),
            Point::new(10.0, 20.0)
        );
        assert!(map.node(left).expect("left").position().x < 10.0);
        assert!(map.node(right).expect("right").position().x > 10.0);
    }

    #[test]
    fn siblings_keep_their_vertical_order() {
        let mut map = MindMap::new("root");
        let lower = map
            .add_child_at(map.root(), "lower", Point::new(100.0, 50.0))
            .expect("root");
        let upper = map
            .add_child_at(map.root(), "upper", Point::new(100.0, -50.0))
            .expect("root");

        tidy(&mut map).expect("consistent map");

        let y = |id| map.node(id).expect("node").position().y;
        assert!(y(upper) < y(lower));
        assert_eq!(y(lower) - y(upper), ROW_HEIGHT);
    }

    #[test]
    fn collapsed_branch_takes_one_row() {
        let (mut map, a, b) = piled_up_map();
        map.set_position(b, Point::new(0.0, 1.0)).expect("b");
        map.set_collapsed(a, true).expect("a");

        tidy(&mut map).expect("consistent map");

        let y = |id| map.node(id).expect("node").position().y;
        // `a` (collapsed) needs one row and `b` three, so their centres are two rows apart.
        assert_eq!(y(b) - y(a), 2.0 * ROW_HEIGHT);
        assert_no_overlaps(&map);
    }
}
