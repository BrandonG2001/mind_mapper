//! Moving the selection around the map with the arrow keys.
//!
//! Directions follow the picture on screen: on the right half of the map, Right goes out
//! to a child and Left back to the parent; on the left half it is the other way round.
//! Up and Down step between siblings on the same side.

use crate::model::{MindMap, NodeId};

/// An arrow key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Arrow up: the sibling above.
    Up,
    /// Arrow down: the sibling below.
    Down,
    /// Arrow left: outwards on the left half, back to the parent on the right half.
    Left,
    /// Arrow right: outwards on the right half, back to the parent on the left half.
    Right,
}

/// Where the selection goes when `direction` is pressed while `from` is selected.
///
/// # Returns
///
/// The node to select, or `None` if there is nothing in that direction (an outermost
/// node, a folded branch, the top or bottom sibling).
#[must_use]
pub fn neighbor(map: &MindMap, from: NodeId, direction: Direction) -> Option<NodeId> {
    let node = map.node(from)?;
    let x = node.position().x;
    let y = node.position().y;
    let parent = node.parent().and_then(|id| Some((id, map.node(id)?)));
    let x_of = |id: NodeId| map.node(id).map_or(x, |n| n.position().x);
    let y_of = |id: NodeId| map.node(id).map_or(y, |n| n.position().y);

    match direction {
        Direction::Up | Direction::Down => {
            let (_, parent_node) = parent?;
            let parent_x = parent_node.position().x;
            let on_right = x >= parent_x;
            let siblings = parent_node
                .children()
                .iter()
                .copied()
                .filter(|&id| id != from && (x_of(id) >= parent_x) == on_right);
            if direction == Direction::Up {
                siblings
                    .filter(|&id| y_of(id) < y)
                    .max_by(|&a, &b| y_of(a).total_cmp(&y_of(b)))
            } else {
                siblings
                    .filter(|&id| y_of(id) > y)
                    .min_by(|&a, &b| y_of(a).total_cmp(&y_of(b)))
            }
        }
        Direction::Left | Direction::Right => {
            let toward_right = direction == Direction::Right;
            if let Some((parent_id, parent_node)) = parent {
                let grows_right = x >= parent_node.position().x;
                if grows_right != toward_right {
                    return Some(parent_id);
                }
            }
            if node.is_collapsed() {
                return None;
            }
            // Outwards: the child closest in height, on the requested side (this only
            // filters anything for the root, whose branches spread both ways).
            node.children()
                .iter()
                .copied()
                .filter(|&id| (x_of(id) >= x) == toward_right)
                .min_by(|&a, &b| (y_of(a) - y).abs().total_cmp(&(y_of(b) - y).abs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;

    /// Root with two right branches (one with two children) and one left branch.
    struct Sample {
        map: MindMap,
        right_top: NodeId,
        right_bottom: NodeId,
        right_child_a: NodeId,
        right_child_b: NodeId,
        left: NodeId,
    }

    fn sample() -> Sample {
        let mut map = MindMap::new("root");
        let root = map.root();
        let mut add = |parent, x, y| {
            map.add_child_at(parent, "n", Point::new(x, y))
                .expect("parent exists")
        };
        let right_top = add(root, 200.0, -50.0);
        let right_bottom = add(root, 200.0, 50.0);
        let left = add(root, -200.0, 10.0);
        let right_child_a = add(right_top, 400.0, -80.0);
        let right_child_b = add(right_top, 400.0, -10.0);
        Sample {
            map,
            right_top,
            right_bottom,
            right_child_a,
            right_child_b,
            left,
        }
    }

    #[test]
    fn root_goes_to_nearest_branch_on_each_side() {
        let s = sample();
        let root = s.map.root();

        assert_eq!(neighbor(&s.map, root, Direction::Right), Some(s.right_top));
        assert_eq!(neighbor(&s.map, root, Direction::Left), Some(s.left));
        assert_eq!(neighbor(&s.map, root, Direction::Up), None);
    }

    #[test]
    fn right_side_goes_out_with_right_and_back_with_left() {
        let s = sample();

        // `right_child_a` (y -80) is closer to `right_top` (y -50) than `right_child_b` (y -10).
        assert_eq!(
            neighbor(&s.map, s.right_top, Direction::Right),
            Some(s.right_child_a)
        );
        assert_eq!(
            neighbor(&s.map, s.right_child_b, Direction::Left),
            Some(s.right_top)
        );
        assert_eq!(neighbor(&s.map, s.right_child_a, Direction::Right), None);
    }

    #[test]
    fn left_side_is_mirrored() {
        let s = sample();

        assert_eq!(
            neighbor(&s.map, s.left, Direction::Right),
            Some(s.map.root())
        );
        assert_eq!(neighbor(&s.map, s.left, Direction::Left), None);
    }

    #[test]
    fn up_and_down_step_through_same_side_siblings() {
        let s = sample();

        assert_eq!(
            neighbor(&s.map, s.right_top, Direction::Down),
            Some(s.right_bottom)
        );
        assert_eq!(
            neighbor(&s.map, s.right_bottom, Direction::Up),
            Some(s.right_top)
        );
        assert_eq!(neighbor(&s.map, s.right_top, Direction::Up), None);
        // The left branch is not a sibling "below" the right ones.
        assert_eq!(neighbor(&s.map, s.left, Direction::Up), None);
        assert_eq!(
            neighbor(&s.map, s.right_child_a, Direction::Down),
            Some(s.right_child_b)
        );
    }

    #[test]
    fn folded_branch_is_not_entered() {
        let mut s = sample();
        s.map.set_collapsed(s.right_top, true).expect("exists");

        assert_eq!(neighbor(&s.map, s.right_top, Direction::Right), None);
    }
}
