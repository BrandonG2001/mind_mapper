//! FreeMind mind maps (`.mm`), also read and written by Freeplane and others:
//! `<map version="1.0.1"><node TEXT="root"><node TEXT="..." POSITION="left"/></node></map>`.
//!
//! Each `<node>` carries its label in `TEXT` (or, for formatted labels, an HTML
//! `<richcontent TYPE="NODE">`), its notes in `<richcontent TYPE="NOTE">`, its link in
//! `LINK`, its colour in `COLOR`, its folded state in `FOLDED`, for branches of the
//! root its side in `POSITION`, and cross-links as `<arrowlink DESTINATION="..."/>`
//! naming another node's `ID`.

use roxmltree::{Document, Node as XmlNode};

use super::{
    Dialect, ImportError, NodeFields, build_map, elements, label_or_default, push_attribute,
    push_escaped, push_indent, write_tree,
};
use crate::model::{MindMap, Node, NodeColor, NodeId};

/// Format name used in error messages.
const FORMAT: &str = "FreeMind";
/// HTML elements that start a new line in rich text.
const BLOCK_TAGS: [&str; 10] = ["p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5"];

/// Read a FreeMind document.
///
/// # Errors
///
/// * `ImportError::Xml` if `text` is not well-formed XML
/// * `ImportError::Invalid` if the top element is not `<map>` or holds no `<node>`
pub fn read(text: &str) -> Result<MindMap, ImportError> {
    let document = Document::parse(text)?;
    let map = document.root_element();
    if !map.has_tag_name("map") {
        return Err(invalid("the top element is not <map>"));
    }
    let root = elements(map, "node")
        .next()
        .ok_or_else(|| invalid("it has no <node>"))?;
    Ok(build_map(&read_node(root), root, "node", read_node)?)
}

/// Write `map` as a FreeMind document.
#[must_use]
pub fn write(map: &MindMap) -> String {
    let root = map.root();
    let root_x = map.node(root).map_or(0.0, |node| node.position().x);
    let mut out = String::from("<map version=\"1.0.1\">\n");
    write_tree(&FreeMind { root, root_x }, map, root, 1, &mut out);
    out.push_str("</map>\n");
    out
}

fn invalid(problem: &'static str) -> ImportError {
    ImportError::Invalid {
        format: FORMAT,
        problem,
    }
}

/// The node a `<node>` element describes.
fn read_node(element: XmlNode<'_, '_>) -> NodeFields {
    let rich = |kind: &str| {
        elements(element, "richcontent")
            .find(|content| content.attribute("TYPE") == Some(kind))
            .map(html_text)
    };
    let text = element
        .attribute("TEXT")
        .map(str::to_owned)
        .or_else(|| rich("NODE"))
        .unwrap_or_default();
    NodeFields {
        text: label_or_default(&text),
        note: rich("NOTE").unwrap_or_default(),
        link: element
            .attribute("LINK")
            .map(str::trim)
            .filter(|link| !link.is_empty())
            .map(str::to_owned),
        color: element
            .attribute("COLOR")
            .and_then(|color| NodeColor::try_from(color.to_owned()).ok()),
        folded: element.attribute("FOLDED") == Some("true"),
        left: element.attribute("POSITION").map(|side| side == "left"),
        file_id: element.attribute("ID").map(str::to_owned),
        links_to: elements(element, "arrowlink")
            .filter_map(|arrow| arrow.attribute("DESTINATION"))
            .map(str::to_owned)
            .collect(),
    }
}

/// The `ID` written for node `id`, which arrow links refer to.
fn file_id(id: NodeId) -> String {
    format!("ID_{}", id.index())
}

