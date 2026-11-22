//! Built-in starting points for new mind maps.
//!
//! Each template is a static tree of labels. [`Template::build`] turns it into a
//! [`MindMap`], spreading the root's branches over both sides and laying it out with
//! [`tidy`](crate::layout::tidy).

use crate::layout::{TreeBuilder, balanced_left};
use crate::model::{MindMap, NodeId};

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

impl Template {
    /// Create a new mind map from this template.
    ///
    /// The first half of the root's branches go to the right, the rest to the left, and
    /// [`tidy`](crate::layout::tidy) arranges everything without overlaps.
    ///
    /// # Panics
    ///
    /// Only on an internal bug: the layout reports a missing node, which cannot happen for
    /// nodes this function has just created.
    #[must_use]
    pub fn build(&self) -> MindMap {
        let mut builder = TreeBuilder::new(self.root.text);
        let root = builder.root();
        let count = self.root.children.len();
        for (index, branch) in self.root.children.iter().enumerate() {
            add_branch(&mut builder, root, branch, balanced_left(index, count));
        }
        builder
            .finish()
            .expect("every template node was just created")
    }
}

/// Add `spec` and its descendants under `parent`. (Recursion is fine here: templates are
/// a few levels deep and fixed at compile time.)
fn add_branch(builder: &mut TreeBuilder, parent: NodeId, spec: &TemplateNode, left: bool) {
    let id = builder
        .add(parent, spec.text, left)
        .expect("parent was created earlier in this build");
    for child in spec.children {
        add_branch(builder, id, child, left);
    }
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

        let sides: Vec<bool> = root
            .children()
            .iter()
            .map(|&id| map.node(id).expect("child exists").position().x > 0.0)
            .collect();

        assert_eq!(sides, vec![true, true, false, false]);
    }

    #[test]
    fn template_order_is_kept_top_to_bottom() {
        let map = TEMPLATES[2].build();
        let root = map.node(map.root()).expect("root exists");
        let ys: Vec<f32> = root.children()[..3]
            .iter()
            .map(|&id| map.node(id).expect("child exists").position().y)
            .collect();

        assert!(ys.windows(2).all(|pair| pair[0] < pair[1]), "{ys:?}");
    }

    #[test]
    fn templates_have_no_overlapping_nodes() {
        for template in &TEMPLATES {
            let map = template.build();
            let rects: Vec<_> = map
                .iter()
                .map(|(_, node)| crate::map_view::estimated_rect(node))
                .collect();
            for (index, rect) in rects.iter().enumerate() {
                assert!(
                    rects[index + 1..]
                        .iter()
                        .all(|other| !rect.shrink(0.5).intersects(other.shrink(0.5))),
                    "{} has overlapping nodes",
                    template.name
                );
            }
        }
    }
}
