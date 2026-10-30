//! Errors produced while capturing a browser page.

use thiserror::Error;

/// A browser capture failure.
#[derive(Debug, Error)]
pub enum Error {
    /// The browser could not be found.
    #[error("browser not found; set VISION_BROWSER to a Chromium executable")]
    BrowserNotFound,
    /// The browser exited before DevTools became available.
    #[error("browser exited before the DevTools endpoint was ready")]
    BrowserExited,
    /// The requested URL is not allowed.
    #[error("URL is not allowed: {0}")]
    DisallowedUrl(String),
    /// The page could not be navigated.
    #[error("navigation failed: {0}")]
    Navigation(String),
    /// The DevTools protocol returned an error.
    #[error("CDP error: {0}")]
    Protocol(String),
    /// The capture timed out.
    #[error("capture timed out: {0}")]
    Timeout(String),
    /// The page returned data that could not be parsed.
    #[error("invalid page data: {0}")]
    Data(String),
    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A WebSocket error.
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tungstenite::Error),
    /// A JSON error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A URL parse error.
    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),
}

/// A browser capture result.
pub type Result<T> = std::result::Result<T, Error>;