/// The text of the HTML inside `content`: one line per paragraph (or line break, list
/// item...), runs of spaces collapsed as a browser would show them.
fn html_text(content: XmlNode<'_, '_>) -> String {
    let mut out = String::new();
    for node in content.descendants() {
        if node.is_element() && BLOCK_TAGS.iter().any(|&tag| node.has_tag_name(tag)) {
            out.push('\n');
        } else if node.is_text()
            && !node
                .ancestors()
                .any(|ancestor| ancestor.has_tag_name("head"))
            && let Some(text) = node.text()
        {
            // `split_whitespace` drops the indentation and line breaks of the HTML source.
            for word in text.split_whitespace() {
                if !out.is_empty() && !out.ends_with([' ', '\n']) {
                    out.push(' ');
                }
                out.push_str(word);
            }
        }
    }
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Writes `<node>` elements.
struct FreeMind {
    root: NodeId,
    /// Where the central idea is; branches left of it are written as `POSITION="left"`.
    root_x: f32,
}

impl Dialect for FreeMind {
    const TAG: &'static str = "node";

    fn attributes(&self, _map: &MindMap, id: NodeId, node: &Node, out: &mut String) {
        push_attribute(out, "ID", &file_id(id));
        push_attribute(out, "TEXT", node.text());
        if node.parent() == Some(self.root) {
            let side = if node.position().x < self.root_x {
                "left"
            } else {
                "right"
            };
            push_attribute(out, "POSITION", side);
        }
        if node.is_collapsed() && !node.children().is_empty() {
            push_attribute(out, "FOLDED", "true");
        }
        if let Some(color) = node.color() {
            push_attribute(out, "COLOR", &String::from(color));
        }
        if let Some(link) = node.link() {
            push_attribute(out, "LINK", link);
        }
    }

    fn has_content(&self, map: &MindMap, id: NodeId, node: &Node) -> bool {
        !node.note().is_empty() || map.cross_links().iter().any(|link| link.from() == id)
    }

    fn content(&self, map: &MindMap, id: NodeId, indent: usize, out: &mut String) {
        let note = map.node(id).map_or("", Node::note);
        if !note.is_empty() {
            push_indent(out, indent);
            out.push_str("<richcontent TYPE=\"NOTE\"><html><head></head><body>");
            for line in note.lines() {
                out.push_str("<p>");
                push_escaped(out, line);
                out.push_str("</p>");
            }
            out.push_str("</body></html></richcontent>\n");
        }
        for link in map.cross_links().iter().filter(|link| link.from() == id) {
            push_indent(out, indent);
            out.push_str("<arrowlink");
            push_attribute(out, "DESTINATION", &file_id(link.to()));
            out.push_str("/>\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(map: &MindMap, parent: NodeId, index: usize) -> NodeId {
        map.node(parent).expect("live").children()[index]
    }

    #[test]
    fn reads_what_freemind_writes() {
        let map = read(
            r##"<map version="1.0.1">
            <node TEXT="Project" ID="ID_1">
              <node TEXT="Goals" POSITION="right" FOLDED="true" COLOR="#e15759"
                    LINK="https://example.com">
                <node TEXT="Ship it"/>
                <richcontent TYPE="NOTE"><html>
                  <head><style>p { margin: 0 }</style></head>
                  <body><p>First   line</p><p>Second <b>line</b></p></body>
                </html></richcontent>
              </node>
              <node POSITION="left">
                <richcontent TYPE="NODE"><html><body><p>Rich label</p></body></html></richcontent>
              </node>
            </node>
            </map>"##,
        )
        .expect("valid");

        let root = map.root();
        assert_eq!(map.node(root).expect("root").text(), "Project");
        let goals = map.node(child(&map, root, 0)).expect("goals");
        assert_eq!(goals.text(), "Goals");
        assert!(goals.is_collapsed());
        assert_eq!(goals.color().map(String::from).as_deref(), Some("#e15759"));
        assert_eq!(goals.link(), Some("https://example.com"));
        assert_eq!(goals.note(), "First line\nSecond line");
        assert_eq!(goals.children().len(), 1);
        let rich = map.node(child(&map, root, 1)).expect("rich");
        assert_eq!(rich.text(), "Rich label");
        assert!(goals.position().x > 0.0 && rich.position().x < 0.0);
    }

    #[test]
    fn writing_then_reading_keeps_every_detail() {
        let mut map = MindMap::new("Root <1>");
        let root = map.root();
        let right = map
            .add_child_at(root, "Right", crate::model::Point::new(200.0, 0.0))
            .expect("root");
        let left = map
            .add_child_at(root, "Left", crate::model::Point::new(-200.0, 0.0))
            .expect("root");
        map.add_child(right, "Hidden").expect("right");
        map.set_collapsed(right, true).expect("right");
        map.set_color(left, Some(crate::details::PALETTE[1]))
            .expect("left");
        map.set_note(left, "Line one\nLine & two").expect("left");
        map.set_link(left, Some("https://example.com"))
            .expect("left");

        let read_back = read(&write(&map)).expect("own output is valid");

        let root_back = read_back.root();
        assert_eq!(read_back.node(root_back).expect("root").text(), "Root <1>");
        let right_back = read_back
            .node(child(&read_back, root_back, 0))
            .expect("right");
        assert_eq!(right_back.text(), "Right");
        assert!(right_back.is_collapsed() && right_back.position().x > 0.0);
        let left_back = read_back
            .node(child(&read_back, root_back, 1))
            .expect("left");
        assert!(left_back.position().x < 0.0);
        assert_eq!(left_back.color(), Some(crate::details::PALETTE[1]));
        assert_eq!(left_back.note(), "Line one\nLine & two");
        assert_eq!(left_back.link(), Some("https://example.com"));
        assert_eq!(read_back.len(), map.len());
    }

    #[test]
    fn arrow_links_become_cross_links_and_back() {
        let map = read(
            r#"<map><node TEXT="root" ID="a">
              <node TEXT="x" ID="b">
                <arrowlink DESTINATION="c"/>
                <arrowlink DESTINATION="b"/>
                <arrowlink DESTINATION="missing"/>
              </node>
              <node TEXT="y" ID="c"><arrowlink DESTINATION="b"/></node>
            </node></map>"#,
        )
        .expect("valid");
        let text_of = |map: &MindMap, id| map.node(id).expect("live").text().to_owned();

        // The link points forward in the file; a link to itself, one to a missing node
        // and the same link back again are left out.
        assert_eq!(map.cross_links().len(), 1);
        let link = map.cross_links()[0];
        assert_eq!(
            (text_of(&map, link.from()), text_of(&map, link.to())),
            ("x".into(), "y".into())
        );

        let back = read(&write(&map)).expect("own output is valid");
        let link = back.cross_links()[0];
        assert_eq!(back.cross_links().len(), 1);
        assert_eq!(
            (text_of(&back, link.from()), text_of(&back, link.to())),
            ("x".into(), "y".into())
        );
    }

    #[test]
    fn other_documents_are_rejected() {
        assert!(matches!(read("<opml/>"), Err(ImportError::Invalid { .. })));
        assert!(matches!(read("<map/>"), Err(ImportError::Invalid { .. })));
        assert!(matches!(read("<map><node"), Err(ImportError::Xml(_))));
    }
}
