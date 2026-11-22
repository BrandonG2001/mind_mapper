//! Core mind-map data model: a tree of text nodes with 2D positions.
//!
//! Nodes live in a slab (`Vec<Option<Node>>`) so lookups by [`NodeId`] are O(1) array
//! indexing, and the iteration order is deterministic, which keeps drawing and hit
//! testing stable from frame to frame. Freed slots are reused through a free list.

use serde::{Deserialize, Serialize};

use crate::history::HeapSize;
use crate::picture::Picture;
use thiserror::Error;

/// Horizontal distance between a parent and a newly created child.
const CHILD_OFFSET_X: f32 = 220.0;
/// Vertical distance between consecutive siblings that are created automatically.
const SIBLING_OFFSET_Y: f32 = 70.0;

/// Handle that identifies one node inside a [`MindMap`].
///
/// This is a "newtype" wrapper: it is a `usize` at runtime, but the compiler will not let
/// you mix it up with any other integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(usize);

impl NodeId {
    /// Position of the node's slot, for per-node lookup tables sized by
    /// [`MindMap::slot_count`].
    #[must_use]
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// A 2D point in world (map) coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate; grows to the right.
    pub x: f32,
    /// Vertical coordinate; grows downwards.
    pub y: f32,
}

impl Point {
    /// Create a point from its coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// One idea in the mind map.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    text: String,
    position: Point,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    /// When true, the node's descendants are hidden.
    collapsed: bool,
    /// Colour chosen for this node (and, unless they choose their own, its descendants).
    color: Option<NodeColor>,
    /// Free-form notes; empty when there are none.
    note: String,
    /// A web address attached to the node.
    link: Option<String>,
    /// A picture shown above the label.
    picture: Option<Picture>,
    /// How the label is written: bold, italic, size.
    style: NodeStyle,
}

impl Node {
    /// A node with only a label, position and parent; every optional detail empty.
    fn new(text: String, position: Point, parent: Option<NodeId>) -> Self {
        Self {
            text,
            position,
            parent,
            children: Vec::new(),
            collapsed: false,
            color: None,
            note: String::new(),
            link: None,
            picture: None,
            style: NodeStyle::default(),
        }
    }
}

/// How big a label is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    /// The usual size.
    #[default]
    Normal,
    /// About a third bigger.
    Large,
    /// About two thirds bigger.
    Larger,
}

impl TextSize {
    /// Every size, smallest first.
    pub const ALL: [Self; 3] = [Self::Normal, Self::Large, Self::Larger];

    /// The font size relative to a normal label.
    #[must_use]
    pub const fn scale(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Large => 1.3,
            Self::Larger => 1.65,
        }
    }

    /// The size's name, as shown in menus.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Large => "Large",
            Self::Larger => "Larger",
        }
    }

    /// `serde` helper: skip writing the usual size.
    // serde passes the field by reference, so this cannot take `self` by value.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    const fn is_normal(&self) -> bool {
        matches!(self, Self::Normal)
    }
}

/// How a node's label is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
// Missing fields take their default, so a style with only `"bold": true` loads.
#[serde(default)]
pub struct NodeStyle {
    /// Written in the bold font.
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    /// Written slanted.
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    /// How big.
    #[serde(skip_serializing_if = "TextSize::is_normal")]
    pub size: TextSize,
}

/// How much wider bold text is than regular text (measured: 7% to 10%).
const BOLD_WIDTH: f32 = 1.1;

impl NodeStyle {
    /// Whether this is the usual style (nothing to save).
    // serde passes the field by reference, so this cannot take `self` by value.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    #[must_use]
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// The font size relative to a normal label.
    #[must_use]
    pub const fn font_scale(self) -> f32 {
        self.size.scale()
    }

    /// How much wider the label is than a normal one with the same text.
    #[must_use]
    pub fn width_scale(self) -> f32 {
        self.size.scale() * if self.bold { BOLD_WIDTH } else { 1.0 }
    }
}

/// An RGB colour chosen for a node, stored in files as `"#rrggbb"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
// serde converts through `String`, using the `TryFrom`/`From` impls below.
#[serde(try_from = "String", into = "String")]
pub struct NodeColor {
    r: u8,
    g: u8,
    b: u8,
}

impl NodeColor {
    /// A colour from its red, green and blue components.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// The red, green and blue components.
    #[must_use]
    pub const fn components(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
}

impl TryFrom<String> for NodeColor {
    type Error = String;

    /// Parse `"#rrggbb"` (case-insensitive).
    fn try_from(text: String) -> Result<Self, Self::Error> {
        let invalid = || format!("invalid colour {text:?}; expected \"#rrggbb\"");
        let hex = text
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6)
            .ok_or_else(invalid)?;
        let channel = |at: usize| {
            hex.get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(invalid)
        };
        Ok(Self::rgb(channel(0)?, channel(2)?, channel(4)?))
    }
}

impl From<NodeColor> for String {
    fn from(color: NodeColor) -> Self {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    }
}

impl Node {
    /// The node's label.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Centre of the node in world coordinates.
    #[must_use]
    pub const fn position(&self) -> Point {
        self.position
    }

    /// The parent node, or `None` for the root.
    #[must_use]
    pub const fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Child nodes in creation order.
    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// Whether the node's descendants are hidden.
    #[must_use]
    pub const fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// The colour chosen for this node itself, if any (see [`MindMap::set_color`]).
    #[must_use]
    pub const fn color(&self) -> Option<NodeColor> {
        self.color
    }

    /// The node's notes; empty when there are none.
    #[must_use]
    pub fn note(&self) -> &str {
        &self.note
    }

    /// The web address attached to the node, if any.
    #[must_use]
    pub fn link(&self) -> Option<&str> {
        self.link.as_deref()
    }

    /// The picture shown above the label, if any.
    #[must_use]
    pub const fn picture(&self) -> Option<&Picture> {
        self.picture.as_ref()
    }

    /// How the label is written.
    #[must_use]
    pub const fn style(&self) -> NodeStyle {
        self.style
    }
}

/// Errors produced by operations on a [`MindMap`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MapError {
    /// The given id does not refer to a live node.
    #[error("node {0:?} does not exist")]
    NodeNotFound(NodeId),
    /// The root node cannot be deleted and has no siblings.
    #[error("the operation is not allowed on the root node")]
    RootNode,
    /// A list of records was empty.
    #[error("a mind map must contain at least one node")]
    Empty,
    /// The first record must be the root, which has no parent.
    #[error("the first node must be the root and must not have a parent")]
    InvalidRoot,
    /// A record pointed at a parent that does not come before it.
    #[error("node {index} refers to parent {parent}, which must appear earlier in the list")]
    InvalidParent {
        /// Position of the offending record.
        index: usize,
        /// The parent index it pointed to.
        parent: usize,
    },
    /// A node cannot be moved underneath itself or one of its own descendants.
    #[error("a node cannot be moved into its own branch")]
    WouldCreateCycle,
    /// A cross-link must join two different nodes.
    #[error("a node cannot be linked to itself")]
    LinkToSelf,
    /// The two nodes are already linked (in either direction).
    #[error("these nodes are already linked")]
    DuplicateLink,
    /// There is no such cross-link to remove.
    #[error("these nodes are not linked")]
    LinkNotFound,
    /// A node record's picture is not a valid PNG or JPEG data URL.
    #[error("the picture of node {index} is damaged")]
    InvalidPicture {
        /// Position of the offending node record.
        index: usize,
    },
    /// A link record points at a node that is not in the list, at itself, or repeats
    /// another link.
    #[error("link {index} is invalid: it must join two different, existing nodes once")]
    InvalidLink {
        /// Position of the offending link record.
        index: usize,
    },
    /// Two nodes that were expected to share a parent do not.
    #[error("only nodes with the same parent can swap places")]
    NotSiblings,
}

