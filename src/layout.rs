//! Automatic, overlap-free layout ("Tidy up").
//!
//! Every visible leaf gets its own row, each parent is centred on the rows of its
//! children, and each child sits a fixed gap beyond its parent's edge. Branches keep the
//! side of the root they are already on, and siblings keep their current top-to-bottom
//! order, so tidying respects how the user arranged things.

use crate::map_view::estimated_size;
use crate::model::{MapError, MindMap, NodeId, Point};

/// Horizontal gap between a node's edge and its children's edges, in world units.
pub(crate) const COLUMN_GAP: f32 = 70.0;
/// Vertical space reserved for each leaf row, in world units.
pub(crate) const ROW_HEIGHT: f32 = 56.0;

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
    let order: Vec<NodeId> = map.preorder().map(|visit| visit.id).collect();
    let mut tidier = Tidier::new(map.slot_count());
    tidier.count_rows(map, &order)?;

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
            let own = own_rows(node.text());
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
        let parent_half_width = estimated_size(parent_node.text()).x * 0.5;

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
            let half_width = estimated_size(child_node.text()).x * 0.5;
            let x = parent_pos.x + dir * (parent_half_width + COLUMN_GAP + half_width);
            map.set_position(child, Point::new(x, top + height * 0.5))?;
            top += height;
        }
        Ok(())
    }
}

/// Rows a node needs for its own box: one for a single-line label, more for a label
/// that wraps, keeping the same spacing between boxes either way.
fn own_rows(text: &str) -> f32 {
    let spacing = ROW_HEIGHT - estimated_size("").y;
    // The small epsilon stops float rounding from turning exactly 1.0 into 2 rows.
    ((estimated_size(text).y + spacing) / ROW_HEIGHT - 1e-3)
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
        map.add_child_at(map.root(), &long, origin).expect("root");
        map.add_child_at(map.root(), "short", Point::new(0.0, 1.0))
            .expect("root");

        tidy(&mut map).expect("consistent map");

        assert!(own_rows(&long) > 1.0);
        assert!((own_rows("short") - 1.0).abs() < f32::EPSILON);
        assert_no_overlaps(&map);
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
