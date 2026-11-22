//! Built-in starting points for new mind maps.
//!
//! Each template is a static tree of labels. [`Template::build`] turns it into a
//! [`MindMap`], spreading the root's branches over both sides and giving every leaf its
//! own row so nothing overlaps.

use crate::model::{MindMap, NodeId, Point};

/// Horizontal distance between tree levels in a built template.
const COLUMN_WIDTH: f32 = 230.0;
/// Vertical space reserved for each leaf in a built template.
const ROW_HEIGHT: f32 = 56.0;

/// One label in a template tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateNode {
    /// Label of the node.
    pub text: &'static str,
    /// Child nodes, in order.
    pub children: &'static [TemplateNode],
}

/// A named, ready-made mind-map structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Template {
    /// Short name shown on the start screen.
    pub name: &'static str,
    /// One-sentence description shown under the name.
    pub description: &'static str,
    /// The central idea and its branches.
    pub root: TemplateNode,
}

/// Build a [`TemplateNode`] from a label and optional `=> [children]`.
///
/// A macro (rather than a `const fn`) because struct and array literals written directly
/// in a constant are stored as `'static` data, whereas values passed through function
/// arguments are temporaries that cannot be borrowed for `'static`.
macro_rules! node {
    ($text:literal) => {
        TemplateNode {
            text: $text,
            children: &[],
        }
    };
    ($text:literal => [$($child:expr),* $(,)?]) => {
        TemplateNode {
            text: $text,
            children: &[$($child),*],
        }
    };
}

/// Every built-in template, in the order shown on the start screen.
pub const TEMPLATES: [Template; 8] = [
    Template {
        name: "Blank",
        description: "A single central idea to grow from.",
        root: node!("Central idea"),
    },
    Template {
        name: "Brainstorm",
        description: "Collect ideas, questions and next steps.",
        root: node!("Brainstorm" => [
            node!("Ideas" => [node!("Idea 1"), node!("Idea 2")]),
            node!("Questions" => [node!("Question 1")]),
            node!("Resources" => [node!("Link or reference")]),
            node!("Next steps" => [node!("Action 1")]),
        ]),
    },
    Template {
        name: "Project Plan",
        description: "Goals, scope, timeline, team and risks.",
        root: node!("Project" => [
            node!("Goals" => [node!("Objective 1"), node!("Objective 2")]),
            node!("Scope" => [node!("In scope"), node!("Out of scope")]),
            node!("Timeline" => [node!("Phase 1"), node!("Phase 2"), node!("Phase 3")]),
            node!("Team" => [node!("Owner"), node!("Stakeholders")]),
            node!("Risks" => [node!("Risk"), node!("Mitigation")]),
        ]),
    },
    Template {
        name: "SWOT Analysis",
        description: "Strengths, weaknesses, opportunities, threats.",
        root: node!("SWOT" => [
            node!("Strengths" => [node!("Strength 1"), node!("Strength 2")]),
            node!("Weaknesses" => [node!("Weakness 1"), node!("Weakness 2")]),
            node!("Opportunities" => [node!("Opportunity 1"), node!("Opportunity 2")]),
            node!("Threats" => [node!("Threat 1"), node!("Threat 2")]),
        ]),
    },
    Template {
        name: "Meeting Notes",
        description: "Attendees, agenda, decisions and action items.",
        root: node!("Meeting" => [
            node!("Attendees" => [node!("Name")]),
            node!("Agenda" => [node!("Topic 1"), node!("Topic 2")]),
            node!("Decisions" => [node!("Decision 1")]),
            node!("Action items" => [node!("Task - owner - due date")]),
        ]),
    },
    Template {
        name: "Decision Making",
        description: "Weigh options against your criteria.",
        root: node!("Decision" => [
            node!("Option A" => [node!("Pros"), node!("Cons")]),
            node!("Option B" => [node!("Pros"), node!("Cons")]),
            node!("Criteria" => [node!("Cost"), node!("Time"), node!("Impact")]),
            node!("Outcome" => [node!("Chosen option"), node!("Reason")]),
        ]),
    },
    Template {
        name: "Study Notes",
        description: "Break a topic into concepts and examples.",
        root: node!("Topic" => [
            node!("Key concepts" => [node!("Concept 1"), node!("Concept 2")]),
            node!("Definitions" => [node!("Term")]),
            node!("Examples" => [node!("Example 1")]),
            node!("Questions" => [node!("What is unclear?")]),
            node!("Summary"),
        ]),
    },
    Template {
        name: "Weekly Planner",
        description: "Plan priorities for each day of the week.",
        root: node!("This week" => [
            node!("Monday" => [node!("Task")]),
            node!("Tuesday" => [node!("Task")]),
            node!("Wednesday" => [node!("Task")]),
            node!("Thursday" => [node!("Task")]),
            node!("Friday" => [node!("Task")]),
            node!("Weekend" => [node!("Rest")]),
        ]),
    },
];