/// Flat, serializable form of a single node.
///
/// A whole map is a list of records in pre-order (every parent comes before its children),
/// and `parent` is the index of the parent record. That ordering rule makes cycles
/// impossible, so a file can be validated in one pass.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeRecord {
    /// Index of the parent record, or `None` for the root.
    pub parent: Option<usize>,
    /// Node label.
    pub text: String,
    /// Node centre in world coordinates.
    pub position: Point,
    /// Whether the node's descendants are hidden. Omitted from the file when false, and
    /// treated as false when absent, so files from older versions still load.
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    /// The node's own colour, if any. Optional like the fields below, so files from
    /// older versions still load and older versions ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<NodeColor>,
    /// Notes; omitted when empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Attached web address, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// The node's picture as a data URL (`data:image/png;base64,...`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    /// How the label is written; omitted for the usual style.
    #[serde(default, skip_serializing_if = "NodeStyle::is_plain")]
    pub style: NodeStyle,
}

/// Flat, serializable form of a cross-link: indices into the node records.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkRecord {
    /// Index of the node record the link starts at.
    pub from: usize,
    /// Index of the node record the link points to.
    pub to: usize,
    /// Text shown on the link; omitted from the file when empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// A whole map in flat form: nodes in pre-order, plus cross-links between them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapRecords {
    /// The nodes, parents before children.
    pub nodes: Vec<NodeRecord>,
    /// The cross-links, by node record index.
    pub links: Vec<LinkRecord>,
}

/// A relationship line between two nodes anywhere in the map, drawn as a dashed arrow
/// from `from` to `to`, optionally labelled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CrossLink {
    from: NodeId,
    to: NodeId,
    label: String,
}

impl CrossLink {
    /// The node the arrow starts at.
    #[must_use]
    pub const fn from(&self) -> NodeId {
        self.from
    }

    /// The node the arrow points to.
    #[must_use]
    pub const fn to(&self) -> NodeId {
        self.to
    }

    /// Text shown on the link; empty when it has none.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether the link joins `a` and `b`, in either direction.
    #[must_use]
    pub fn joins(&self, a: NodeId, b: NodeId) -> bool {
        (self.from, self.to) == (a, b) || (self.from, self.to) == (b, a)
    }

    /// Whether the link starts or ends at `id`.
    #[must_use]
    pub fn touches(&self, id: NodeId) -> bool {
        self.from == id || self.to == id
    }
}

/// `serde` helper: skip writing a `false` flag.
// serde passes the field by reference, so this cannot take `bool` by value.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(flag: &bool) -> bool {
    !*flag
}

impl HeapSize for MindMap {
    /// Slot table plus every node's text, notes, link and child list.
    fn heap_size(&self) -> usize {
        let slots = self.slots.capacity() * std::mem::size_of::<Option<Node>>()
            + self.free.capacity() * std::mem::size_of::<usize>()
            + self.links.capacity() * std::mem::size_of::<CrossLink>()
            + self
                .links
                .iter()
                .map(|link| link.label.capacity())
                .sum::<usize>();
        let nodes: usize = self
            .iter()
            .map(|(_, node)| {
                node.text.capacity()
                    + node.note.capacity()
                    + node.link.as_ref().map_or(0, String::capacity)
                    + node.picture.as_ref().map_or(0, Picture::heap_share)
                    + node.children.capacity() * std::mem::size_of::<NodeId>()
            })
            .sum();
        std::mem::size_of::<Self>() + slots + nodes
    }
}

/// A node visited by [`MindMap::preorder`].
#[derive(Debug, Clone, Copy)]
pub struct Visit<'a> {
    /// The visited node's id.
    pub id: NodeId,
    /// The visited node.
    pub node: &'a Node,
    /// Index of the root's child this node descends from (`None` for the root itself).
    /// The UI uses it to give every top-level branch its own colour.
    pub branch: Option<usize>,
}

/// A tree of ideas with a single root.
#[derive(Debug, Clone, PartialEq)]
pub struct MindMap {
    slots: Vec<Option<Node>>,
    free: Vec<usize>,
    root: NodeId,
    /// Relationship lines between nodes; both ends always exist.
    links: Vec<CrossLink>,
}

impl MindMap {
    /// Create a map that contains only a root node at the origin.
    ///
    /// # Arguments
    ///
    /// * `root_text` - Label of the central idea
    #[must_use]
    pub fn new(root_text: &str) -> Self {
        let root = Node::new(root_text.to_owned(), Point::default(), None);
        Self {
            slots: vec![Some(root)],
            free: Vec::new(),
            root: NodeId(0),
            links: Vec::new(),
        }
    }

    /// The root node's id.
    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// Number of live nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// Always `false`: a map always has its root node. Provided because clippy expects
    /// every type with a `len` method to also have `is_empty`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up a node.
    ///
    /// # Returns
    ///
    /// The node, or `None` if `id` was deleted or never existed.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        // `?` on an Option returns None early, much like a guard clause in Python.
        self.slots.get(id.0)?.as_ref()
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, MapError> {
        self.slots
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .ok_or(MapError::NodeNotFound(id))
    }

