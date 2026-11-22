//! Finding nodes by their text: the find bar and its list of matches.

use eframe::egui::{self, Key, Ui};

use crate::model::{MindMap, NodeId};

/// Width of the find bar's text field.
const FIELD_WIDTH: f32 = 240.0;

/// Every node whose label contains `query`, ignoring case, in tree order.
/// An empty (or blank) query matches nothing.
#[must_use]
pub fn find_matches(map: &MindMap, query: &str) -> Vec<NodeId> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    map.preorder()
        .filter(|visit| visit.node.text().to_lowercase().contains(&query))
        .map(|visit| visit.id)
        .collect()
}

/// What the user asked the find bar to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchCommand {
    /// The query was edited: jump to the first match.
    QueryChanged,
    /// Go to the next match (Enter, F3 or the button).
    Next,
    /// Go to the previous match (Shift+Enter, Shift+F3 or the button).
    Previous,
    /// Close the find bar (Escape or the button).
    Close,
}

/// State of an open find bar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Search {
    query: String,
    matches: Vec<NodeId>,
    /// `is_match[id.index()]`, for O(1) highlighting while drawing.
    is_match: Vec<bool>,
    /// Position in `matches` of the match last jumped to.
    current: Option<usize>,
    /// The query and map revision `matches` was computed for; `None` = never computed.
    computed_for: Option<(String, u64)>,
    focus_requested: bool,
    has_focus: bool,
}

impl Search {
    /// A new, empty find bar that grabs keyboard focus when first shown.
    #[must_use]
    pub fn new() -> Self {
        Self {
            focus_requested: true,
            ..Self::default()
        }
    }

    /// Put the keyboard focus back in the text field (e.g. Ctrl+F while already open).
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    /// Whether the text field has keyboard focus, so other shortcuts should be ignored.
    #[must_use]
    pub const fn has_focus(&self) -> bool {
        self.has_focus
    }

    /// Replace the query, as if typed (tests drive the find bar without a window).
    #[cfg(test)]
    pub(crate) fn set_query(&mut self, query: &str) {
        query.clone_into(&mut self.query);
    }

    /// Whether `id` matches the query (as of the last [`Search::refresh`]).
    #[must_use]
    pub fn is_match(&self, id: NodeId) -> bool {
        self.is_match.get(id.index()).copied().unwrap_or(false)
    }

    /// Recompute the matches if the query or the map changed since last time.
    ///
    /// # Arguments
    ///
    /// * `map` - The map being searched
    /// * `revision` - A number that changes whenever the map is edited
    pub fn refresh(&mut self, map: &MindMap, revision: u64) {
        let up_to_date = self
            .computed_for
            .as_ref()
            .is_some_and(|(query, rev)| *query == self.query && *rev == revision);
        if up_to_date {
            return;
        }
        self.matches = find_matches(map, &self.query);
        self.is_match.clear();
        self.is_match.resize(map.slot_count(), false);
        for id in &self.matches {
            self.is_match[id.index()] = true;
        }
        self.current = self.current.filter(|&index| index < self.matches.len());
        self.computed_for = Some((self.query.clone(), revision));
    }

    /// Move to the first match.
    pub fn first(&mut self) -> Option<NodeId> {
        self.current = (!self.matches.is_empty()).then_some(0);
        self.current_match()
    }

    /// Move to the next (`forward`) or previous match, wrapping around at the ends.
    pub fn step(&mut self, forward: bool) -> Option<NodeId> {
        let count = self.matches.len();
        if count == 0 {
            return None;
        }
        self.current = Some(match (self.current, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
        });
        self.current_match()
    }

    fn current_match(&self) -> Option<NodeId> {
        self.current
            .and_then(|index| self.matches.get(index).copied())
    }

    /// Draw the find bar: text field, match count and buttons.
    ///
    /// # Returns
    ///
    /// The command the user gave this frame, if any.
    pub fn show(&mut self, ui: &mut Ui) -> Option<SearchCommand> {
        let mut command = None;
        ui.horizontal(|ui| {
            ui.label("Find");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search node labels")
                    .desired_width(FIELD_WIDTH),
            );
            if std::mem::take(&mut self.focus_requested) {
                field.request_focus();
            }
            if field.changed() {
                command = Some(SearchCommand::QueryChanged);
            }
            // A single-line text field gives up focus when Enter or Escape is pressed.
            if field.lost_focus() {
                let (enter, escape, shift) = ui.input(|input| {
                    (
                        input.key_pressed(Key::Enter),
                        input.key_pressed(Key::Escape),
                        input.modifiers.shift,
                    )
                });
                if escape {
                    command = Some(SearchCommand::Close);
                } else if enter {
                    command = Some(if shift {
                        SearchCommand::Previous
                    } else {
                        SearchCommand::Next
                    });
                    // Keep typing after jumping.
                    field.request_focus();
                }
            }
            self.has_focus = field.has_focus();

            let summary = match (self.current, self.matches.len()) {
                (_, 0) if self.query.trim().is_empty() => String::new(),
                (_, 0) => "No matches".to_owned(),
                (Some(index), count) => format!("{} of {count}", index + 1),
                (None, count) => format!("{count} matches"),
            };
            ui.label(summary);
            if ui
                .button("Previous")
                .on_hover_text("Shift+Enter or Shift+F3")
                .clicked()
            {
                command = Some(SearchCommand::Previous);
            }
            if ui.button("Next").on_hover_text("Enter or F3").clicked() {
                command = Some(SearchCommand::Next);
            }
            if ui.button("Close").on_hover_text("Escape").clicked() {
                command = Some(SearchCommand::Close);
            }
        });
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (MindMap, NodeId, NodeId, NodeId) {
        let mut map = MindMap::new("Project plan");
        let goals = map.add_child(map.root(), "Goals").expect("root");
        let plan_b = map.add_child(goals, "Backup PLAN").expect("goals");
        let other = map.add_child(map.root(), "Other").expect("root");
        (map, goals, plan_b, other)
    }

    #[test]
    fn matches_ignore_case_and_follow_tree_order() {
        let (map, _, plan_b, other) = sample();

        assert_eq!(find_matches(&map, " plan "), vec![map.root(), plan_b]);
        assert_eq!(find_matches(&map, "OTH"), vec![other]);
        assert_eq!(find_matches(&map, "   "), Vec::new());
        assert_eq!(find_matches(&map, "missing"), Vec::new());
    }

    #[test]
    fn step_wraps_around_in_both_directions() {
        let (map, _, plan_b, other) = sample();
        let mut search = Search::new();
        "plan".clone_into(&mut search.query);
        search.refresh(&map, 0);

        assert_eq!(search.first(), Some(map.root()));
        assert_eq!(search.step(true), Some(plan_b));
        assert_eq!(search.step(true), Some(map.root()));
        assert_eq!(search.step(false), Some(plan_b));
        assert!(search.is_match(plan_b));
        assert!(!search.is_match(other));
    }

    #[test]
    fn refresh_recomputes_only_when_needed() {
        let (mut map, goals, _, _) = sample();
        let mut search = Search::new();
        "goal".clone_into(&mut search.query);
        search.refresh(&map, 0);
        assert_eq!(search.matches, vec![goals]);

        map.set_text(goals, "Aims").expect("goals exist");
        search.refresh(&map, 0);
        assert_eq!(search.matches, vec![goals], "same revision: cached");
        search.refresh(&map, 1);

        assert_eq!(search.matches, Vec::new());
        assert_eq!(search.step(true), None);
    }
}
