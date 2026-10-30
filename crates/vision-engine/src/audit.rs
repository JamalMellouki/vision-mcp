//! Layout and accessibility audits.

use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::document::style::{Display, Overflow, StyleTree, Visibility};
use crate::document::{Document, NodeId};
use crate::geom::Rect;
use crate::layout::{BoxId, LayoutTree, LineItem};

/// The kind of an audit issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// A click target is too small.
    Target,
    /// A form control has no accessible name.
    Label,
    /// A child extends beyond its parent.
    Overflow,
    /// Text extends beyond its container.
    Clipped,
    /// An element is empty.
    Empty,
    /// Text contrast is below WCAG AA.
    Contrast,
    /// Sibling boxes overlap.
    Overlap,
}

/// A single audit finding.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    /// The kind of issue.
    pub kind: IssueKind,
    /// The DOM node index, when the issue belongs to an element.
    pub node: Option<u32>,
    /// A human-readable description.
    pub message: String,
    /// The affected rectangle, when known.
    pub rect: Option<Rect>,
}

/// The result of an audit.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditReport {
    /// Findings in document order.
    pub issues: Vec<Issue>,
    /// A score from 0 to 100.
    ///
    /// The score is zero when the render used unsupported layout features and
    /// therefore cannot be treated as an accurate audit target.
    pub score: u8,
    /// Whether unsupported features made the render approximate.
    pub blocked: bool,
}

/// Runs the audit checks.
pub(crate) fn audit(
    document: &Document,
    styles: &StyleTree,
    layout: &LayoutTree,
    compat: &crate::compat::CompatReport,
) -> AuditReport {
    let mut issues = Vec::new();

    for (index, layout_box) in layout.boxes().iter().enumerate() {
        let node = layout_box.node;
        let box_id = BoxId(index as u32);
        let Some(element) = document.element(node) else {
            continue;
        };
        let style = styles.get(node);

        if style.display == Display::None || style.visibility == Visibility::Hidden {
            continue;
        }

        if is_interactive(&element.name) {
            let rect = layout_box.rect;
            if rect.width < 24.0 || rect.height < 24.0 {
                issues.push(Issue {
                    kind: IssueKind::Target,
                    node: Some(node.0),
                    message: format!(
                        "<{}> target is {:.0}x{:.0}px; minimum is 24x24px",
                        element.name, rect.width, rect.height
                    ),
                    rect: Some(rect),
                });
            }
        }

        if matches!(element.name.as_str(), "input" | "select" | "textarea") {
            let has_name = document.attr(node, "aria-label").is_some()
                || document.attr(node, "aria-labelledby").is_some()
                || document.attr(node, "title").is_some()
                || document.attr(node, "placeholder").is_some()
                || has_label_ancestor(document, node);
            if !has_name {
                issues.push(Issue {
                    kind: IssueKind::Label,
                    node: Some(node.0),
                    message: format!("<{}> has no accessible name", element.name),
                    rect: Some(layout_box.rect),
                });
            }
        }

        if layout_box.style.overflow != Overflow::Hidden {
            for child in &layout_box.children {
                let child_rect = layout.get(*child).rect;
                if child_rect.right() > layout_box.content.right() + 1.0
                    || child_rect.bottom() > layout_box.content.bottom() + 1.0
                {
                    issues.push(Issue {
                        kind: IssueKind::Overflow,
                        node: Some(node.0),
                        message: format!("<{}> overflows its parent", element.name),
                        rect: Some(child_rect),
                    });
                }
            }
            for (index, left) in layout_box.children.iter().enumerate() {
                let left_rect = layout.get(*left).rect;
                for right in layout_box.children.iter().skip(index + 1) {
                    let Some(overlap) = left_rect.intersection(layout.get(*right).rect) else {
                        continue;
                    };
                    if overlap.width <= 1.0 || overlap.height <= 1.0 {
                        continue;
                    }
                    let left_name = document
                        .element_name(layout.get(*left).node)
                        .unwrap_or("box");
                    let right_name = document
                        .element_name(layout.get(*right).node)
                        .unwrap_or("box");
                    issues.push(Issue {
                        kind: IssueKind::Overlap,
                        node: Some(node.0),
                        message: format!("<{left_name}> overlaps <{right_name}>"),
                        rect: Some(overlap),
                    });
                }
            }
        }

        for line in &layout_box.lines {
            for item in &line.items {
                let LineItem::Text(fragment) = item else {
                    continue;
                };
                if layout_box.style.overflow != Overflow::Hidden
                    && fragment.x + fragment.width > layout_box.content.right() + 1.0
                {
                    issues.push(Issue {
                        kind: IssueKind::Clipped,
                        node: Some(node.0),
                        message: "text extends beyond its container".to_owned(),
                        rect: Some(Rect::new(
                            fragment.x,
                            line.baseline - fragment.style.font_size,
                            fragment.width,
                            fragment.style.font_size,
                        )),
                    });
                }
                let background = effective_background(layout, Some(box_id));
                if let Some(issue) = contrast_issue(node, background, fragment) {
                    issues.push(issue);
                }
            }
        }

        if layout_box.children.is_empty()
            && layout_box.lines.is_empty()
            && !is_void(&element.name)
            && document.text_content(node).trim().is_empty()
            && layout_box.style.background.a == 0
            && layout_box.style.border.widths().top == 0.0
            && layout_box.style.border.widths().right == 0.0
            && layout_box.style.border.widths().bottom == 0.0
            && layout_box.style.border.widths().left == 0.0
        {
            issues.push(Issue {
                kind: IssueKind::Empty,
                node: Some(node.0),
                message: format!("<{}> is empty", element.name),
                rect: Some(layout_box.rect),
            });
        }
    }

    let mut seen = std::collections::HashSet::new();
    issues.retain(|issue| seen.insert((issue.kind, issue.node, issue.message.clone())));

    let blocked = !compat.unsupported.is_empty();
    let score = if blocked {
        0
    } else {
        100_u8.saturating_sub((issues.len() as u8).saturating_mul(3))
    };
    AuditReport {
        issues,
        score,
        blocked,
    }
}

