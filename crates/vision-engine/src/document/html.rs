//! HTML parsing into the arena DOM.

use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::error::{Error, Result};

use super::dom::{Attribute, Document, NodeId};

/// Parses an HTML string into a document.
pub(crate) fn parse_html(html: &str) -> Result<Document> {
    let mut bytes = html.as_bytes();
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut bytes)
        .map_err(|error| Error::Html(error.to_string()))?;

    let mut document = Document::new();
    let root = document.root();
    for child in dom.document.children.borrow().iter() {
        if let Some(child_id) = convert(&mut document, child) {
            document.append(root, child_id);
        }
    }
    Ok(document)
}

fn convert(document: &mut Document, handle: &Handle) -> Option<NodeId> {
    match &handle.data {
        NodeData::Element { name, attrs, .. } => {
            let attributes = attrs
                .borrow()
                .iter()
                .map(|attribute| Attribute {
                    name: attribute.name.local.to_string(),
                    value: attribute.value.to_string(),
                })
                .collect();
            let node = document.create_element(name.local.to_string(), attributes);
            for child in handle.children.borrow().iter() {
                if let Some(child_id) = convert(document, child) {
                    document.append(node, child_id);
                }
            }
            Some(node)
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            (!text.is_empty()).then(|| document.create_text(text))
        }
        NodeData::Document
        | NodeData::Doctype { .. }
        | NodeData::Comment { .. }
        | NodeData::ProcessingInstruction { .. } => None,
    }
}
