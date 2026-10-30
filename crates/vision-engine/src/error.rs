//! Error type shared by the public API.

/// Errors produced while loading, parsing, laying out, or painting a page.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A resource could not be read.
    #[error("failed to read {path}: {source}")]
    Io {
        /// The resource path or URL.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The HTML document could not be parsed.
    #[error("failed to parse HTML: {0}")]
    Html(String),
    /// A stylesheet could not be parsed.
    #[error("failed to parse CSS: {0}")]
    Css(String),
    /// A required resource is missing.
    #[error("missing resource: {0}")]
    MissingResource(String),
    /// A configured limit was exceeded.
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
    /// The requested operation is not supported.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The input or configuration is invalid.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// Painting failed.
    #[error("render failed: {0}")]
    Render(String),
}

/// The result type used by Vision.
pub type Result<T> = std::result::Result<T, Error>;