    /// Iterate over all live nodes in slot order (deterministic, but not tree order).
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|node| (NodeId(index), node)))
    }

    /// Walk the tree depth-first, visiting parents before their children.
    ///
    /// # Returns
    ///
    /// An iterator over [`Visit`]s, starting with the root.
    pub fn preorder(&self) -> impl Iterator<Item = Visit<'_>> {
        self.walk(self.root, false)
    }

    /// Walk one branch depth-first: `start` first, then its descendants in tree order.
    pub fn subtree_preorder(&self, start: NodeId) -> impl Iterator<Item = Visit<'_>> {
        self.walk(start, false)
    }

    /// The parent, grandparent and so on of `id`, up to and including the root.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        // `successors` repeatedly applies the closure: parent, grandparent, ...
        std::iter::successors(self.node(id).and_then(|node| node.parent), |&ancestor| {
            self.node(ancestor).and_then(|node| node.parent)
        })
    }

    /// Like [`MindMap::preorder`], but skips the descendants of collapsed nodes: exactly
    /// the nodes that are on screen.
    pub fn visible_preorder(&self) -> impl Iterator<Item = Visit<'_>> {
        self.walk(self.root, true)
    }

    /// Like [`MindMap::visible_preorder`], but only `start` and the visible part of its
    /// branch.
    pub fn visible_subtree_preorder(&self, start: NodeId) -> impl Iterator<Item = Visit<'_>> {
        self.walk(start, true)
    }

    /// Index among the root's children of the top-level branch that contains `id`
    /// (`None` for the root itself, or a node that does not exist).
    pub(crate) fn branch_index(&self, id: NodeId) -> Option<usize> {
        let top = std::iter::once(id)
            .chain(self.ancestors(id))
            .find(|&node| self.node(node).and_then(|node| node.parent) == Some(self.root))?;
        self.node(self.root)?
            .children
            .iter()
            .position(|&child| child == top)
    }

    fn walk(&self, start: NodeId, skip_collapsed: bool) -> impl Iterator<Item = Visit<'_>> {
        // An explicit stack instead of recursion, so deep trees cannot overflow the
        // call stack. A walk starting inside a branch still reports that branch's index.
        let mut stack = vec![(start, self.branch_index(start))];
        std::iter::from_fn(move || {
            let (id, branch) = stack.pop()?;
            let node = self.node(id)?;
            if !(skip_collapsed && node.collapsed) {
                // Push in reverse so the first child is popped (visited) first.
                stack.extend(
                    node.children
                        .iter()
                        .enumerate()
                        .rev()
                        .map(|(index, &child)| (child, branch.or(Some(index)))),
                );
            }
            Some(Visit { id, node, branch })
        })
    }

    /// Size for per-node lookup tables indexed by [`NodeId::index`] (counts free slots).
    #[must_use]
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Add a new child under `parent`, placed next to its existing children.
    ///
    /// # Arguments
    ///
    /// * `parent` - Node that will own the new child
    /// * `text` - Label of the new node
    ///
    /// # Returns
    ///
    /// The id of the new node.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `parent` does not exist.
    pub fn add_child(&mut self, parent: NodeId, text: &str) -> Result<NodeId, MapError> {
        let position = self.next_child_position(parent)?;
        self.add_child_at(parent, text, position)
    }

    /// Add a new child under `parent` at an explicit world position.
    ///
    /// # Arguments
    ///
    /// * `parent` - Node that will own the new child
    /// * `text` - Label of the new node
    /// * `position` - Centre of the new node in world coordinates
    ///
    /// # Returns
    ///
    /// The id of the new node.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `parent` does not exist.
    pub fn add_child_at(
        &mut self,
        parent: NodeId,
        text: &str,
        position: Point,
    ) -> Result<NodeId, MapError> {
        // Validate before touching the slab so a failure leaves the map unchanged.
        self.node(parent).ok_or(MapError::NodeNotFound(parent))?;
        let node = Node::new(text.to_owned(), position, Some(parent));
        let id = if let Some(index) = self.free.pop() {
            self.slots[index] = Some(node);
            NodeId(index)
        } else {
            self.slots.push(Some(node));
            NodeId(self.slots.len() - 1)
        };
        self.node_mut(parent)?.children.push(id);
        Ok(id)
    }

    /// Add a new node directly after `sibling`, under the same parent.
    ///
    /// # Arguments
    ///
    /// * `sibling` - Existing node whose parent receives the new node
    /// * `text` - Label of the new node
    ///
    /// # Returns
    ///
    /// The id of the new node.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `sibling` does not exist and
    /// `MapError::RootNode` if `sibling` is the root.
    pub fn add_sibling(&mut self, sibling: NodeId, text: &str) -> Result<NodeId, MapError> {
        let parent = self
            .node(sibling)
            .ok_or(MapError::NodeNotFound(sibling))?
            .parent
            .ok_or(MapError::RootNode)?;
        self.add_child(parent, text)
    }

    /// Choose where a new child of `parent` should go: one step away from the
    /// grandparent (to the right for root children), below the last existing child.
    fn next_child_position(&self, parent: NodeId) -> Result<Point, MapError> {
        let parent_node = self.node(parent).ok_or(MapError::NodeNotFound(parent))?;
        if let Some(last) = parent_node.children.last().and_then(|&id| self.node(id)) {
            return Ok(Point::new(
                last.position.x,
                last.position.y + SIBLING_OFFSET_Y,
            ));
        }
        let direction =
            parent_node
                .parent
                .and_then(|id| self.node(id))
                .map_or(1.0, |grandparent| {
                    if parent_node.position.x < grandparent.position.x {
                        -1.0
                    } else {
                        1.0
                    }
                });
        Ok(Point::new(
            parent_node.position.x + direction * CHILD_OFFSET_X,
            parent_node.position.y,
        ))
    }

    /// Replace a node's label.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), MapError> {
        let node = self.node_mut(id)?;
        // Reuses the existing String allocation when it is large enough.
        node.text.clear();
        node.text.push_str(text);
        Ok(())
    }

    /// Place a single node (not its descendants) at `position`.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_position(&mut self, id: NodeId, position: Point) -> Result<(), MapError> {
        self.node_mut(id)?.position = position;
        Ok(())
    }

    /// Hide (`true`) or show (`false`) a node's descendants.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist and `MapError::RootNode`
    /// for the root, whose branches spread to both sides and cannot fold one way.
    pub fn set_collapsed(&mut self, id: NodeId, collapsed: bool) -> Result<(), MapError> {
        if id == self.root {
            return Err(MapError::RootNode);
        }
        self.node_mut(id)?.collapsed = collapsed;
        Ok(())
    }

    /// Give a node its own colour (also used by its descendants that have none), or
    /// `None` to go back to the automatic branch colour.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_color(&mut self, id: NodeId, color: Option<NodeColor>) -> Result<(), MapError> {
        self.node_mut(id)?.color = color;
        Ok(())
    }

    /// Replace a node's notes (trailing whitespace is trimmed; empty removes them).
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_note(&mut self, id: NodeId, note: &str) -> Result<(), MapError> {
        let target = self.node_mut(id)?;
        target.note.clear();
        target.note.push_str(note.trim_end());
        Ok(())
    }

    /// Attach a web address to a node, or remove it with `None` or a blank string.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_link(&mut self, id: NodeId, link: Option<&str>) -> Result<(), MapError> {
        self.node_mut(id)?.link = link
            .map(str::trim)
            .filter(|link| !link.is_empty())
            .map(str::to_owned);
        Ok(())
    }

    /// Write a node's label in `style`.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_style(&mut self, id: NodeId, style: NodeStyle) -> Result<(), MapError> {
        self.node_mut(id)?.style = style;
        Ok(())
    }

    /// Show `picture` on a node, or remove its picture with `None`.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn set_picture(&mut self, id: NodeId, picture: Option<Picture>) -> Result<(), MapError> {
        self.node_mut(id)?.picture = picture;
        Ok(())
    }

    /// Move a node and all of its descendants by the same offset.
    ///
    /// # Arguments
    ///
    /// * `id` - Root of the subtree to move
    /// * `dx`, `dy` - Offset in world units
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn translate_subtree(&mut self, id: NodeId, dx: f32, dy: f32) -> Result<(), MapError> {
        self.node(id).ok_or(MapError::NodeNotFound(id))?;
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            let node = self.node_mut(current)?;
            node.position.x += dx;
            node.position.y += dy;
            stack.extend_from_slice(&node.children);
        }
        Ok(())
    }

    /// Whether `node` is `ancestor` itself or lies somewhere below it.
    ///
    /// Walks up from `node`, so the cost is the depth of `node`, not the subtree size.
    #[must_use]
    pub fn is_in_subtree(&self, node: NodeId, ancestor: NodeId) -> bool {
        node == ancestor || self.ancestors(node).any(|id| id == ancestor)
    }

    /// Re-attach `id` (with its whole branch) as the last child of `new_parent`, and move
    /// the branch to where a new child of `new_parent` would be placed.
    ///
    /// If the branch ends up on the other side of its parent (e.g. it moves from the left
    /// half of the map to the right), its descendants are mirrored so they keep growing
    /// outwards instead of overlapping the rest of the map.
    ///
    /// # Arguments
    ///
    /// * `id` - Root of the branch to move
    /// * `new_parent` - Node that will own the branch
    ///
    /// # Errors
    ///
    /// * `MapError::NodeNotFound` if either node does not exist
    /// * `MapError::RootNode` if `id` is the root
    /// * `MapError::WouldCreateCycle` if `new_parent` is `id` or one of its descendants
    pub fn move_subtree(&mut self, id: NodeId, new_parent: NodeId) -> Result<(), MapError> {
        let old_parent = self
            .node(id)
            .ok_or(MapError::NodeNotFound(id))?
            .parent
            .ok_or(MapError::RootNode)?;
        // Computed before re-attaching so the slot does not account for `id` itself.
        let slot = self.next_child_position(new_parent)?;
        if self.is_in_subtree(new_parent, id) {
            return Err(MapError::WouldCreateCycle);
        }

        // Which way the branch grows is read from its own children, not from where the
        // node sits now: a drag may already have carried it across to the other side.
        let x_of = |map: &Self, node: NodeId| map.node(node).map_or(0.0, |n| n.position.x);
        let node_x = x_of(self, id);
        let grows_left = self
            .node(id)
            .and_then(|node| node.children.first())
            .map(|&child| x_of(self, child) < node_x);
        let goes_left = slot.x < x_of(self, new_parent);
        if grows_left.is_some_and(|grows_left| grows_left != goes_left) {
            self.mirror_descendants(id, node_x)?;
        }

        self.node_mut(old_parent)?
            .children
            .retain(|&child| child != id);
        self.node_mut(new_parent)?.children.push(id);
        let node = self.node_mut(id)?;
        node.parent = Some(new_parent);
        let (dx, dy) = (slot.x - node.position.x, slot.y - node.position.y);
        self.translate_subtree(id, dx, dy)
    }

    /// Let two siblings trade places on screen, each with its whole branch, and swap
    /// their order among the parent's children.
    ///
    /// The upper branch's visible part moves down to end where the lower one ended, and the
    /// lower one moves up to start where the upper one started, keeping the gap between
    /// them; so branches of different heights trade places without overlapping.
    ///
    /// # Errors
    ///
    /// * `MapError::NodeNotFound` if either node does not exist
    /// * `MapError::RootNode` if `a` is the root
    /// * `MapError::NotSiblings` if the nodes are the same or have different parents
    pub fn swap_siblings(&mut self, a: NodeId, b: NodeId) -> Result<(), MapError> {
        let node_a = self.node(a).ok_or(MapError::NodeNotFound(a))?;
        let parent = node_a.parent.ok_or(MapError::RootNode)?;
        let node_b = self.node(b).ok_or(MapError::NodeNotFound(b))?;
        if a == b || node_b.parent != Some(parent) {
            return Err(MapError::NotSiblings);
        }
        let (upper, lower) = if node_a.position.y <= node_b.position.y {
            (a, b)
        } else {
            (b, a)
        };
        let (upper_top, upper_bottom) = self.visible_span(upper);
        let (lower_top, lower_bottom) = self.visible_span(lower);
        self.translate_subtree(lower, 0.0, upper_top - lower_top)?;
        self.translate_subtree(upper, 0.0, lower_bottom - upper_bottom)?;
        let children = &mut self.node_mut(parent)?.children;
        // Both are children of `parent`, checked above.
        if let (Some(index_a), Some(index_b)) = (
            children.iter().position(|&child| child == a),
            children.iter().position(|&child| child == b),
        ) {
            children.swap(index_a, index_b);
        }
        Ok(())
    }

    /// The lowest and highest node centre (y) in the visible part of `id`'s branch.
    fn visible_span(&self, id: NodeId) -> (f32, f32) {
        self.walk(id, true)
            .map(|visit| visit.node.position.y)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(top, bottom), y| {
                (top.min(y), bottom.max(y))
            })
    }

    /// Re-attach `id` (with its whole branch) next to `sibling`, just before or `after` it
    /// among `sibling`'s parent's children, on the same side of the map and just above or
    /// below it. Running [`crate::layout::tidy_branch`] on the parent afterwards makes room.
    ///
    /// # Errors
    ///
    /// * `MapError::NodeNotFound` if either node does not exist
    /// * `MapError::RootNode` if either is the root (the root has no siblings)
    /// * `MapError::WouldCreateCycle` if `sibling` is `id` or inside its branch
    pub fn move_beside(
        &mut self,
        id: NodeId,
        sibling: NodeId,
        after: bool,
    ) -> Result<(), MapError> {
        let parent = self
            .node(sibling)
            .ok_or(MapError::NodeNotFound(sibling))?
            .parent
            .ok_or(MapError::RootNode)?;
        if self.is_in_subtree(sibling, id) {
            return Err(MapError::WouldCreateCycle);
        }
        self.move_subtree(id, parent)?;
        let parent_x = self.node_mut(parent)?.position.x;
        let target = self.node_mut(sibling)?.position;
        let current = self.node_mut(id)?.position;
        // A branch keeps growing away from the centre: flip it if it changes sides.
        if (current.x < parent_x) != (target.x < parent_x) {
            self.mirror_descendants(id, current.x)?;
        }
        // One unit above or below is enough for the layout to keep this order.
        let offset = if after { 1.0 } else { -1.0 };
        self.translate_subtree(id, target.x - current.x, target.y + offset - current.y)?;
        let children = &mut self.node_mut(parent)?.children;
        children.retain(|&child| child != id);
        let at = children
            .iter()
            .position(|&child| child == sibling)
            .map_or(children.len(), |index| index + usize::from(after));
        children.insert(at, id);
        Ok(())
    }

    /// Reflect every descendant of `id` horizontally across the vertical line `axis_x`.
    fn mirror_descendants(&mut self, id: NodeId, axis_x: f32) -> Result<(), MapError> {
        let mut stack = self
            .node(id)
            .ok_or(MapError::NodeNotFound(id))?
            .children
            .clone();
        while let Some(current) = stack.pop() {
            let node = self.node_mut(current)?;
            node.position.x = 2.0 * axis_x - node.position.x;
            stack.extend_from_slice(&node.children);
        }
        Ok(())
    }

    /// The relationship lines between nodes, in the order they were added.
    #[must_use]
    pub fn cross_links(&self) -> &[CrossLink] {
        &self.links
    }

    /// Add a relationship line from `from` to `to`.
    ///
    /// # Errors
    ///
    /// * `MapError::NodeNotFound` if either node does not exist
    /// * `MapError::LinkToSelf` if they are the same node
    /// * `MapError::DuplicateLink` if they are already linked, in either direction
    pub fn add_cross_link(&mut self, from: NodeId, to: NodeId) -> Result<(), MapError> {
        self.node(from).ok_or(MapError::NodeNotFound(from))?;
        self.node(to).ok_or(MapError::NodeNotFound(to))?;
        if from == to {
            return Err(MapError::LinkToSelf);
        }
        if self.links.iter().any(|link| link.joins(from, to)) {
            return Err(MapError::DuplicateLink);
        }
        self.links.push(CrossLink {
            from,
            to,
            label: String::new(),
        });
        Ok(())
    }

    /// Set the text shown on the link between `a` and `b` (whichever way it points);
    /// surrounding spaces are trimmed, and an empty label removes it.
    ///
    /// # Errors
    ///
    /// Returns `MapError::LinkNotFound` if they are not linked.
    pub fn set_cross_link_label(
        &mut self,
        a: NodeId,
        b: NodeId,
        label: &str,
    ) -> Result<(), MapError> {
        let link = self
            .links
            .iter_mut()
            .find(|link| link.joins(a, b))
            .ok_or(MapError::LinkNotFound)?;
        label.trim().clone_into(&mut link.label);
        Ok(())
    }

    /// Remove the relationship line between `a` and `b`, whichever way it points.
    ///
    /// # Errors
    ///
    /// Returns `MapError::LinkNotFound` if they are not linked.
    pub fn remove_cross_link(&mut self, a: NodeId, b: NodeId) -> Result<(), MapError> {
        let index = self
            .links
            .iter()
            .position(|link| link.joins(a, b))
            .ok_or(MapError::LinkNotFound)?;
        // `remove` (not `swap_remove`) keeps the remaining links in order.
        self.links.remove(index);
        Ok(())
    }

    /// Delete a node together with all of its descendants.
    ///
    /// # Returns
    ///
    /// The number of nodes removed.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist and `MapError::RootNode` if
    /// `id` is the root.
    pub fn remove_subtree(&mut self, id: NodeId) -> Result<usize, MapError> {
        let parent = self
            .node(id)
            .ok_or(MapError::NodeNotFound(id))?
            .parent
            .ok_or(MapError::RootNode)?;
        self.node_mut(parent)?.children.retain(|&child| child != id);

        let mut removed = 0;
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            // `take()` moves the node out of its slot and leaves `None` behind.
            if let Some(node) = self.slots[current.0].take() {
                stack.extend(node.children);
                self.free.push(current.0);
                removed += 1;
            }
        }
        // Links to or from removed nodes go with them.
        let slots = &self.slots;
        let live = |id: NodeId| slots.get(id.0).is_some_and(Option::is_some);
        self.links.retain(|link| live(link.from) && live(link.to));
        Ok(removed)
    }

    /// Flatten the map's nodes into pre-order records (without cross-links).
    #[must_use]
    pub fn to_records(&self) -> Vec<NodeRecord> {
        self.to_map_records().nodes
    }

    /// Flatten the whole map, nodes and cross-links, into records for saving.
    #[must_use]
    pub fn to_map_records(&self) -> MapRecords {
        self.records_from(self.root)
    }

    /// A new map holding a copy of `id`'s branch, with `id` as its central idea: every
    /// descendant with its details, and the cross-links between nodes of the branch.
    ///
    /// # Errors
    ///
    /// Returns `MapError::NodeNotFound` if `id` does not exist.
    pub fn branch_copy(&self, id: NodeId) -> Result<Self, MapError> {
        self.node(id).ok_or(MapError::NodeNotFound(id))?;
        Self::from_map_records(self.records_from(id))
    }

    /// Flatten the branch of `start` (the whole map for the root) into records, with
    /// `start` as the first, parentless record. Links leaving the branch are left out.
    fn records_from(&self, start: NodeId) -> MapRecords {
        let mut index_of = vec![usize::MAX; self.slots.len()];
        let mut records = Vec::new();
        for visit in self.subtree_preorder(start) {
            index_of[visit.id.0] = records.len();
            let parent = visit.node.parent.filter(|_| visit.id != start);
            records.push(NodeRecord {
                parent: parent.map(|parent| index_of[parent.0]),
                text: visit.node.text.clone(),
                position: visit.node.position,
                collapsed: visit.node.collapsed,
                color: visit.node.color,
                note: visit.node.note.clone(),
                link: visit.node.link.clone(),
                picture: visit.node.picture.as_ref().map(Picture::to_data_url),
                style: visit.node.style,
            });
        }
        let inside = |id: NodeId| Some(index_of[id.0]).filter(|&index| index != usize::MAX);
        let links = self
            .links
            .iter()
            .filter_map(|link| {
                Some(LinkRecord {
                    from: inside(link.from)?,
                    to: inside(link.to)?,
                    label: link.label.clone(),
                })
            })
            .collect();
        MapRecords {
            nodes: records,
            links,
        }
    }

    /// Rebuild a map without cross-links from node records (see
    /// [`MindMap::from_map_records`]).
    ///
    /// # Errors
    ///
    /// As [`MindMap::from_map_records`].
    pub fn from_records(records: Vec<NodeRecord>) -> Result<Self, MapError> {
        Self::from_map_records(MapRecords {
            nodes: records,
            links: Vec::new(),
        })
    }

    /// Rebuild a map from records produced by [`MindMap::to_map_records`].
    ///
    /// # Errors
    ///
    /// * `MapError::Empty` if there are no node records
    /// * `MapError::InvalidRoot` if the first record has a parent or a later one has none
    /// * `MapError::InvalidParent` if a record's parent does not appear before it
    /// * `MapError::InvalidLink` if a link record names a missing node, links a node to
    ///   itself, or repeats a link
    pub fn from_map_records(records: MapRecords) -> Result<Self, MapError> {
        let MapRecords {
            nodes: records,
            links,
        } = records;
        if records.is_empty() {
            return Err(MapError::Empty);
        }
        let mut slots: Vec<Option<Node>> = Vec::with_capacity(records.len());
        for (index, record) in records.into_iter().enumerate() {
            let parent = match (index, record.parent) {
                (0, None) => None,
                (0, Some(_)) | (_, None) => return Err(MapError::InvalidRoot),
                (_, Some(parent)) if parent >= index => {
                    return Err(MapError::InvalidParent { index, parent });
                }
                (_, Some(parent)) => {
                    // Every earlier slot is filled, so this lookup always succeeds.
                    if let Some(parent_node) = slots[parent].as_mut() {
                        parent_node.children.push(NodeId(index));
                    }
                    Some(NodeId(parent))
                }
            };
            let picture = record
                .picture
                .as_deref()
                .map(Picture::from_data_url)
                .transpose()
                .map_err(|_| MapError::InvalidPicture { index })?;
            slots.push(Some(Node {
                picture,
                style: record.style,
                collapsed: record.collapsed,
                color: record.color,
                note: record.note,
                // An empty address is the same as none.
                link: record.link.filter(|link| !link.trim().is_empty()),
                ..Node::new(record.text, record.position, parent)
            }));
        }
        let mut map = Self {
            slots,
            free: Vec::new(),
            root: NodeId(0),
            links: Vec::with_capacity(links.len()),
        };
        for (index, link) in links.into_iter().enumerate() {
            // Record indices are slot indices here: the map was just built in that order.
            let (from, to) = (NodeId(link.from), NodeId(link.to));
            map.add_cross_link(from, to)
                .and_then(|()| map.set_cross_link_label(from, to, &link.label))
                .map_err(|_| MapError::InvalidLink { index })?;
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_map() -> (MindMap, NodeId, NodeId, NodeId) {
        let mut map = MindMap::new("root");
        let a = map.add_child(map.root(), "a").expect("root exists");
        let b = map.add_child(map.root(), "b").expect("root exists");
        let a1 = map.add_child(a, "a1").expect("a exists");
        (map, a, b, a1)
    }

    #[test]
    fn new_map_has_only_root() {
        let map = MindMap::new("center");

        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
        let root = map.node(map.root()).expect("root exists");
        assert_eq!(root.text(), "center");
        assert_eq!(root.parent(), None);
    }

    #[test]
    fn add_child_links_parent_and_places_node() {
        let (map, a, b, a1) = sample_map();

        assert_eq!(map.node(map.root()).expect("root").children(), &[a, b]);
        assert_eq!(map.node(a1).expect("a1").parent(), Some(a));
        let a_pos = map.node(a).expect("a").position();
        let b_pos = map.node(b).expect("b").position();
        assert_eq!(a_pos, Point::new(CHILD_OFFSET_X, 0.0));
        assert_eq!(b_pos, Point::new(CHILD_OFFSET_X, SIBLING_OFFSET_Y));
    }

    #[test]
    fn child_of_left_branch_grows_left() {
        let mut map = MindMap::new("root");
        let left = map.add_child(map.root(), "left").expect("root exists");
        map.translate_subtree(left, -2.0 * CHILD_OFFSET_X, 0.0)
            .expect("left exists");

        let grandchild = map.add_child(left, "x").expect("left exists");

        let x = map.node(grandchild).expect("grandchild").position().x;
        assert_eq!(x, -2.0 * CHILD_OFFSET_X);
    }

    #[test]
    fn add_child_to_missing_parent_fails() {
        let mut map = MindMap::new("root");

        let result = map.add_child(NodeId(42), "x");

        assert_eq!(result, Err(MapError::NodeNotFound(NodeId(42))));
    }

    #[test]
    fn add_child_at_uses_given_position_and_validates_parent() {
        let mut map = MindMap::new("root");

        let id = map
            .add_child_at(map.root(), "x", Point::new(-5.0, 7.0))
            .expect("root exists");

        assert_eq!(map.node(id).expect("x").position(), Point::new(-5.0, 7.0));
        assert_eq!(
            map.add_child_at(NodeId(9), "y", Point::default()),
            Err(MapError::NodeNotFound(NodeId(9)))
        );
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn add_sibling_uses_same_parent_and_rejects_root() {
        let (mut map, a, _, a1) = sample_map();

        let sibling = map.add_sibling(a1, "a2").expect("a1 exists");

        assert_eq!(map.node(sibling).expect("a2").parent(), Some(a));
        assert_eq!(map.add_sibling(map.root(), "x"), Err(MapError::RootNode));
    }

    #[test]
    fn set_text_replaces_label() {
        let (mut map, a, _, _) = sample_map();

        map.set_text(a, "renamed").expect("a exists");

        assert_eq!(map.node(a).expect("a").text(), "renamed");
    }

    #[test]
    fn translate_subtree_moves_descendants_only() {
        let (mut map, a, b, a1) = sample_map();
        let before_b = map.node(b).expect("b").position();
        let before_a1 = map.node(a1).expect("a1").position();

        map.translate_subtree(a, 10.0, -5.0).expect("a exists");

        assert_eq!(map.node(b).expect("b").position(), before_b);
        let after_a1 = map.node(a1).expect("a1").position();
        assert_eq!(after_a1, Point::new(before_a1.x + 10.0, before_a1.y - 5.0));
    }

    #[test]
    fn remove_subtree_frees_and_reuses_slots() {
        let (mut map, a, b, a1) = sample_map();

        assert_eq!(map.remove_subtree(a), Ok(2));

        assert_eq!(map.len(), 2);
        assert!(map.node(a).is_none());
        assert!(map.node(a1).is_none());
        assert_eq!(map.node(map.root()).expect("root").children(), &[b]);
        let reused = map.add_child(b, "new").expect("b exists");
        assert!(reused == a || reused == a1);
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn is_in_subtree_follows_ancestry() {
        let (map, a, b, a1) = sample_map();

        assert!(map.is_in_subtree(a1, a));
        assert!(map.is_in_subtree(a, a));
        assert!(map.is_in_subtree(a1, map.root()));
        assert!(!map.is_in_subtree(a1, b));
        assert!(!map.is_in_subtree(a, a1));
    }

    #[test]
    fn move_subtree_reattaches_and_relocates_branch() {
        let (mut map, a, b, a1) = sample_map();
        let grandchild_before = map.node(a1).expect("a1").position();
        let a_before = map.node(a).expect("a").position();

        map.move_subtree(a, b).expect("valid move");

        assert_eq!(map.node(a).expect("a").parent(), Some(b));
        assert_eq!(map.node(b).expect("b").children(), &[a]);
        assert_eq!(map.node(map.root()).expect("root").children(), &[b]);
        let b_pos = map.node(b).expect("b").position();
        let a_after = map.node(a).expect("a").position();
        assert_eq!(a_after, Point::new(b_pos.x + CHILD_OFFSET_X, b_pos.y));
        // The descendant moved by the same offset as the branch root.
        let grandchild_after = map.node(a1).expect("a1").position();
        assert_eq!(
            grandchild_after.x - grandchild_before.x,
            a_after.x - a_before.x
        );
        assert_eq!(
            grandchild_after.y - grandchild_before.y,
            a_after.y - a_before.y
        );
    }

    #[test]
    fn branch_moved_to_other_side_is_mirrored() {
        let (mut map, a, _, a1) = sample_map();
        let left = map
            .add_child_at(map.root(), "left", Point::new(-CHILD_OFFSET_X, 100.0))
            .expect("root exists");

        map.move_subtree(a, left).expect("valid move");

        // `a` goes one step further left, and its child now grows leftwards too.
        let a_pos = map.node(a).expect("a").position();
        assert_eq!(a_pos, Point::new(-2.0 * CHILD_OFFSET_X, 100.0));
        let child_pos = map.node(a1).expect("a1").position();
        assert_eq!(child_pos, Point::new(-3.0 * CHILD_OFFSET_X, 100.0));
    }

    #[test]
    fn branch_dragged_across_before_drop_is_still_mirrored() {
        let (mut map, a, _, a1) = sample_map();
        let left = map
            .add_child_at(map.root(), "left", Point::new(-CHILD_OFFSET_X, 100.0))
            .expect("root exists");
        // The user drags `a` over to the left side before releasing it.
        map.translate_subtree(a, -3.0 * CHILD_OFFSET_X, 100.0)
            .expect("a exists");

        map.move_subtree(a, left).expect("valid move");

        let a_x = map.node(a).expect("a").position().x;
        let child_x = map.node(a1).expect("a1").position().x;
        assert_eq!(child_x, a_x - CHILD_OFFSET_X);
    }

    #[test]
    fn swapped_siblings_trade_places_without_overlapping() {
        let mut map = MindMap::new("root");
        let root = map.root();
        let small = map
            .add_child_at(root, "small", Point::new(100.0, 0.0))
            .expect("root");
        let big = map
            .add_child_at(root, "big", Point::new(100.0, 100.0))
            .expect("root");
        for (index, y) in [60.0, 100.0, 140.0].into_iter().enumerate() {
            map.add_child_at(big, &format!("big {index}"), Point::new(200.0, y))
                .expect("big");
        }
        let hidden = map
            .add_child_at(small, "hidden", Point::new(200.0, 500.0))
            .expect("small");
        map.set_collapsed(small, true).expect("small");

        map.swap_siblings(small, big).expect("siblings");

        // Big's branch (60..140, gap 60 below small) now starts where small was (0), and
        // small ends where big's branch ended (140); hidden nodes do not count.
        let y = |id| map.node(id).expect("live").position().y;
        assert!((y(big) - 40.0).abs() < 1e-4, "{}", y(big));
        assert!((y(small) - 140.0).abs() < 1e-4, "{}", y(small));
        assert!((y(hidden) - 640.0).abs() < 1e-4, "moves with its branch");
        assert_eq!(map.node(root).expect("root").children(), &[big, small]);
    }

    #[test]
    fn swap_siblings_needs_two_children_of_one_parent() {
        let (mut map, a, b, a1) = sample_map();
        let before = map.clone();
        assert_eq!(map.swap_siblings(a, a1), Err(MapError::NotSiblings));
        assert_eq!(map.swap_siblings(a, a), Err(MapError::NotSiblings));
        assert_eq!(map.swap_siblings(map.root(), b), Err(MapError::RootNode));
        assert_eq!(map, before);
    }

    #[test]
    fn cross_links_join_two_different_nodes_once() {
        let (mut map, a, b, a1) = sample_map();

        map.add_cross_link(a1, b).expect("valid");
        assert_eq!(map.add_cross_link(b, a1), Err(MapError::DuplicateLink));
        assert_eq!(map.add_cross_link(a, a), Err(MapError::LinkToSelf));
        assert_eq!(
            map.add_cross_link(a, NodeId(99)),
            Err(MapError::NodeNotFound(NodeId(99)))
        );
        let link = &map.cross_links()[0];
        assert_eq!((link.from(), link.to()), (a1, b));
        assert!(link.touches(b) && !link.touches(a));
        map.set_cross_link_label(b, a1, "  depends on ")
            .expect("linked");
        assert_eq!(map.cross_links()[0].label(), "depends on");

        // Either direction names the same link.
        map.remove_cross_link(b, a1).expect("linked");
        assert_eq!(map.remove_cross_link(a1, b), Err(MapError::LinkNotFound));
    }

    #[test]
    fn deleting_a_branch_removes_its_links() {
        let (mut map, a, b, a1) = sample_map();
        map.add_cross_link(a1, b).expect("valid");
        map.add_cross_link(a, b).expect("valid");

        map.remove_subtree(a).expect("a exists");

        assert_eq!(map.cross_links(), &[]);
        // A new node reusing a removed slot is not linked by accident.
        let reused = map.add_child(b, "new").expect("b exists");
        assert!(reused == a || reused == a1, "slot is reused");
        assert_eq!(map.cross_links(), &[]);
    }

    #[test]
    fn links_survive_records_and_bad_ones_are_rejected() {
        let (mut map, _, b, a1) = sample_map();
        map.add_cross_link(a1, b).expect("valid");
        map.set_cross_link_label(a1, b, "see also").expect("linked");

        let records = map.to_map_records();
        let rebuilt = MindMap::from_map_records(records.clone()).expect("valid");

        assert_eq!(rebuilt.to_map_records(), records);
        assert_eq!(rebuilt.cross_links().len(), 1);
        for (from, to) in [
            (0, 0),
            (0, 99),
            (records.links[0].to, records.links[0].from),
        ] {
            let mut bad = records.clone();
            bad.links.push(LinkRecord {
                from,
                to,
                label: String::new(),
            });
            assert_eq!(
                MindMap::from_map_records(bad),
                Err(MapError::InvalidLink { index: 1 }),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn pictures_are_saved_in_records_and_checked_when_read() {
        let (mut map, a, _, _) = sample_map();
        let picture = crate::picture::test_picture(30, 20);
        map.set_picture(a, Some(picture.clone())).expect("a exists");

        let mut records = map.to_map_records();
        let rebuilt = MindMap::from_map_records(records.clone()).expect("valid");
        assert_eq!(rebuilt.node(a).and_then(Node::picture), Some(&picture));

        records.nodes[1].picture = Some("data:image/png;base64,AAAA".to_owned());
        assert_eq!(
            MindMap::from_map_records(records),
            Err(MapError::InvalidPicture { index: 1 })
        );
        map.set_picture(a, None).expect("a exists");
        assert_eq!(map.to_records()[1].picture, None);
    }

    #[test]
    fn styles_are_saved_only_when_not_plain() {
        let (mut map, a, b, _) = sample_map();
        let style = NodeStyle {
            bold: true,
            size: TextSize::Large,
            ..NodeStyle::default()
        };
        map.set_style(a, style).expect("a");

        let records = map.to_map_records();
        let json = serde_json::to_string(&records.nodes[1]).expect("serializes");
        assert!(
            json.contains(r#""style":{"bold":true,"size":"large"}"#),
            "{json}"
        );
        let plain = serde_json::to_string(&records.nodes[2]).expect("serializes");
        assert!(!plain.contains("style"), "{plain}");
        let rebuilt = MindMap::from_map_records(records).expect("valid");
        assert_eq!(rebuilt.node(a).expect("a").style(), style);
        assert!(rebuilt.node(b).expect("b").style().is_plain());
        assert!(style.width_scale() > style.font_scale());
    }

    #[test]
    fn a_branch_copy_is_a_map_of_its_own() {
        let (mut map, a, b, a1) = sample_map();
        let a2 = map.add_child(a, "a2").expect("a");
        map.set_note(a1, "details").expect("a1");
        map.add_cross_link(a1, a2).expect("valid");
        map.add_cross_link(a2, b).expect("valid");

        let copy = map.branch_copy(a).expect("a exists");

        assert_eq!(copy.len(), 3);
        let root = copy.node(copy.root()).expect("root");
        assert_eq!((root.text(), root.parent()), ("a", None));
        let texts: Vec<&str> = root
            .children()
            .iter()
            .map(|&id| copy.node(id).expect("live").text())
            .collect();
        assert_eq!(texts, ["a1", "a2"]);
        assert_eq!(copy.node(root.children()[0]).expect("a1").note(), "details");
        // The link inside the branch comes along; the one leaving it does not.
        assert_eq!(copy.cross_links().len(), 1);
        assert_eq!(map.branch_copy(map.root()).expect("root").len(), map.len());
    }

    #[test]
    fn walks_inside_a_branch_report_its_index() {
        let (map, a, b, a1) = sample_map();

        let visits: Vec<(NodeId, Option<usize>)> = map
            .visible_subtree_preorder(a)
            .map(|visit| (visit.id, visit.branch))
            .collect();

        assert_eq!(visits, [(a, Some(0)), (a1, Some(0))]);
        assert_eq!(map.branch_index(b), Some(1));
        assert_eq!(map.branch_index(map.root()), None);
    }

    #[test]
    fn moving_beside_a_sibling_sets_the_order_and_the_side() {
        let mut map = MindMap::new("root");
        let root = map.root();
        let right = map
            .add_child_at(root, "right", Point::new(100.0, 0.0))
            .expect("root");
        let left = map
            .add_child_at(root, "left", Point::new(-100.0, 0.0))
            .expect("root");
        let moved = map
            .add_child_at(right, "moved", Point::new(200.0, 50.0))
            .expect("right");
        let grandchild = map
            .add_child_at(moved, "grandchild", Point::new(300.0, 50.0))
            .expect("moved");

        map.move_beside(moved, left, false).expect("valid");

        let node = |id| map.node(id).expect("live");
        assert_eq!(node(moved).parent(), Some(root));
        assert_eq!(node(root).children(), &[right, moved, left]);
        // Just above `left`, on its side, with its own child mirrored to keep growing out.
        assert_eq!(node(moved).position(), Point::new(-100.0, -1.0));
        assert!(node(grandchild).position().x < node(moved).position().x);

        map.move_beside(right, moved, true).expect("valid");
        assert_eq!(
            map.node(root).expect("live").children(),
            &[moved, right, left]
        );
        assert_eq!(
            map.move_beside(moved, grandchild, true),
            Err(MapError::WouldCreateCycle)
        );
        assert_eq!(map.move_beside(moved, root, true), Err(MapError::RootNode));
    }

    #[test]
    fn move_subtree_rejects_invalid_moves() {
        let (mut map, a, _, a1) = sample_map();
        let before = map.clone();

        assert_eq!(map.move_subtree(a, a1), Err(MapError::WouldCreateCycle));
        assert_eq!(map.move_subtree(a, a), Err(MapError::WouldCreateCycle));
        assert_eq!(map.move_subtree(map.root(), a), Err(MapError::RootNode));
        assert_eq!(
            map.move_subtree(a, NodeId(99)),
            Err(MapError::NodeNotFound(NodeId(99)))
        );
        assert_eq!(map, before);
    }

    #[test]
    fn moved_subtree_survives_save_round_trip() {
        let (mut map, a, b, _) = sample_map();
        map.move_subtree(a, b).expect("valid move");

        let rebuilt = MindMap::from_records(map.to_records()).expect("records are valid");

        assert_eq!(rebuilt.to_records(), map.to_records());
    }

    #[test]
    fn remove_root_is_rejected() {
        let mut map = MindMap::new("root");

        assert_eq!(map.remove_subtree(map.root()), Err(MapError::RootNode));
    }

    #[test]
    fn preorder_visits_parents_first_with_branch_indices() {
        let (map, a, b, a1) = sample_map();

        let visits: Vec<(NodeId, Option<usize>)> = map
            .preorder()
            .map(|visit| (visit.id, visit.branch))
            .collect();

        assert_eq!(
            visits,
            vec![
                (map.root(), None),
                (a, Some(0)),
                (a1, Some(0)),
                (b, Some(1))
            ]
        );
    }

    #[test]
    fn subtree_preorder_and_ancestors() {
        let (map, a, b, a1) = sample_map();

        let branch: Vec<NodeId> = map.subtree_preorder(a).map(|visit| visit.id).collect();
        assert_eq!(branch, vec![a, a1]);
        let ancestors: Vec<NodeId> = map.ancestors(a1).collect();
        assert_eq!(ancestors, vec![a, map.root()]);
        assert_eq!(map.ancestors(map.root()).count(), 0);
        assert_eq!(map.subtree_preorder(b).count(), 1);
    }

    #[test]
    fn visible_preorder_skips_collapsed_descendants() {
        let (mut map, a, b, a1) = sample_map();

        map.set_collapsed(a, true).expect("a exists");

        let visible: Vec<NodeId> = map.visible_preorder().map(|visit| visit.id).collect();
        assert_eq!(visible, vec![map.root(), a, b]);
        assert!(map.node(a).expect("a").is_collapsed());
        assert_eq!(map.preorder().count(), 4);
        assert!(map.preorder().any(|visit| visit.id == a1));
    }

    #[test]
    fn root_cannot_be_collapsed() {
        let mut map = MindMap::new("root");

        assert_eq!(map.set_collapsed(map.root(), true), Err(MapError::RootNode));
    }

    #[test]
    fn collapsed_flag_survives_records() {
        let (mut map, a, _, _) = sample_map();
        map.set_collapsed(a, true).expect("a exists");

        let rebuilt = MindMap::from_records(map.to_records()).expect("records are valid");

        // Rebuilding renumbers nodes in tree order, so compare the saved form.
        assert_eq!(rebuilt.to_records(), map.to_records());
        assert!(rebuilt.to_records().iter().any(|record| record.collapsed));
    }

    #[test]
    fn colour_note_and_link_survive_records() {
        let (mut map, a, _, _) = sample_map();
        map.set_color(a, Some(NodeColor::rgb(0x12, 0xab, 0xff)))
            .expect("a exists");
        map.set_note(a, "Remember this\nsecond line  \n")
            .expect("a exists");
        map.set_link(a, Some("  https://example.com  "))
            .expect("a exists");

        let rebuilt = MindMap::from_records(map.to_records()).expect("valid");
        let node = rebuilt
            .iter()
            .find(|(_, node)| node.text() == "a")
            .expect("a exists")
            .1;

        assert_eq!(node.color(), Some(NodeColor::rgb(0x12, 0xab, 0xff)));
        assert_eq!(node.note(), "Remember this\nsecond line");
        assert_eq!(node.link(), Some("https://example.com"));
        map.set_link(a, Some("   ")).expect("a exists");
        assert_eq!(map.node(a).expect("a").link(), None);
    }

    #[test]
    fn node_colour_reads_and_writes_hex() {
        let color = NodeColor::try_from("#0aFF10".to_owned()).expect("valid");

        assert_eq!(color.components(), [0x0a, 0xff, 0x10]);
        assert_eq!(String::from(color), "#0aff10");
        for bad in ["0aff10", "#0aff1", "#0aff1g", "#0aff100"] {
            assert!(NodeColor::try_from(bad.to_owned()).is_err(), "{bad}");
        }
    }

    #[test]
    fn set_position_moves_only_that_node() {
        let (mut map, a, _, a1) = sample_map();
        let child_before = map.node(a1).expect("a1").position();

        map.set_position(a, Point::new(5.0, 6.0)).expect("a exists");

        assert_eq!(map.node(a).expect("a").position(), Point::new(5.0, 6.0));
        assert_eq!(map.node(a1).expect("a1").position(), child_before);
    }

    #[test]
    fn heap_size_grows_with_content() {
        let mut map = MindMap::new("root");
        let small = map.heap_size();
        let child = map.add_child(map.root(), "child").expect("root");
        map.set_note(child, &"note ".repeat(100)).expect("child");

        assert!(map.heap_size() >= small + 500);
    }

    #[test]
    fn iter_yields_every_live_node() {
        let (map, _, _, _) = sample_map();

        assert_eq!(map.iter().count(), map.len());
    }

    #[test]
    fn records_round_trip() {
        let (mut map, a, _, _) = sample_map();
        map.remove_subtree(a).expect("a exists");
        map.add_child(map.root(), "c").expect("root exists");

        let rebuilt = MindMap::from_records(map.to_records()).expect("records are valid");

        assert_eq!(rebuilt.to_records(), map.to_records());
        assert_eq!(rebuilt.len(), map.len());
    }

    #[test]
    fn from_records_rejects_invalid_input() {
        let record = |parent| NodeRecord {
            parent,
            ..NodeRecord::default()
        };

        assert_eq!(MindMap::from_records(Vec::new()), Err(MapError::Empty));
        assert_eq!(
            MindMap::from_records(vec![record(Some(0))]),
            Err(MapError::InvalidRoot)
        );
        assert_eq!(
            MindMap::from_records(vec![record(None), record(None)]),
            Err(MapError::InvalidRoot)
        );
        assert_eq!(
            MindMap::from_records(vec![record(None), record(Some(1))]),
            Err(MapError::InvalidParent {
                index: 1,
                parent: 1
            })
        );
    }
}
