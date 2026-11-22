//! OPML outlines: `<opml><head><title/></head><body><outline text="..."/></body></opml>`.
//!
//! Each `<outline>` is a node: `text` is its label, `_note` its notes (the attribute
//! OmniOutliner, Workflowy and others use) and `url` (or `htmlUrl`) its link. An outline
//! with a single top-level item, as mind-mapping apps write, becomes a map with that item
//! as the central idea; with several, the document title is the central idea.

use roxmltree::{Document, Node as XmlNode};

use super::{
    Dialect, ImportError, NodeFields, build_map, elements, label_or_default, push_attribute,
    push_escaped, write_tree,
};
use crate::model::{MindMap, Node, NodeId};

/// Format name used in error messages.
const FORMAT: &str = "OPML";
/// Central idea for an outline with several top-level items and no title.
const DEFAULT_TITLE: &str = "Imported outline";

/// Read an OPML document.
///
/// # Errors
///
/// * `ImportError::Xml` if `text` is not well-formed XML
/// * `ImportError::Invalid` if the top element is not `<opml>` or has no `<body>`
pub fn read(text: &str) -> Result<MindMap, ImportError> {
    let document = Document::parse(text)?;
    let opml = document.root_element();
    if !opml.has_tag_name("opml") {
        return Err(invalid("the top element is not <opml>"));
    }
    let body = elements(opml, "body")
        .next()
        .ok_or_else(|| invalid("it has no <body>"))?;
    let top: Vec<XmlNode<'_, '_>> = elements(body, "outline").collect();
    let map = if let [single] = top.as_slice() {
        build_map(&read_outline(*single), *single, "outline", read_outline)?
    } else {
        let title = elements(opml, "head")
            .flat_map(|head| elements(head, "title"))
            .find_map(|title| title.text())
            .map_or_else(|| DEFAULT_TITLE.to_owned(), label_or_default);
        let root = NodeFields {
            text: title,
            ..NodeFields::default()
        };
        build_map(&root, body, "outline", read_outline)?
    };
    Ok(map)
}

/// Write `map` as an OPML document with the central idea as its single top-level item.
#[must_use]
pub fn write(map: &MindMap) -> String {
    let root = map.root();
    let title = map.node(root).map_or("", Node::text);
    let mut out =
        String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n");
    out.push_str("  <head>\n    <title>");
    push_escaped(&mut out, title);
    out.push_str("</title>\n  </head>\n  <body>\n");
    write_tree(&Opml, map, root, 2, &mut out);
    out.push_str("  </body>\n</opml>\n");
    out
}

fn invalid(problem: &'static str) -> ImportError {
    ImportError::Invalid {
        format: FORMAT,
        problem,
    }
}

/// The node an `<outline>` element describes.
fn read_outline(outline: XmlNode<'_, '_>) -> NodeFields {
    NodeFields {
        text: label_or_default(
            outline
                .attribute("text")
                .or_else(|| outline.attribute("title"))
                .unwrap_or_default(),
        ),
        note: outline
            .attribute("_note")
            .unwrap_or_default()
            .trim()
            .to_owned(),
        link: outline
            .attribute("url")
            .or_else(|| outline.attribute("htmlUrl"))
            .map(str::trim)
            .filter(|link| !link.is_empty())
            .map(str::to_owned),
        ..NodeFields::default()
    }
}

/// Writes `<outline>` elements.
struct Opml;

impl Dialect for Opml {
    const TAG: &'static str = "outline";

    fn attributes(&self, _map: &MindMap, _id: NodeId, node: &Node, out: &mut String) {
        push_attribute(out, "text", node.text());
        if !node.note().is_empty() {
            push_attribute(out, "_note", node.note());
        }
        if let Some(link) = node.link() {
            push_attribute(out, "url", link);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(map: &MindMap, id: NodeId) -> Vec<&str> {
        map.node(id)
            .expect("live")
            .children()
            .iter()
            .map(|&child| map.node(child).expect("live").text())
            .collect()
    }

    #[test]
    fn a_single_top_item_becomes_the_central_idea() {
        let map = read(
            r#"<?xml version="1.0"?>
            <opml version="2.0"><head><title>Ignored</title></head><body>
              <outline text="Plan">
                <outline text="Goals" _note="Why we do it" url="https://example.com">
                  <outline text="Ship"/>
                </outline>
                <outline title="Risks"/>
              </outline>
            </body></opml>"#,
        )
        .expect("valid");

        let root = map.root();
        assert_eq!(map.node(root).expect("root").text(), "Plan");
        assert_eq!(labels(&map, root), ["Goals", "Risks"]);
        let goals = map.node(root).expect("root").children()[0];
        let goals_node = map.node(goals).expect("goals");
        assert_eq!(goals_node.note(), "Why we do it");
        assert_eq!(goals_node.link(), Some("https://example.com"));
        assert_eq!(labels(&map, goals), ["Ship"]);
    }

    #[test]
    fn several_top_items_hang_off_the_title() {
        let map = read(
            "<opml><head><title>Week</title></head><body>\
             <outline text=\"Mon\"/><outline text=\"Tue\"/><outline text=\"\"/></body></opml>",
        )
        .expect("valid");

        assert_eq!(map.node(map.root()).expect("root").text(), "Week");
        assert_eq!(labels(&map, map.root()), ["Mon", "Tue", "Untitled"]);
        // Spread over both sides like a template.
        let sides: Vec<bool> = map
            .node(map.root())
            .expect("root")
            .children()
            .iter()
            .map(|&id| map.node(id).expect("live").position().x < 0.0)
            .collect();
        assert_eq!(sides, [false, false, true]);
    }

    #[test]
    fn writing_then_reading_keeps_labels_notes_and_links() {
        let mut map = MindMap::new("Trip & \"plans\"");
        let root = map.root();
        let day = map.add_child(root, "Day 1\nmorning").expect("root");
        map.set_note(day, "Pack <early>").expect("day");
        map.set_link(day, Some("https://example.com/?a=1&b=2"))
            .expect("day");
        map.add_child(day, "Museum").expect("day");
        map.add_child(root, "Day 2").expect("root");

        let text = write(&map);
        let read_back = read(&text).expect("own output is valid");

        assert_eq!(
            labels(&read_back, read_back.root()),
            ["Day 1\nmorning", "Day 2"]
        );
        let day_back = read_back.node(read_back.root()).expect("root").children()[0];
        let day_node = read_back.node(day_back).expect("day");
        assert_eq!(day_node.note(), "Pack <early>");
        assert_eq!(day_node.link(), Some("https://example.com/?a=1&b=2"));
        assert_eq!(labels(&read_back, day_back), ["Museum"]);
        assert_eq!(
            read_back.node(read_back.root()).expect("root").text(),
            "Trip & \"plans\""
        );
    }

    #[test]
    fn other_documents_are_rejected() {
        assert!(matches!(read("<html/>"), Err(ImportError::Invalid { .. })));
        assert!(matches!(read("<opml/>"), Err(ImportError::Invalid { .. })));
        assert!(matches!(read("<opml"), Err(ImportError::Xml(_))));
    }

    #[test]
    fn very_deep_outlines_do_not_overflow_the_stack() {
        let depth = 5_000;
        let text = format!(
            "<opml><body>{}{}</body></opml>",
            "<outline text=\"level\">".repeat(depth),
            "</outline>".repeat(depth)
        );

        let map = read(&text).expect("valid");

        assert_eq!(map.len(), depth);
        assert!(write(&map).len() > depth * 20);
    }
}
