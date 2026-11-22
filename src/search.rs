//! Finding nodes by their text, and replacing it: the find bar and its list of matches.

use eframe::egui::{self, Key, Ui};

use crate::model::{MapError, MindMap, NodeId};

/// Width of the find bar's text field.
const FIELD_WIDTH: f32 = 240.0;

/// Every node whose label or notes contain `query`, ignoring case, in tree order.
/// An empty (or blank) query matches nothing.
#[must_use]
pub fn find_matches(map: &MindMap, query: &str) -> Vec<NodeId> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    map.preorder()
        .filter(|visit| {
            visit.node.text().to_lowercase().contains(&query)
                || visit.node.note().to_lowercase().contains(&query)
        })
        .map(|visit| visit.id)
        .collect()
}

/// `text` with every occurrence of `query` (ignoring case, like [`find_matches`])
/// replaced by `replacement`.
///
/// # Returns
///
/// The new text and how many occurrences were replaced.
#[must_use]
pub fn replace_ignoring_case(text: &str, query: &str, replacement: &str) -> (String, usize) {
    // The query in lower case, one char at a time (some characters lower to several).
    let needle: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return (text.to_owned(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        if let Some(length) = matching_prefix(rest, &needle) {
            out.push_str(replacement);
            rest = &rest[length..];
            count += 1;
        } else {
            out.push(next);
            rest = &rest[next.len_utf8()..];
        }
    }
    (out, count)
}

/// If `text` starts with the lower-cased `needle` (ignoring case), the length in bytes of
/// the part of `text` that matched.
fn matching_prefix(text: &str, needle: &[char]) -> Option<usize> {
    let mut wanted = needle.iter();
    for (at, character) in text.char_indices() {
        for lower in character.to_lowercase() {
            if wanted.next() != Some(&lower) {
                return None;
            }
        }
        if wanted.as_slice().is_empty() {
            return Some(at + character.len_utf8());
        }
    }
    None
}

/// Replace `query` with `replacement` (ignoring case) in the labels and notes of every
/// node, or only of node `only`. A label that would become blank is left as it was.
///
/// # Returns
///
/// How many occurrences were replaced.
///
/// # Errors
///
/// Returns `MapError::NodeNotFound` if `only` names a node that does not exist.
pub fn replace_in_map(
    map: &mut MindMap,
    query: &str,
    replacement: &str,
    only: Option<NodeId>,
) -> Result<usize, MapError> {
    let targets: Vec<NodeId> = match only {
        Some(id) => vec![id],
        None => map.iter().map(|(id, _)| id).collect(),
    };
    let mut total = 0;
    for id in targets {
        let node = map.node(id).ok_or(MapError::NodeNotFound(id))?;
        let (new_text, in_text) = replace_ignoring_case(node.text(), query, replacement);
        let (new_note, in_note) = replace_ignoring_case(node.note(), query, replacement);
        if in_text > 0 && !new_text.trim().is_empty() {
            map.set_text(id, new_text.trim())?;
            total += in_text;
        }
        if in_note > 0 {
            map.set_note(id, &new_note)?;
            total += in_note;
        }
    }
    Ok(total)
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
    /// Replace the text in the current match, then go to the next one.
    Replace,
    /// Replace the text in every match, as one undo step.
    ReplaceAll,
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
    /// Whether the "Replace with" row is shown.
    replacing: bool,
    /// The text that replaces the query.
    replacement: String,
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

    /// Show the "Replace with" row (Ctrl+H), with the keyboard in the search field.
    pub fn show_replace(&mut self) {
        self.replacing = true;
        self.focus_requested = true;
    }

    /// The text being searched for.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The text that replaces it.
    #[must_use]
    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    /// The match last jumped to, if any.
    #[must_use]
    pub fn current(&self) -> Option<NodeId> {
        self.current_match()
    }

    /// How many nodes match (as of the last [`Search::refresh`]).
    #[must_use]
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Replace the query, as if typed (tests drive the find bar without a window).
    #[cfg(test)]
    pub(crate) fn set_query(&mut self, query: &str) {
        query.clone_into(&mut self.query);
    }

    /// Set the replacement text, as if typed.
    #[cfg(test)]
    pub(crate) fn set_replacement(&mut self, replacement: &str) {
        replacement.clone_into(&mut self.replacement);
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

    /// After the current match `replaced` was edited (the map is now at `revision`), move
    /// to the next match. If `replaced` no longer matches, the next match has taken its
    /// place in the list, so the position stays (wrapping around at the end).
    pub fn next_after_replace(
        &mut self,
        map: &MindMap,
        revision: u64,
        replaced: NodeId,
    ) -> Option<NodeId> {
        let position = self.current;
        self.refresh(map, revision);
        if self.is_match(replaced) {
            return self.step(true);
        }
        let count = self.matches.len();
        self.current = position.filter(|_| count > 0).map(|index| index % count);
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
            // The label also names the field for screen readers.
            let label = ui.label("Find");
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search node labels")
                        .desired_width(FIELD_WIDTH),
                )
                .labelled_by(label.id);
            if std::mem::take(&mut self.focus_requested) {
                field.request_focus();
            }
            if field.changed() {
                command = Some(SearchCommand::QueryChanged);
            }
            // F3 / Shift+F3 step through matches while typing, as they do elsewhere (other
            // shortcuts are paused while a text field has focus).
            if field.has_focus()
                && let Some(shift) =
                    ui.input(|input| input.key_pressed(Key::F3).then_some(input.modifiers.shift))
            {
                command = Some(if shift {
                    SearchCommand::Previous
                } else {
                    SearchCommand::Next
                });
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
            if !self.replacing && ui.button("Replace...").on_hover_text("Ctrl+H").clicked() {
                self.replacing = true;
            }
            if ui.button("Close").on_hover_text("Escape").clicked() {
                command = Some(SearchCommand::Close);
            }
        });
        if self.replacing {
            ui.horizontal(|ui| {
                let label = ui.label("Replace with");
                ui.add(
                    egui::TextEdit::singleline(&mut self.replacement)
                        .hint_text("New text")
                        .desired_width(FIELD_WIDTH),
                )
                .labelled_by(label.id);
                let has_matches = !self.matches.is_empty();
                if ui
                    .add_enabled(has_matches, egui::Button::new("Replace"))
                    .on_hover_text("Replace in the current match, then go to the next")
                    .clicked()
                {
                    command = Some(SearchCommand::Replace);
                }
                if ui
                    .add_enabled(has_matches, egui::Button::new("Replace all"))
                    .on_hover_text("Replace in every match (one undo step)")
                    .clicked()
                {
                    command = Some(SearchCommand::ReplaceAll);
                }
            });
        }
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
    fn replacing_ignores_case_and_counts_occurrences() {
        assert_eq!(
            replace_ignoring_case("Plan A, plan B", "PLAN", "Option"),
            ("Option A, Option B".to_owned(), 2)
        );
        assert_eq!(
            replace_ignoring_case("Größe und GRÖSSE", "größe", "Size"),
            ("Size und GRÖSSE".to_owned(), 1)
        );
        assert_eq!(
            replace_ignoring_case("abc", "  ", "x"),
            ("abc".to_owned(), 0)
        );
        assert_eq!(
            replace_ignoring_case("aaa", "aa", "b"),
            ("ba".to_owned(), 1)
        );
    }

    #[test]
    fn replacing_in_a_map_covers_labels_and_notes_but_never_blanks_a_label() {
        let (mut map, goals, plan_b, other) = sample();
        map.set_note(other, "see the plan").expect("other");

        let one = replace_in_map(&mut map, "plan", "path", Some(plan_b)).expect("exists");
        assert_eq!(one, 1);
        assert_eq!(map.node(plan_b).expect("live").text(), "Backup path");
        assert_eq!(map.node(map.root()).expect("root").text(), "Project plan");

        let all = replace_in_map(&mut map, "PLAN", "route", None).expect("valid");
        assert_eq!(all, 2, "the root's label and the note");
        assert_eq!(map.node(other).expect("live").note(), "see the route");
        assert_eq!(replace_in_map(&mut map, "Goals", " ", None), Ok(0));
        assert_eq!(map.node(goals).expect("live").text(), "Goals");
    }

    #[test]
    fn matches_ignore_case_and_follow_tree_order() {
        let (map, _, plan_b, other) = sample();

        assert_eq!(find_matches(&map, " plan "), vec![map.root(), plan_b]);
        assert_eq!(find_matches(&map, "OTH"), vec![other]);
        assert_eq!(find_matches(&map, "   "), Vec::new());
        assert_eq!(find_matches(&map, "missing"), Vec::new());
        let mut map = map;
        map.set_note(other, "mentions the PLAN too").expect("other");
        assert_eq!(find_matches(&map, "plan").len(), 3);
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
