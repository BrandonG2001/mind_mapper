//! Maps in other apps' formats: OPML outlines and FreeMind (`.mm`) mind maps.
//!
//! Both are XML trees with one element per node. Importing keeps labels, notes and links,
//! plus colours, folded branches, sides and cross-links (arrow links) from FreeMind. Neither format stores positions
//! the way this app does, so imported maps are arranged by Tidy up; exports carry the same
//! details back out.

pub mod freemind;
pub mod opml;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use roxmltree::Node as XmlNode;
use thiserror::Error;

use crate::layout::{TreeBuilder, balanced_left};
use crate::model::{MapError, MindMap, Node, NodeColor, NodeId};

/// Errors that can occur while importing a file.
#[derive(Debug, Error)]
pub enum ImportError {
    /// The file could not be read.
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not well-formed XML.
    #[error("the file is not valid XML: {0}")]
    Xml(#[from] roxmltree::Error),
    /// The XML is not laid out the way the format requires.
    #[error("not a valid {format} file: {problem}")]
    Invalid {
        /// Name of the format, e.g. "OPML".
        format: &'static str,
        /// What is missing or wrong.
        problem: &'static str,
    },
    /// Building the map failed (an internal error).
    #[error(transparent)]
    Map(#[from] MapError),
}

/// A format maps can be imported from and exported to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// An OPML outline (`.opml`), which most outliners read and write.
    Opml,
    /// A FreeMind mind map (`.mm`), also read by Freeplane and other mind-mapping apps.
    FreeMind,
}

impl Format {
    /// The format named by `path`'s extension (case-insensitive), if any.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "opml" => Some(Self::Opml),
            "mm" => Some(Self::FreeMind),
            _ => None,
        }
    }
}

/// Read the file at `path` as a map in `format`.
///
/// # Errors
///
/// * `ImportError::Io` if the file cannot be read (or is not UTF-8 text)
/// * `ImportError::Xml` or `ImportError::Invalid` if it is not a valid file of `format`
pub fn import(path: &Path, format: Format) -> Result<MindMap, ImportError> {
    let text = fs::read_to_string(path)?;
    // A byte-order mark some editors add is not part of the XML.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match format {
        Format::Opml => opml::read(text),
        Format::FreeMind => freemind::read(text),
    }
}

/// Everything an imported node can carry.
#[derive(Debug, Clone, Default, PartialEq)]
struct NodeFields {
    text: String,
    note: String,
    link: Option<String>,
    color: Option<NodeColor>,
    folded: bool,
    /// For a branch of the root: `Some(true)` if the file puts it on the left.
    left: Option<bool>,
    /// The node's identifier in the file, which cross-links refer to.
    file_id: Option<String>,
    /// The cross-links this node starts: the target's identifier and the link's label.
    links_to: Vec<(String, String)>,
}

/// The child elements of `parent` named `tag`, in document order.
fn elements<'a, 'input>(
    parent: XmlNode<'a, 'input>,
    tag: &'static str,
) -> impl Iterator<Item = XmlNode<'a, 'input>> {
    parent
        .children()
        .filter(move |child| child.has_tag_name(tag))
}

/// Build a map whose central idea is `root`, and whose branches are the `tag` elements
/// under `branches` (nested `tag` elements becoming deeper nodes), each read by `read`.
fn build_map(
    root: &NodeFields,
    branches: XmlNode<'_, '_>,
    tag: &'static str,
    read: impl Fn(XmlNode<'_, '_>) -> NodeFields,
) -> Result<MindMap, MapError> {
    let mut builder = TreeBuilder::new(&root.text);
    let root_id = builder.root();
    apply(builder.map_mut(), root_id, root)?;
    // Cross-links can point forward in the file, so they are joined up at the end.
    let mut ids: HashMap<String, NodeId> = HashMap::new();
    let mut pending: Vec<(NodeId, String, String)> = Vec::new();
    remember_links(&mut ids, &mut pending, root_id, root);
    let top: Vec<XmlNode<'_, '_>> = elements(branches, tag).collect();
    // An explicit stack instead of recursion, so deeply nested files cannot overflow the
    // call stack. Each entry: element, parent node, side if the file does not say.
    let mut stack: Vec<(XmlNode<'_, '_>, NodeId, bool)> = top
        .iter()
        .enumerate()
        .rev()
        .map(|(index, &element)| (element, root_id, balanced_left(index, top.len())))
        .collect();
    while let Some((element, parent, default_left)) = stack.pop() {
        let fields = read(element);
        let left = fields.left.unwrap_or(default_left);
        let id = builder.add(parent, &fields.text, left)?;
        apply(builder.map_mut(), id, &fields)?;
        remember_links(&mut ids, &mut pending, id, &fields);
        // Reversed, so the first child is taken off the stack (and added) first.
        let children: Vec<XmlNode<'_, '_>> = elements(element, tag).collect();
        stack.extend(children.into_iter().rev().map(|child| (child, id, false)));
    }
    for (from, target, label) in pending {
        if let Some(&to) = ids.get(&target) {
            // Links a file repeats or points from a node to itself are left out.
            match builder.map_mut().add_cross_link(from, to) {
                Ok(()) => builder.map_mut().set_cross_link_label(from, to, &label)?,
                Err(MapError::DuplicateLink | MapError::LinkToSelf) => {}
                Err(err) => return Err(err),
            }
        }
    }
    builder.finish()
}

