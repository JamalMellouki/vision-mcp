//! Input document and render state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::geom::Point;

mod resource;

pub use resource::Resource;

/// Maximum HTML snapshot size accepted by [`Snapshot::from_file`].
pub const MAX_SNAPSHOT_BYTES: u64 = 32 * 1024 * 1024;

/// A viewport in CSS pixels and a device pixel ratio.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Width in CSS pixels.
    pub width: u32,
    /// Height in CSS pixels.
    pub height: u32,
    /// Device pixel ratio used for output.
    pub dpr: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            dpr: 1.0,
        }
    }
}

impl Viewport {
    /// Creates a viewport with a device pixel ratio of one.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            dpr: 1.0,
        }
    }

    /// Returns the viewport with a different device pixel ratio.
    #[must_use]
    pub fn with_dpr(mut self, dpr: f32) -> Self {
        self.dpr = dpr;
        self
    }

    /// Returns the width in CSS pixels.
    #[must_use]
    pub fn css_width(self) -> f32 {
        self.width as f32
    }

    /// Returns the height in CSS pixels.
    #[must_use]
    pub fn css_height(self) -> f32 {
        self.height as f32
    }
}

/// A stylesheet that belongs to a snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stylesheet {
    /// The original `href`, or an empty string for an inline stylesheet.
    pub href: String,
    /// The stylesheet source.
    pub css: String,
}

/// Form values, focus, and scroll offsets carried between renders.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// Document scroll offset.
    pub scroll: Point,
    /// Focused element selector, if any.
    pub focus: Option<String>,
    /// Form values keyed by element selector.
    pub values: BTreeMap<String, String>,
}

/// A self-contained page input.
///
/// A snapshot contains the HTML, the external stylesheets that were loaded for
/// it, and the initial viewport and state. The renderer performs no network or
/// filesystem access of its own.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// The HTML source.
    pub html: String,
    /// Stylesheets referenced by the document, preloaded by the caller.
    pub stylesheets: Vec<Stylesheet>,
    /// Binary resources referenced by the document, preloaded by the caller.
    pub resources: Vec<Resource>,
    /// The initial viewport.
    pub viewport: Viewport,
    /// The initial form and scroll state.
    pub state: State,
    /// Non-fatal problems encountered while preloading a file snapshot.
    ///
    /// The engine copies these into the compatibility report and treats them
    /// as blocking for audits, so a missing stylesheet or image can never look
    /// like a clean render.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Snapshot {
    /// Creates a snapshot from an HTML string and the default viewport.
    #[must_use]
    pub fn from_html(html: impl Into<String>) -> Self {
        Self {
            html: html.into(),
            stylesheets: Vec::new(),
            resources: Vec::new(),
            viewport: Viewport::default(),
            state: State::default(),
            warnings: Vec::new(),
        }
    }

    /// Reads an HTML file and preloads local stylesheets referenced by it.
    ///
    /// Remote stylesheets and files outside the document directory are not
    /// loaded. Every skipped or unreadable resource is recorded in
    /// [`Snapshot::warnings`] and later placed in the compatibility report, so
    /// it cannot silently produce a clean-looking render.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        if metadata.len() > MAX_SNAPSHOT_BYTES {
            return Err(Error::Limit("snapshot file exceeds 32 MiB"));
        }
        let html = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        if html.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(Error::Limit("snapshot HTML exceeds 32 MiB"));
        }
        let document = Document::parse(&html)?;
        let base = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from("."));
        let mut stylesheets = Vec::new();
        let mut warnings = Vec::new();

        for node in document.descendants() {
            let Some(element) = document.element(node) else {
                continue;
            };
            if element.name != "link" {
                continue;
            }
            let rel = document.attr(node, "rel").unwrap_or_default();
            if !rel
                .split_ascii_whitespace()
                .any(|value| value.eq_ignore_ascii_case("stylesheet"))
            {
                continue;
            }
            let Some(href) = document.attr(node, "href") else {
                warnings.push("stylesheet: <link> has no href".to_owned());
                continue;
            };
            let resolved = match resolve_local(&base, href) {
                Ok(path) => path,
                Err(reason) => {
                    warnings.push(format!("stylesheet not loaded ({reason}): {href}"));
                    continue;
                }
            };
            match std::fs::read_to_string(&resolved) {
                Ok(css) => stylesheets.push(Stylesheet {
                    href: href.to_owned(),
                    css,
                }),
                Err(source) => warnings.push(format!("stylesheet unreadable: {href}: {source}")),
            }
        }

        let mut resources = Vec::new();
        for node in document.descendants() {
            let Some(name) = document.element_name(node) else {
                continue;
            };
            let attribute = match name {
                "img" | "script" => "src",
                _ => continue,
            };
            let Some(src) = document.attr(node, attribute) else {
                continue;
            };
            let resolved = match resolve_local(&base, src) {
                Ok(path) => path,
                Err(reason) => {
                    warnings.push(format!("{name} resource not loaded ({reason}): {src}"));
                    continue;
                }
            };
            match std::fs::read(&resolved) {
                Ok(bytes) => resources.push(Resource {
                    url: src.to_owned(),
                    bytes,
                }),
                Err(source) => {
                    warnings.push(format!("{name} resource unreadable: {src}: {source}"))
                }
            }
        }

        Ok(Self {
            html,
            stylesheets,
            resources,
            viewport: Viewport::default(),
            state: State::default(),
            warnings,
        })
    }

    /// Sets the viewport.
    #[must_use]
    pub fn with_viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = viewport;
        self
    }

    /// Adds a stylesheet to the snapshot.
    #[must_use]
    pub fn with_stylesheet(mut self, href: impl Into<String>, css: impl Into<String>) -> Self {
        self.stylesheets.push(Stylesheet {
            href: href.into(),
            css: css.into(),
        });
        self
    }

    /// Adds an image resource to the snapshot.
    #[must_use]
    pub fn with_resource(mut self, url: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.resources.push(Resource {
            url: url.into(),
            bytes: bytes.into(),
        });
        self
    }
}

