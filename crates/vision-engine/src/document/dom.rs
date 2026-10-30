//! Arena DOM.

use crate::error::Result;

/// Identifies a node in a [`Document`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeId(pub(crate) u32);

impl NodeId {
    /// Returns the index into the document arena.
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// An HTML element.
#[derive(Clone, Debug)]
pub(crate) struct ElementData {
    /// Lowercase local name.
    pub(crate) name: String,
    /// Attributes in source order.
    pub(crate) attrs: Vec<Attribute>,
}

/// An element attribute.
#[derive(Clone, Debug)]
pub(crate) struct Attribute {
    /// Lowercase local name.
    pub(crate) name: String,
    /// Attribute value.
    pub(crate) value: String,
}

/// The kind of a DOM node.
#[derive(Clone, Debug)]
pub(crate) enum NodeKind {
    /// The document root.
    Document,
    /// An element.
    Element(ElementData),
    /// A text node.
    Text(String),
}

/// A node in the arena.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) parent: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub(crate) prev_sibling: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
    pub(crate) kind: NodeKind,
}

/// A parsed document backed by a flat arena.
#[derive(Clone, Debug)]
pub(crate) struct Document {
    nodes: Vec<Node>,
    root: NodeId,
}

impl Document {
    /// Creates an empty document.
    pub(crate) fn new() -> Self {
        let root = NodeId(0);
        Self {
            nodes: vec![Node {
                parent: None,
                first_child: None,
                last_child: None,
                prev_sibling: None,
                next_sibling: None,
                kind: NodeKind::Document,
            }],
            root,
        }
    }

    /// Parses an HTML string.
    pub(crate) fn parse(html: &str) -> Result<Self> {
        super::html::parse_html(html)
    }

    /// Returns the document root.
    pub(crate) fn root(&self) -> NodeId {
        self.root
    }

    /// Returns the number of nodes.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns a node.
    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Returns a mutable node.
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    /// Returns the parent of a node.
    pub(crate) fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// Returns the element data for a node.
    pub(crate) fn element(&self, id: NodeId) -> Option<&ElementData> {
        match &self.node(id).kind {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }

    /// Returns the element name for a node.
    pub(crate) fn element_name(&self, id: NodeId) -> Option<&str> {
        self.element(id).map(|element| element.name.as_str())
    }

    /// Returns an attribute value.
    pub(crate) fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        self.element(id).and_then(|element| {
            element
                .attrs
                .iter()
                .find(|attribute| attribute.name == name)
                .map(|attribute| attribute.value.as_str())
        })
    }

    /// Sets an attribute on an element, adding it when absent.
    pub(crate) fn set_attr(&mut self, id: NodeId, name: &str, value: impl Into<String>) {
        let value = value.into();
        let NodeKind::Element(element) = &mut self.node_mut(id).kind else {
            return;
        };
        if let Some(attribute) = element
            .attrs
            .iter_mut()
            .find(|attribute| attribute.name == name)
        {
            attribute.value = value;
        } else {
            element.attrs.push(Attribute {
                name: name.to_owned(),
                value,
            });
        }
    }

    /// Removes an attribute from an element.
    pub(crate) fn remove_attr(&mut self, id: NodeId, name: &str) {
        let NodeKind::Element(element) = &mut self.node_mut(id).kind else {
            return;
        };
        element.attrs.retain(|attribute| attribute.name != name);
    }

    /// Returns the classes on an element.
    pub(crate) fn classes(&self, id: NodeId) -> Vec<&str> {
        self.attr(id, "class")
            .map(|value| value.split_ascii_whitespace().collect())
            .unwrap_or_default()
    }

    /// Returns the text of a node and its descendants.
    pub(crate) fn text_content(&self, id: NodeId) -> String {
        let mut output = String::new();
        self.collect_text(id, &mut output);
        output
    }

    /// Returns rendered text, excluding scripts, styles, and document metadata.
    pub(crate) fn visible_text(&self, id: NodeId) -> String {
        let mut output = String::new();
        self.collect_visible_text(id, &mut output);
        output
    }

