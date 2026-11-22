//! Cut, copy and paste of branches through the system clipboard.
//!
//! Branches travel as indented bullet outlines (Markdown), so a copied branch can be
//! pasted into any document, and a bullet list copied from anywhere can be pasted into a
//! map as new branches.

use crate::layout;
use crate::model::{MapError, MindMap, NodeId};

/// Spaces added per outline level, and the width counted for a tab when reading one.
const INDENT: usize = 2;
/// Width of a tab character when measuring indentation.
const TAB_WIDTH: usize = 4;

/// One line of a parsed outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    /// Index of the item this one is nested under, or `None` for a top-level item.
    /// Always smaller than the item's own index.
    pub parent: Option<usize>,
    /// The line's text, without bullet markers.
    pub text: String,
    /// The address of a Markdown link (`[text](address)`) the line consisted of.
    pub link: Option<String>,
}

/// `branches` and their descendants as one bullet outline, each branch a top-level item,
/// e.g. `"- Goals\n  - Ship it\n- Risks\n"`. Nodes with a web link become Markdown links,
/// `- [Goals](https://...)`.
///
/// The branches should not contain each other (a nested one would appear twice).
#[must_use]
pub fn branches_outline(map: &MindMap, branches: &[NodeId]) -> String {
    // One depth table for all branches, so the cost is O(slots + outlined nodes).
    let mut depth = vec![0_usize; map.slot_count()];
    let mut out = String::new();
    // `move` copies `branch` into the inner closure, which outlives this loop step.
    let visits = branches.iter().flat_map(|&branch| {
        map.subtree_preorder(branch)
            .map(move |visit| (branch, visit))
    });
    for (branch, visit) in visits {
        let level = match visit.node.parent() {
            Some(parent) if visit.id != branch => depth[parent.index()] + 1,
            _ => 0,
        };
        depth[visit.id.index()] = level;
        out.push_str(&" ".repeat(level * INDENT));
        out.push_str("- ");
        // A line break inside a label would end the bullet early.
        let text = visit.node.text().replace(['\r', '\n'], " ");
        match visit.node.link() {
            Some(link) => {
                out.push('[');
                out.push_str(&text);
                out.push_str("](");
                out.push_str(link);
                out.push(')');
            }
            None => out.push_str(&text),
        }
        out.push('\n');
    }
    out
}

/// Read an outline: Markdown headings, `-`/`*`/`+` bullets, `1.` or `1)` numbered items or
/// plain lines, nested by indentation (spaces or tabs). Blank lines are skipped.
#[must_use]
pub fn parse_outline(text: &str) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    // Open ancestors: (indentation, item index). Headings get negative indentation so
    // every bullet nests under the nearest heading above it.
    let mut open: Vec<(isize, usize)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let width: usize = line[..line.len() - trimmed.len()]
            .chars()
            .map(|character| if character == '\t' { TAB_WIDTH } else { 1 })
            .sum();
        let (indent, label) = match heading_level(trimmed) {
            // `#` -> -6 ... `######` -> -1: deeper headings nest under shallower ones.
            Some((level, rest)) => (level - 7, rest),
            None => (
                isize::try_from(width).unwrap_or(isize::MAX),
                strip_marker(trimmed),
            ),
        };
        let label = label.trim();
        if label.is_empty() {
            continue;
        }
        while open
            .last()
            .is_some_and(|&(open_indent, _)| open_indent >= indent)
        {
            open.pop();
        }
        let (text, link) = split_link(label);
        items.push(OutlineItem {
            parent: open.last().map(|&(_, index)| index),
            text: text.to_owned(),
            link: link.map(str::to_owned),
        });
        open.push((indent, items.len() - 1));
    }
    items
}

/// Split a Markdown link line, `[text](address)`, into its text and address. Anything
/// else is returned unchanged with no address.
fn split_link(label: &str) -> (&str, Option<&str>) {
    label
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|inner| inner.split_once("]("))
        .filter(|(text, address)| !text.is_empty() && !address.trim().is_empty())
        .map_or((label, None), |(text, address)| {
            (text, Some(address.trim()))
        })
}

/// The level (1-6) and text of a Markdown heading line.
fn heading_level(line: &str) -> Option<(isize, &str)> {
    let hashes = line
        .chars()
        .take_while(|&character| character == '#')
        .count();
    let rest = &line[hashes..];
    // `hashes` is at most 6 here, so the conversion is lossless.
    ((1..=6).contains(&hashes) && rest.starts_with(' '))
        .then(|| (isize::try_from(hashes).unwrap_or(6), rest))
}

/// `line` without a leading bullet (`- `, `* `, `+ `) or number (`1. `, `1) `).
fn strip_marker(line: &str) -> &str {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return rest;
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let after = &line[digits..];
    if digits > 0
        && let Some(rest) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
    {
        return rest;
    }
    line
}