fn is_interactive(name: &str) -> bool {
    matches!(
        name,
        "a" | "button" | "input" | "select" | "textarea" | "summary"
    )
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn has_label_ancestor(document: &Document, node: NodeId) -> bool {
    let mut parent = document.parent(node);
    while let Some(current) = parent {
        if document.element_name(current) == Some("label") {
            return true;
        }
        parent = document.parent(current);
    }
    false
}

fn effective_background(layout: &LayoutTree, parent: Option<BoxId>) -> Color {
    let mut current = parent;
    while let Some(box_id) = current {
        let background = layout.get(box_id).style.background;
        if background.a >= 128 {
            return background;
        }
        current = layout.get(box_id).parent;
    }
    Color::WHITE
}

fn contrast_issue(
    node: NodeId,
    background: Color,
    fragment: &crate::layout::TextFragment,
) -> Option<Issue> {
    if fragment.style.color.a < 128 {
        return None;
    }
    let background = if background.a < 128 {
        Color::WHITE
    } else {
        background
    };
    let ratio = contrast_ratio(fragment.style.color, background);
    let threshold = if fragment.style.font_size >= 24.0
        || (fragment.style.font_size >= 18.66 && fragment.style.font_weight >= 700)
    {
        3.0
    } else {
        4.5
    };
    if ratio >= threshold {
        return None;
    }
    Some(Issue {
        kind: IssueKind::Contrast,
        node: Some(node.0),
        message: format!("contrast ratio {ratio:.2}:1 is below {threshold:.1}:1"),
        rect: None,
    })
}

fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (lighter, darker) = {
        let a = luminance(a);
        let b = luminance(b);
        if a > b { (a, b) } else { (b, a) }
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn luminance(color: Color) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}