/// Note node `id`'s file identifier, and the cross-links it starts, for [`build_map`].
fn remember_links(
    ids: &mut HashMap<String, NodeId>,
    pending: &mut Vec<(NodeId, String, String)>,
    id: NodeId,
    fields: &NodeFields,
) {
    if let Some(file_id) = &fields.file_id {
        ids.insert(file_id.clone(), id);
    }
    pending.extend(
        fields
            .links_to
            .iter()
            .map(|(target, label)| (id, target.clone(), label.clone())),
    );
}

/// Give node `id` the details in `fields` (its label is set when it is created).
fn apply(map: &mut MindMap, id: NodeId, fields: &NodeFields) -> Result<(), MapError> {
    if !fields.note.is_empty() {
        map.set_note(id, &fields.note)?;
    }
    map.set_link(id, fields.link.as_deref())?;
    map.set_color(id, fields.color)?;
    // The central idea cannot be folded.
    if fields.folded && id != map.root() {
        map.set_collapsed(id, true)?;
    }
    Ok(())
}

/// A label for a node whose file gives it none.
fn label_or_default(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        "Untitled".to_owned()
    } else {
        text.to_owned()
    }
}

/// How one format writes a node as an XML element.
trait Dialect {
    /// Name of a node's element.
    const TAG: &'static str;

    /// Write the element's attributes, each with a leading space (see [`push_attribute`]).
    fn attributes(&self, map: &MindMap, id: NodeId, node: &Node, out: &mut String);

    /// Whether [`Dialect::content`] writes anything for node `id`.
    fn has_content(&self, _map: &MindMap, _id: NodeId, _node: &Node) -> bool {
        false
    }

    /// Write what goes inside node `id`'s element before its children, as lines indented
    /// by `indent` levels.
    fn content(&self, _map: &MindMap, _id: NodeId, _indent: usize, _out: &mut String) {}
}

/// Write the branch at `start` as nested `D::TAG` elements, one per line, the outermost
/// indented by `indent` levels.
fn write_tree<D: Dialect>(
    dialect: &D,
    map: &MindMap,
    start: NodeId,
    indent: usize,
    out: &mut String,
) {
    /// One step of the walk: open a node's element, or close one opened earlier.
    enum Step {
        Open(NodeId, usize),
        Close(usize),
    }
    // An explicit stack, so a very deep map cannot overflow the call stack.
    let mut stack = vec![Step::Open(start, indent)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Close(depth) => {
                push_indent(out, depth);
                out.push_str("</");
                out.push_str(D::TAG);
                out.push_str(">\n");
            }
            Step::Open(id, depth) => {
                let Some(node) = map.node(id) else {
                    continue;
                };
                push_indent(out, depth);
                out.push('<');
                out.push_str(D::TAG);
                dialect.attributes(map, id, node, out);
                if node.children().is_empty() && !dialect.has_content(map, id, node) {
                    out.push_str("/>\n");
                    continue;
                }
                out.push_str(">\n");
                dialect.content(map, id, depth + 1, out);
                stack.push(Step::Close(depth));
                stack.extend(
                    node.children()
                        .iter()
                        .rev()
                        .map(|&child| Step::Open(child, depth + 1)),
                );
            }
        }
    }
}

/// Two spaces per level.
fn push_indent(out: &mut String, depth: usize) {
    out.extend(std::iter::repeat_n("  ", depth));
}

/// Append ` name="value"`, escaping `value`.
fn push_attribute(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_escaped(out, value);
    out.push('"');
}

/// Append `text` with the characters XML gives a meaning to escaped. Line breaks and tabs
/// become character references so they survive inside attributes; other control
/// characters, which XML 1.0 does not allow at all, are dropped.
pub(crate) fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            control if control.is_control() => {}
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn formats_come_from_the_extension() {
        assert_eq!(
            Format::from_path(Path::new("a/ideas.OPML")),
            Some(Format::Opml)
        );
        assert_eq!(
            Format::from_path(Path::new("ideas.mm")),
            Some(Format::FreeMind)
        );
        assert_eq!(Format::from_path(Path::new("ideas.json")), None);
        assert_eq!(Format::from_path(Path::new("ideas")), None);
    }

    #[test]
    fn escaping_keeps_text_and_line_breaks() {
        let mut out = String::new();
        push_attribute(&mut out, "text", "a & <b> \"c\"\nd\u{7}");
        assert_eq!(out, " text=\"a &amp; &lt;b&gt; &quot;c&quot;&#10;d\"");
    }

    #[test]
    fn import_reads_files_and_skips_a_byte_order_mark() {
        let dir = TempDir::new("formats_import");
        let path = dir.path().join("outline.opml");
        fs::write(
            &path,
            "\u{feff}<opml version=\"2.0\"><body><outline text=\"Only\"/></body></opml>",
        )
        .expect("writable");

        let map = import(&path, Format::Opml).expect("imports");

        assert_eq!(map.node(map.root()).expect("root").text(), "Only");
        assert!(matches!(
            import(&dir.path().join("missing.mm"), Format::FreeMind),
            Err(ImportError::Io(_))
        ));
    }
}