    fn collect_text(&self, id: NodeId, output: &mut String) {
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            match &self.node(current).kind {
                NodeKind::Text(text) => output.push_str(text),
                NodeKind::Document | NodeKind::Element(_) => {
                    let children: Vec<_> = self.children(current).collect();
                    for child in children.into_iter().rev() {
                        stack.push(child);
                    }
                }
            }
        }
    }

    fn collect_visible_text(&self, id: NodeId, output: &mut String) {
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            match &self.node(current).kind {
                NodeKind::Text(text) => output.push_str(text),
                NodeKind::Element(element) => {
                    if matches!(
                        element.name.as_str(),
                        "script" | "style" | "head" | "title" | "meta" | "link" | "template"
                    ) {
                        continue;
                    }
                    let children: Vec<_> = self.children(current).collect();
                    for child in children.into_iter().rev() {
                        stack.push(child);
                    }
                }
                NodeKind::Document => {
                    let children: Vec<_> = self.children(current).collect();
                    for child in children.into_iter().rev() {
                        stack.push(child);
                    }
                }
            }
        }
    }

    /// Returns a node's children in tree order.
    pub(crate) fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            document: self,
            next: self.node(id).first_child,
        }
    }

    /// Returns a node's first child, if any.
    pub(crate) fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).first_child
    }

    /// Returns a node's last child, if any.
    pub(crate) fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).last_child
    }

    /// Returns all nodes in tree order, starting at the document root.
    pub(crate) fn descendants(&self) -> Descendants<'_> {
        Descendants {
            document: self,
            stack: vec![self.root],
        }
    }

    /// Returns the first element with the given name.
    pub(crate) fn find_first_element(&self, name: &str) -> Option<NodeId> {
        self.descendants()
            .find(|id| self.element_name(*id) == Some(name))
    }

    /// Detaches a node from its parent.
    pub(crate) fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.node(child).parent else {
            return;
        };
        let previous = self.node(child).prev_sibling;
        let next = self.node(child).next_sibling;
        if let Some(previous) = previous {
            self.node_mut(previous).next_sibling = next;
        } else {
            self.node_mut(parent).first_child = next;
        }
        if let Some(next) = next {
            self.node_mut(next).prev_sibling = previous;
        } else {
            self.node_mut(parent).last_child = previous;
        }
        let child_node = self.node_mut(child);
        child_node.parent = None;
        child_node.prev_sibling = None;
        child_node.next_sibling = None;
    }

    /// Removes a child from a parent.
    pub(crate) fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if self.node(child).parent == Some(parent) {
            self.detach(child);
        }
    }

    /// Inserts a child before a reference child.
    pub(crate) fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: NodeId) {
        if self.node(reference).parent != Some(parent) {
            self.append(parent, child);
            return;
        }
        if child == reference {
            return;
        }
        self.detach(child);
        let previous = self.node(reference).prev_sibling;
        {
            let child_node = self.node_mut(child);
            child_node.parent = Some(parent);
            child_node.prev_sibling = previous;
            child_node.next_sibling = Some(reference);
        }
        if let Some(previous) = previous {
            self.node_mut(previous).next_sibling = Some(child);
        } else {
            self.node_mut(parent).first_child = Some(child);
        }
        self.node_mut(reference).prev_sibling = Some(child);
    }

    /// Removes all children from a parent.
    pub(crate) fn clear_children(&mut self, parent: NodeId) {
        while let Some(child) = self.node(parent).first_child {
            self.detach(child);
        }
    }

    /// Replaces the content of a node with text.
    pub(crate) fn set_text_content(&mut self, node: NodeId, text: &str) {
        if let NodeKind::Text(current) = &mut self.node_mut(node).kind {
            *current = text.to_owned();
            return;
        }
        self.clear_children(node);
        let child = self.create_text(text);
        self.append(node, child);
    }

    /// Appends a child to a parent.
    pub(crate) fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        let previous = self.node(parent).last_child;
        {
            let child_node = self.node_mut(child);
            child_node.parent = Some(parent);
            child_node.prev_sibling = previous;
        }
        if let Some(previous) = previous {
            self.node_mut(previous).next_sibling = Some(child);
        } else {
            self.node_mut(parent).first_child = Some(child);
        }
        self.node_mut(parent).last_child = Some(child);
    }

    /// Creates an element node.
    pub(crate) fn create_element(
        &mut self,
        name: impl Into<String>,
        attrs: Vec<Attribute>,
    ) -> NodeId {
        self.push(NodeKind::Element(ElementData {
            name: name.into(),
            attrs,
        }))
    }

    /// Creates a text node.
    pub(crate) fn create_text(&mut self, text: impl Into<String>) -> NodeId {
        self.push(NodeKind::Text(text.into()))
    }

    /// Copies a node and its descendants into another document.
    pub(crate) fn copy_subtree_into(
        &self,
        id: NodeId,
        target: &mut Self,
        parent: NodeId,
    ) -> NodeId {
        let copied = match &self.node(id).kind {
            NodeKind::Document => target.create_element("div", Vec::new()),
            NodeKind::Text(text) => target.create_text(text.clone()),
            NodeKind::Element(element) => {
                target.create_element(element.name.clone(), element.attrs.clone())
            }
        };
        target.append(parent, copied);

        let mut stack: Vec<(NodeId, NodeId)> =
            self.children(id).map(|child| (child, copied)).collect();
        stack.reverse();
        while let Some((source, target_parent)) = stack.pop() {
            let child = match &self.node(source).kind {
                NodeKind::Document => target.create_element("div", Vec::new()),
                NodeKind::Text(text) => target.create_text(text.clone()),
                NodeKind::Element(element) => {
                    target.create_element(element.name.clone(), element.attrs.clone())
                }
            };
            target.append(target_parent, child);
            let mut children: Vec<(NodeId, NodeId)> =
                self.children(source).map(|node| (node, child)).collect();
            children.reverse();
            stack.extend(children);
        }
        copied
    }

    fn push(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            kind,
        });
        id
    }
}

/// Iterator over a node's children.
pub(crate) struct Children<'a> {
    document: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.document.node(current).next_sibling;
        Some(current)
    }
}

/// Iterator over a document in tree order.
pub(crate) struct Descendants<'a> {
    document: &'a Document,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.stack.pop()?;
        let mut child = self.document.node(current).last_child;
        while let Some(id) = child {
            self.stack.push(id);
            child = self.document.node(id).prev_sibling;
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deep_document(depth: usize) -> Document {
        let mut document = Document::new();
        let mut parent = document.root();
        for _ in 0..depth {
            let node = document.create_element("div", Vec::new());
            document.append(parent, node);
            parent = node;
        }
        let text = document.create_text("leaf");
        document.append(parent, text);
        document
    }

    #[test]
    fn text_content_handles_deep_documents() {
        let document = deep_document(20_000);
        assert_eq!(document.text_content(document.root()), "leaf");
        assert_eq!(document.visible_text(document.root()), "leaf");
    }

    #[test]
    fn visible_text_skips_script_and_style_subtrees() {
        let document = Document::parse(
            "<body>before<script>hidden()</script><style>.x{}</style><p>after</p></body>",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(document.visible_text(document.root()).contains("before"));
        assert!(document.visible_text(document.root()).contains("after"));
        assert!(!document.visible_text(document.root()).contains("hidden"));
    }
}
