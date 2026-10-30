//! Render pipeline helpers.

use crate::compat::CompatReport;
use crate::document::Document;
use crate::document::css::{Origin, Rule, parse_stylesheet};
use crate::layout::LayoutTree;
use crate::snapshot::Snapshot;

use crate::api::Element;

/// Collects author stylesheets from the document.
pub(crate) fn author_rules(
    document: &Document,
    snapshot: &Snapshot,
    compat: &mut CompatReport,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    for node in document.descendants() {
        let Some(element) = document.element(node) else {
            continue;
        };
        match element.name.as_str() {
            "style" => {
                let kind = document.attr(node, "type").unwrap_or("text/css");
                if !kind.is_empty() && !kind.eq_ignore_ascii_case("text/css") {
                    compat.unsupported(format!("style type: {kind}"));
                    continue;
                }
                if document.attr(node, "media").is_some() {
                    compat.unsupported("style media attribute");
                }
                let css = document.text_content(node);
                rules.extend(parse_stylesheet(&css, Origin::Author, compat));
            }
            "link" => {
                let rel = document.attr(node, "rel").unwrap_or_default();
                if !rel
                    .split_ascii_whitespace()
                    .any(|value| value.eq_ignore_ascii_case("stylesheet"))
                {
                    continue;
                }
                let Some(href) = document.attr(node, "href") else {
                    continue;
                };
                if document.attr(node, "media").is_some() {
                    compat.unsupported("link media attribute");
                }
                if let Some(stylesheet) = snapshot
                    .stylesheets
                    .iter()
                    .find(|stylesheet| stylesheet.href == href)
                {
                    rules.extend(parse_stylesheet(&stylesheet.css, Origin::Author, compat));
                } else {
                    compat.unsupported(format!("stylesheet: {href}"));
                }
            }
            _ => {}
        }
    }
    rules
}

/// Builds the public element index from the layout tree.
pub(crate) fn elements(document: &Document, layout: &LayoutTree) -> Vec<Element> {
    let mut elements = Vec::new();
    for layout_box in layout.boxes() {
        let Some(element) = document.element(layout_box.node) else {
            continue;
        };
        let mut text = if element.name == "input" {
            document
                .attr(layout_box.node, "value")
                .or_else(|| document.attr(layout_box.node, "placeholder"))
                .unwrap_or_default()
                .to_owned()
        } else if element.name == "textarea" {
            document
                .attr(layout_box.node, "value")
                .map_or_else(|| document.text_content(layout_box.node), str::to_owned)
        } else {
            document.visible_text(layout_box.node)
        };
        text = text.trim().to_owned();
        if text.len() > 512 {
            let mut boundary = 512;
            while !text.is_char_boundary(boundary) {
                boundary -= 1;
            }
            text.truncate(boundary);
        }
        elements.push(Element {
            node: layout_box.node.0,
            tag: element.name.clone(),
            id: document.attr(layout_box.node, "id").map(str::to_owned),
            classes: document
                .classes(layout_box.node)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            rect: layout_box.rect,
            text: (!text.is_empty()).then_some(text),
        });
    }
    elements
}