/// Add `items` as new branches under `target`, then lay each pasted branch out tidily.
/// A folded `target` is unfolded so the pasted nodes are visible.
///
/// # Returns
///
/// The ids of the new top-level branches, in order.
///
/// # Errors
///
/// * `MapError::NodeNotFound` if `target` does not exist
/// * `MapError::InvalidParent` if an item's parent does not come before it; nothing is
///   added in that case
pub fn paste(
    map: &mut MindMap,
    target: NodeId,
    items: &[OutlineItem],
) -> Result<Vec<NodeId>, MapError> {
    let target_node = map.node(target).ok_or(MapError::NodeNotFound(target))?;
    // Check everything first so a bad outline adds nothing.
    for (index, item) in items.iter().enumerate() {
        if let Some(parent) = item.parent.filter(|&parent| parent >= index) {
            return Err(MapError::InvalidParent { index, parent });
        }
    }
    if target_node.is_collapsed() {
        map.set_collapsed(target, false)?;
    }

    let mut ids = Vec::with_capacity(items.len());
    let mut branches = Vec::new();
    for item in items {
        let parent = item.parent.map_or(target, |parent| ids[parent]);
        let id = map.add_child(parent, &item.text)?;
        if item.link.is_some() {
            map.set_link(id, item.link.as_deref())?;
        }
        if item.parent.is_none() {
            branches.push(id);
        }
        ids.push(id);
    }
    for &branch in &branches {
        layout::tidy_branch(map, branch)?;
    }
    Ok(branches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Point;

    fn texts(map: &MindMap, ids: &[NodeId]) -> Vec<String> {
        ids.iter()
            .map(|&id| map.node(id).expect("node").text().to_owned())
            .collect()
    }

    #[test]
    fn outline_nests_descendants() {
        let mut map = MindMap::new("root");
        let goals = map.add_child(map.root(), "Goals").expect("root");
        let ship = map.add_child(goals, "Ship it").expect("goals");
        map.add_child(ship, "Line one\nline two").expect("ship");

        assert_eq!(
            branches_outline(&map, &[goals]),
            "- Goals\n  - Ship it\n    - Line one line two\n"
        );
    }

    #[test]
    fn parse_reads_bullets_numbers_tabs_and_headings() {
        let text = "# Project\n\n## Goals\n- First\n\t* Nested with a tab\n2) Second\n# Other";

        let items = parse_outline(text);

        let expected = [
            (None, "Project"),
            (Some(0), "Goals"),
            (Some(1), "First"),
            (Some(2), "Nested with a tab"),
            (Some(1), "Second"),
            (None, "Other"),
        ];
        let actual: Vec<(Option<usize>, &str)> = items
            .iter()
            .map(|item| (item.parent, item.text.as_str()))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn links_survive_copy_and_paste() {
        let mut map = MindMap::new("root");
        let docs = map.add_child(map.root(), "Docs").expect("root");
        map.set_link(docs, Some("https://example.com/a(b)"))
            .expect("docs");
        let outline = branches_outline(&map, &[docs]);
        assert_eq!(outline, "- [Docs](https://example.com/a(b))\n");

        let items = parse_outline(&outline);
        assert_eq!(items[0].text, "Docs");
        assert_eq!(items[0].link.as_deref(), Some("https://example.com/a(b)"));
        let root = map.root();
        let pasted = paste(&mut map, root, &items).expect("valid");

        assert_eq!(
            map.node(pasted[0]).expect("copy").link(),
            Some("https://example.com/a(b)")
        );
        assert_eq!(split_link("[not a link"), ("[not a link", None));
        assert_eq!(split_link("[](x)"), ("[](x)", None));
    }

    #[test]
    fn plain_lines_and_blank_input() {
        assert_eq!(parse_outline("  \n\n"), Vec::new());
        let items = parse_outline("alpha\n  beta\ngamma");

        assert_eq!(items[1].parent, Some(0));
        assert_eq!(items[2].parent, None);
    }

    #[test]
    fn copy_then_paste_reproduces_the_branch() {
        let mut map = MindMap::new("root");
        let goals = map.add_child(map.root(), "Goals").expect("root");
        map.add_child(goals, "Ship it").expect("goals");
        map.add_child(goals, "Grow").expect("goals");
        let target = map.add_child(map.root(), "Target").expect("root");

        let items = parse_outline(&branches_outline(&map, &[goals]));

        let pasted = paste(&mut map, target, &items).expect("valid outline");

        assert_eq!(pasted.len(), 1);
        assert_eq!(
            branches_outline(&map, &[pasted[0]]),
            branches_outline(&map, &[goals])
        );
        assert_eq!(map.node(pasted[0]).expect("copy").parent(), Some(target));
    }

    #[test]
    fn paste_unfolds_target_and_lays_out_new_nodes() {
        let mut map = MindMap::new("root");
        let target = map
            .add_child_at(map.root(), "Target", Point::new(-200.0, 0.0))
            .expect("root");
        map.add_child(target, "existing").expect("target");
        map.set_collapsed(target, true).expect("target");

        let pasted = paste(&mut map, target, &parse_outline("- a\n  - b\n- c")).expect("valid");

        assert!(!map.node(target).expect("target").is_collapsed());
        assert_eq!(texts(&map, &pasted), ["a", "c"]);
        // The target is on the left, so pasted grandchildren grow further left.
        let a = map.node(pasted[0]).expect("a");
        let b = map.node(a.children()[0]).expect("b");
        assert!(b.position().x < a.position().x);
    }

    #[test]
    fn invalid_items_add_nothing() {
        let mut map = MindMap::new("root");
        let items = [OutlineItem {
            parent: Some(0),
            text: "self-parent".to_owned(),
            link: None,
        }];

        let root = map.root();

        let result = paste(&mut map, root, &items);

        assert_eq!(
            result,
            Err(MapError::InvalidParent {
                index: 0,
                parent: 0
            })
        );
        assert_eq!(map.len(), 1);
    }
}