impl TemplateNode {
    /// Number of leaves below (or including) this node, i.e. how many rows it needs.
    fn leaf_count(&self) -> usize {
        self.children
            .iter()
            .map(Self::leaf_count)
            .sum::<usize>()
            .max(1)
    }
}

impl Template {
    /// Create a new mind map from this template.
    ///
    /// The first half of the root's branches go to the right, the rest to the left, and
    /// each subtree is centred vertically on the rows of its leaves.
    #[must_use]
    pub fn build(&self) -> MindMap {
        let mut map = MindMap::new(self.root.text);
        let root = map.root();
        let (right, left) = self
            .root
            .children
            .split_at(self.root.children.len().div_ceil(2));
        place_side(&mut map, root, right, 1.0);
        place_side(&mut map, root, left, -1.0);
        map
    }
}

/// Lay out the root's branches on one side, centred vertically on the root.
fn place_side(map: &mut MindMap, root: NodeId, branches: &[TemplateNode], direction: f32) {
    let rows: usize = branches.iter().map(TemplateNode::leaf_count).sum();
    let mut top = -rows_height(rows) * 0.5;
    for branch in branches {
        top = place_subtree(map, root, branch, direction, top);
    }
}

/// Add `spec` and its descendants under `parent`, starting at `top`.
///
/// # Returns
///
/// The y coordinate just below the rows this subtree used.
fn place_subtree(
    map: &mut MindMap,
    parent: NodeId,
    spec: &TemplateNode,
    direction: f32,
    top: f32,
) -> f32 {
    let height = rows_height(spec.leaf_count());
    let parent_x = map.node(parent).map_or(0.0, |node| node.position().x);
    let position = Point::new(parent_x + direction * COLUMN_WIDTH, top + height * 0.5);
    let id = map
        .add_child_at(parent, spec.text, position)
        .expect("parent was created earlier in this build");
    let mut child_top = top;
    for child in spec.children {
        child_top = place_subtree(map, id, child, direction, child_top);
    }
    top + height
}

/// Height in world units of `rows` leaf rows.
fn rows_height(rows: usize) -> f32 {
    // Templates are small and static; saturating keeps the conversion lossless.
    f32::from(u16::try_from(rows).unwrap_or(u16::MAX)) * ROW_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_count(spec: &TemplateNode) -> usize {
        1 + spec.children.iter().map(node_count).sum::<usize>()
    }

    #[test]
    fn every_template_builds_all_of_its_nodes() {
        for template in &TEMPLATES {
            let map = template.build();

            assert_eq!(map.len(), node_count(&template.root), "{}", template.name);
            let root = map.node(map.root()).expect("root exists");
            assert_eq!(root.text(), template.root.text);
        }
    }

    #[test]
    fn template_names_are_unique() {
        for (index, template) in TEMPLATES.iter().enumerate() {
            assert!(
                TEMPLATES[index + 1..]
                    .iter()
                    .all(|other| other.name != template.name),
                "duplicate template name {}",
                template.name
            );
        }
    }

    #[test]
    fn branches_are_split_across_both_sides() {
        let map = TEMPLATES[3].build();
        let root = map.node(map.root()).expect("root exists");

        let xs: Vec<f32> = root
            .children()
            .iter()
            .map(|&id| map.node(id).expect("child exists").position().x)
            .collect();

        assert_eq!(
            xs,
            vec![COLUMN_WIDTH, COLUMN_WIDTH, -COLUMN_WIDTH, -COLUMN_WIDTH]
        );
    }

    #[test]
    fn nodes_in_the_same_column_do_not_overlap() {
        for template in &TEMPLATES {
            let map = template.build();
            let mut positions: Vec<Point> = map.iter().map(|(_, node)| node.position()).collect();
            positions.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));

            for pair in positions.windows(2) {
                if (pair[0].x - pair[1].x).abs() < f32::EPSILON {
                    assert!(
                        pair[1].y - pair[0].y >= ROW_HEIGHT - f32::EPSILON,
                        "{}: {pair:?} overlap",
                        template.name
                    );
                }
            }
        }
    }

    #[test]
    fn leaf_count_counts_rows() {
        let spec = node!("a" => [node!("b" => [node!("c"), node!("d")]), node!("e")]);

        assert_eq!(spec.leaf_count(), 3);
        assert_eq!(node!("solo").leaf_count(), 1);
    }
}
