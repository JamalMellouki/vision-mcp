//! Browser capture for Vision snapshots.
//!
//! The adapter launches a local Chromium over the DevTools protocol and returns
//! the final DOM, stylesheets, resources, viewport, and initial scroll state.
//! The renderer never depends on the browser, and no page scripts run inside
//! `vision-engine`.

mod browser;
mod capture;
mod cdp;
mod error;

pub use capture::{CaptureOptions, WaitStrategy, capture};
pub use error::{Error, Result};
