//! Vision: HTML and CSS rendering, layout auditing, and page sessions.
//!
//! The engine parses a self-contained [`Snapshot`] and produces a [`Capture`].
//! Trusted local scripts run in an embedded QuickJS runtime with a small DOM
//! facade. The engine performs no network or filesystem I/O after construction.
//! The runtime is not a security sandbox; do not feed it untrusted pages.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod api;
mod audit;
mod color;
mod compat;
mod document;
mod error;
mod geom;
mod image;
mod js;
mod layout;
mod paint;
mod render;
mod session;
mod snapshot;
mod text;

pub use api::{Capture, Config, Element, Engine, Timings};
pub use audit::{AuditReport, Issue, IssueKind};
pub use compat::CompatReport;
pub use error::{Error, Result};
pub use geom::{Edges, Point, Rect, Size};
pub use session::{Action, Session, Target};
pub use snapshot::{MAX_SNAPSHOT_BYTES, Resource, Snapshot, State, Stylesheet, Viewport};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