fn resolve_local(base: &Path, raw: &str) -> std::result::Result<PathBuf, &'static str> {
    if raw.is_empty()
        || raw.starts_with('#')
        || raw.starts_with("//")
        || raw.starts_with("data:")
        || raw.contains("://")
    {
        return Err("not a local file");
    }
    let canonical = base
        .join(raw)
        .canonicalize()
        .map_err(|_| "file not found")?;
    if canonical.starts_with(base) {
        Ok(canonical)
    } else {
        Err("outside the document directory")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::error::Error;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn from_html_uses_default_viewport() {
        let snapshot = Snapshot::from_html("<h1>Hello</h1>");
        assert_eq!(snapshot.viewport, Viewport::default());
        assert!(snapshot.stylesheets.is_empty());
        assert!(snapshot.resources.is_empty());
        assert!(snapshot.warnings.is_empty());
    }

    #[test]
    fn from_file_does_not_escape_the_document_directory() -> std::result::Result<(), Box<dyn Error>>
    {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let root = std::env::temp_dir().join(format!(
            "vision-snapshot-escape-{}-{stamp}",
            std::process::id()
        ));
        let child = root.join("child");
        std::fs::create_dir_all(&child)?;
        std::fs::write(root.join("secret.css"), "body { color: #ff0000; }")?;
        std::fs::write(root.join("secret.txt"), b"private")?;
        std::fs::write(
            child.join("index.html"),
            "<link rel=\"stylesheet\" href=\"../secret.css\"><img src=\"../secret.txt\">",
        )?;

        let snapshot = Snapshot::from_file(child.join("index.html"))?;
        assert!(snapshot.stylesheets.is_empty());
        assert!(snapshot.resources.is_empty());
        assert_eq!(snapshot.warnings.len(), 2);
        assert!(
            snapshot
                .warnings
                .iter()
                .all(|warning| warning.contains("outside the document directory"))
        );

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn from_file_records_missing_local_resources() -> std::result::Result<(), Box<dyn Error>> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let directory = std::env::temp_dir().join(format!(
            "vision-snapshot-missing-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("index.html"),
            "<link rel=\"stylesheet\" href=\"missing.css\"><img src=\"missing.png\">",
        )?;

        let snapshot = Snapshot::from_file(directory.join("index.html"))?;
        assert!(snapshot.stylesheets.is_empty());
        assert!(snapshot.resources.is_empty());
        assert_eq!(snapshot.warnings.len(), 2);
        assert!(
            snapshot
                .warnings
                .iter()
                .any(|warning| warning.contains("missing.css"))
        );
        assert!(
            snapshot
                .warnings
                .iter()
                .any(|warning| warning.contains("missing.png"))
        );

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn from_file_loads_stylesheets_and_images() -> std::result::Result<(), Box<dyn Error>> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let directory =
            std::env::temp_dir().join(format!("vision-snapshot-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&directory)?;
        std::fs::write(
            directory.join("index.html"),
            "<link rel=\"stylesheet\" href=\"style.css\"><img src=\"dot.png\">",
        )?;
        std::fs::write(directory.join("style.css"), "body { color: #000000; }")?;
        std::fs::write(directory.join("dot.png"), b"not-an-image-yet")?;

        let snapshot = Snapshot::from_file(directory.join("index.html"))?;
        assert_eq!(snapshot.stylesheets.len(), 1);
        assert_eq!(snapshot.stylesheets[0].href, "style.css");
        assert_eq!(snapshot.resources.len(), 1);
        assert_eq!(snapshot.resources[0].url, "dot.png");
        assert_eq!(snapshot.resources[0].bytes, b"not-an-image-yet");

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }
}
