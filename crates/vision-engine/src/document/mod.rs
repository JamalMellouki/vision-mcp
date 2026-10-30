//! Document model, selectors, and style computation.

pub(crate) mod css;
mod dom;
mod html;
pub(crate) mod selector;
pub(crate) mod style;

pub(crate) use dom::{Document, NodeId, NodeKind};

/// The user-agent stylesheet.
pub(crate) const UA_CSS: &str = include_str!("ua.css");
