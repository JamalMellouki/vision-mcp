//! Public engine API.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::audit::{AuditReport, audit};
use crate::compat::CompatReport;
use crate::document::Document;
use crate::document::UA_CSS;
use crate::document::css::{Origin, Rule, parse_stylesheet};
use crate::document::selector::parse_selector_list;
use crate::document::style::StyleTree;
use crate::error::{Error, Result};
use crate::geom::Rect;
use crate::image::ImageStore;
use crate::layout::{LayoutTree, build_layout};
use crate::paint::paint;
use crate::render::{author_rules, elements};
use crate::snapshot::Snapshot;
use crate::text::FontBook;

use tiny_skia::Pixmap;

/// Renderer configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Run layout and accessibility checks.
    pub audit: bool,
    /// Build the element index.
    pub elements: bool,
    /// Execute inline and preloaded scripts with QuickJS.
    pub javascript: bool,
    /// Maximum HTML input size in bytes.
    pub max_html_bytes: usize,
    /// Wall-clock deadline for each script execution and event dispatch.
    pub script_timeout: Duration,
    /// QuickJS heap limit in bytes.
    pub script_memory_bytes: usize,
    /// QuickJS stack limit in bytes.
    pub script_stack_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audit: false,
            elements: true,
            javascript: true,
            max_html_bytes: 32 * 1024 * 1024,
            script_timeout: Duration::from_secs(2),
            script_memory_bytes: 64 * 1024 * 1024,
            script_stack_bytes: 2 * 1024 * 1024,
        }
    }
}

/// A reusable renderer.
pub struct Engine {
    pub(crate) ua_rules: Vec<Rule>,
    pub(crate) fonts: FontBook,
    pub(crate) config: Config,
}

/// Parsed and laid-out state shared by rendering and sessions.
pub(crate) struct Prepared {
    pub(crate) document: crate::js::SharedDocument,
    pub(crate) script: Option<crate::js::ScriptRuntime>,
    pub(crate) author_rules: Vec<Rule>,
    pub(crate) styles: StyleTree,
    pub(crate) layout: LayoutTree,
    pub(crate) images: ImageStore,
    pub(crate) compat: CompatReport,
    pub(crate) timings: Timings,
}

impl Engine {
    /// Creates an engine with system fonts and the built-in user-agent styles.
    pub fn native(config: Config) -> Result<Self> {
        let mut compat = CompatReport::default();
        let ua_rules = parse_stylesheet(UA_CSS, Origin::UserAgent, &mut compat);
        let fonts = FontBook::new()?;
        Ok(Self {
            ua_rules,
            fonts,
            config,
        })
    }

    /// Parses, styles, and lays out a snapshot.
    pub(crate) fn prepare(&self, snapshot: &Snapshot) -> Result<Prepared> {
        let total_start = Instant::now();
        if snapshot.html.len() > self.config.max_html_bytes {
            return Err(Error::Limit("HTML exceeds configured max_html_bytes"));
        }

        let parse_start = Instant::now();
        let document = Rc::new(RefCell::new(Document::parse(&snapshot.html)?));
        apply_state(&mut document.borrow_mut(), &snapshot.state);
        let mut compat = CompatReport::default();
        for warning in snapshot.warnings.iter().take(32) {
            compat.unsupported(format!("resource: {warning}"));
            compat.warn(warning.clone());
        }
        let script = if self.config.javascript {
            crate::js::execute_scripts(
                &document,
                snapshot,
                &mut compat,
                crate::js::ScriptLimits {
                    timeout: self.config.script_timeout,
                    memory_bytes: self.config.script_memory_bytes,
                    stack_bytes: self.config.script_stack_bytes,
                },
            )
        } else {
            let has_script = {
                let document = document.borrow();
                document
                    .descendants()
                    .any(|node| document.element_name(node) == Some("script"))
            };
            if has_script {
                compat.unsupported("script: disabled by configuration");
            }
            None
        };
        let parse_ms = elapsed_ms(parse_start);

        let images = ImageStore::new(snapshot);
        let (author_rules, styles, layout, style_ms, layout_ms) = {
            let document = document.borrow();
            let style_start = Instant::now();
            let author_rules = author_rules(&document, snapshot, &mut compat);
            let styles = StyleTree::build(
                &document,
                &self.ua_rules,
                &author_rules,
                snapshot.viewport,
                &mut compat,
            );
            let style_ms = elapsed_ms(style_start);

            let layout_start = Instant::now();
            let layout = build_layout(
                &document,
                &styles,
                &self.fonts,
                snapshot.viewport,
                &images,
                &mut compat,
            );
            let layout_ms = elapsed_ms(layout_start);
            (author_rules, styles, layout, style_ms, layout_ms)
        };

        Ok(Prepared {
            document,
            script,
            author_rules,
            styles,
            layout,
            images,
            compat,
            timings: Timings {
                parse_ms,
                style_ms,
                layout_ms,
                paint_ms: 0.0,
                total_ms: elapsed_ms(total_start),
            },
        })
    }

    /// Renders a snapshot to PNG and optional metadata.
    pub fn render(&self, snapshot: &Snapshot) -> Result<Capture> {
        let mut prepared = self.prepare(snapshot)?;
        let paint_start = Instant::now();
        let pixmap = paint(
            &prepared.layout,
            snapshot.viewport,
            snapshot.state.scroll,
            &prepared.document.borrow(),
            &mut prepared.compat,
        )?;
        let paint_ms = elapsed_ms(paint_start);
        prepared.capture(pixmap, &self.config, paint_ms)
    }

    /// Opens an interactive session. The engine is consumed because a session
    /// owns the font database and configuration it renders with.
    pub fn open(self, snapshot: Snapshot) -> Result<crate::session::Session> {
        let prepared = self.prepare(&snapshot)?;
        Ok(crate::session::Session::new(self, snapshot, prepared))
    }
}

fn apply_state(document: &mut Document, state: &crate::snapshot::State) {
    for (raw, value) in &state.values {
        let selectors = parse_selector_list(raw);
        if selectors.is_empty() {
            continue;
        }
        let nodes = document
            .descendants()
            .filter(|node| {
                document.element(*node).is_some()
                    && selectors
                        .iter()
                        .any(|selector| selector.matches(document, *node))
            })
            .collect::<Vec<_>>();
        for node in nodes {
            if matches!(
                document.element_name(node),
                Some("input" | "textarea" | "select")
            ) {
                document.set_attr(node, "value", value.clone());
            }
        }
    }
}

impl Prepared {
    pub(crate) fn capture(
        &self,
        pixmap: Pixmap,
        config: &Config,
        paint_ms: f64,
    ) -> Result<Capture> {
        let png = pixmap
            .encode_png()
            .map_err(|error| Error::Render(error.to_string()))?;
        let elements = if config.elements {
            elements(&self.document.borrow(), &self.layout)
        } else {
            Vec::new()
        };
        let audit = config.audit.then(|| {
            audit(
                &self.document.borrow(),
                &self.styles,
                &self.layout,
                &self.compat,
            )
        });
        Ok(Capture {
            png,
            width: pixmap.width(),
            height: pixmap.height(),
            document_height: self.layout.height(),
            elements,
            audit,
            compat: self.compat.clone(),
            timings: Timings {
                paint_ms,
                total_ms: self.timings.total_ms + paint_ms,
                ..self.timings
            },
        })
    }
}

/// A rendered page.
#[derive(Clone, Debug, Serialize)]
pub struct Capture {
    /// PNG bytes.
    #[serde(skip)]
    pub png: Vec<u8>,
    /// Output width in device pixels.
    pub width: u32,
    /// Output height in device pixels.
    pub height: u32,
    /// Full document height in CSS pixels.
    pub document_height: f32,
    /// Element rectangles in document order.
    pub elements: Vec<Element>,
    /// Audit results, when enabled.
    pub audit: Option<AuditReport>,
    /// Unsupported features encountered during the render.
    pub compat: CompatReport,
    /// Phase timings.
    pub timings: Timings,
}

/// A rendered element.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Element {
    /// DOM node index.
    pub node: u32,
    /// Lowercase tag name.
    pub tag: String,
    /// Element id.
    pub id: Option<String>,
    /// Element classes.
    pub classes: Vec<String>,
    /// Border box in CSS pixels.
    pub rect: Rect,
    /// Text content, truncated to 512 bytes.
    pub text: Option<String>,
}

/// Render phase timings in milliseconds.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Timings {
    /// HTML parsing.
    pub parse_ms: f64,
    /// Style computation.
    pub style_ms: f64,
    /// Layout.
    pub layout_ms: f64,
    /// Rasterization.
    pub paint_ms: f64,
    /// Total time.
    pub total_ms: f64,
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Viewport;

    use std::error::Error;

    #[test]
    fn renders_png_and_elements() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot =
            Snapshot::from_html("<style>h1 { color: #336699; }</style><h1>Hello</h1><p>Body</p>")
                .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        assert!(capture.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(capture.width, 400);
        assert_eq!(capture.height, 300);
        assert!(capture.elements.iter().any(|element| element.tag == "h1"));
        assert!(capture.compat.is_empty());
        Ok(())
    }

    #[test]
    fn element_index_reports_form_values() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<input id=\"name\" value=\"Hello\"><textarea id=\"note\">Note</textarea>",
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let name = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("name"))
            .ok_or("name missing")?;
        assert_eq!(name.text.as_deref(), Some("Hello"));
        let note = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("note"))
            .ok_or("note missing")?;
        assert_eq!(note.text.as_deref(), Some("Note"));
        Ok(())
    }

    #[test]
    fn script_value_property_updates_form_value() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<input id=\"name\"><script>document.getElementById(\"name\").value = \"Ada\";</script>",
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let name = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("name"))
            .ok_or("name missing")?;
        assert_eq!(name.text.as_deref(), Some("Ada"));
        Ok(())
    }

    #[test]
    fn applies_initial_form_values() -> std::result::Result<(), Box<dyn Error>> {
        let mut snapshot =
            Snapshot::from_html("<input id=\"name\">").with_viewport(Viewport::new(320, 200));
        snapshot
            .state
            .values
            .insert("#name".to_owned(), "ready".to_owned());
        let session = Engine::native(Config::default())?.open(snapshot)?;
        let element = session
            .find(&crate::Target::Id("name".to_owned()))
            .unwrap_or_else(|| panic!("input missing"));
        assert_eq!(element.text.as_deref(), Some("ready"));
        Ok(())
    }

    #[test]
    fn places_form_controls_on_the_text_line() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html("<label>Name <input id=\"name\"></label>")
            .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let input = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("name"))
            .ok_or("input missing")?;
        assert!(input.rect.x > 8.0);
        assert!(input.rect.y < 40.0);
        assert!(input.rect.width > 100.0);
        assert!(
            !capture
                .compat
                .unsupported
                .iter()
                .any(|item| item.contains("atomic child") || item.contains("inline element"))
        );
        Ok(())
    }

    #[test]
    fn places_inline_blocks_side_by_side() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div><span id=\"one\" style=\"display:inline-block;width:20px;height:20px\"></span><span id=\"two\" style=\"display:inline-block;width:20px;height:20px\"></span></div>",
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let one = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("one"))
            .ok_or("first inline-block missing")?;
        let two = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("two"))
            .ok_or("second inline-block missing")?;
        assert!((one.rect.y - two.rect.y).abs() < 0.5);
        assert!((two.rect.x - (one.rect.x + 20.0)).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn keeps_nested_atomic_inlines_inside_their_parent() -> std::result::Result<(), Box<dyn Error>>
    {
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([20, 80, 200, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )?;
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div><span id=\"outer\" style=\"display:inline-block;padding:2px\"><img src=\"dot.png\" width=\"8\" height=\"8\"></span><span id=\"next\" style=\"display:inline-block;width:10px;height:10px\"></span></div>",
        )
        .with_resource("dot.png", bytes)
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let outer = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("outer"))
            .ok_or("outer inline-block missing")?;
        let next = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("next"))
            .ok_or("next inline-block missing")?;
        assert!((outer.rect.width - 12.0).abs() < 0.5);
        assert!(
            (next.rect.x - (outer.rect.x + outer.rect.width)).abs() < 0.5,
            "outer {:?}, next {:?}",
            outer.rect,
            next.rect
        );
        assert!(
            !capture
                .compat
                .unsupported
                .iter()
                .any(|item| item.contains("inline element") || item.contains("atomic child"))
        );
        Ok(())
    }

    #[test]
    fn wraps_atomic_inlines_at_the_content_width() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"width:50px\"><span id=\"one\" style=\"display:inline-block;width:30px;height:20px\"></span><span id=\"two\" style=\"display:inline-block;width:30px;height:20px\"></span></div>",
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let one = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("one"))
            .ok_or("first inline-block missing")?;
        let two = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("two"))
            .ok_or("second inline-block missing")?;
        assert!(two.rect.y >= one.rect.y + 19.0);
        assert!((two.rect.x - one.rect.x).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn shrinks_automatic_inline_blocks_to_their_content() -> std::result::Result<(), Box<dyn Error>>
    {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div><span id=\"chip\" style=\"display:inline-block;padding:4px\">Chip</span></div>",
        )
        .with_viewport(Viewport::new(1000, 200));
        let capture = engine.render(&snapshot)?;
        let chip = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("chip"))
            .ok_or("chip missing")?;
        assert!(chip.rect.width > 10.0);
        assert!(chip.rect.width < 200.0);
        assert!(chip.rect.height >= 8.0);
        Ok(())
    }

    #[test]
    fn places_images_in_the_inline_flow() -> std::result::Result<(), Box<dyn Error>> {
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([20, 80, 200, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )?;
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<p id=\"line\">Hello <img id=\"dot\" src=\"dot.png\" width=\"8\" height=\"8\"></p>",
        )
        .with_resource("dot.png", bytes)
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let paragraph = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("line"))
            .ok_or("paragraph missing")?;
        let dot = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("dot"))
            .ok_or("image missing")?;
        assert_eq!(dot.rect.width, 8.0);
        assert!(dot.rect.x > paragraph.rect.x);
        assert!(dot.rect.y > paragraph.rect.y);
        assert!(dot.rect.bottom() <= paragraph.rect.bottom());
        assert!(
            !capture
                .compat
                .unsupported
                .iter()
                .any(|item| item.contains("inline element"))
        );
        Ok(())
    }

    #[test]
    fn lays_out_table_cells_in_columns() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html("<table><tr><td>Left</td><td>Right</td></tr></table>")
            .with_viewport(Viewport::new(600, 400));
        let capture = engine.render(&snapshot)?;
        let cells = capture
            .elements
            .iter()
            .filter(|element| element.tag == "td")
            .collect::<Vec<_>>();
        assert_eq!(cells.len(), 2);
        assert!((cells[0].rect.y - cells[1].rect.y).abs() < 0.5);
        assert!(cells[0].rect.x < cells[1].rect.x);
        Ok(())
    }

    #[test]
    fn lays_out_captions_and_colspan() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<table><caption>Caption</caption><tr><td colspan=\"2\">Wide</td></tr><tr><td>A</td><td>B</td></tr></table>",
        )
        .with_viewport(Viewport::new(600, 400));
        let capture = engine.render(&snapshot)?;
        assert!(
            capture
                .elements
                .iter()
                .any(|element| element.tag == "caption")
        );
        let cells = capture
            .elements
            .iter()
            .filter(|element| element.tag == "td")
            .collect::<Vec<_>>();
        assert_eq!(cells.len(), 3);
        assert!(cells[0].rect.width > cells[1].rect.width * 1.8);
        assert!(cells[1].rect.x < cells[2].rect.x);
        assert!(capture.compat.unsupported.is_empty());
        Ok(())
    }

    #[test]
    fn audit_finds_small_targets_and_low_contrast() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            elements: true,
            ..Config::default()
        })?;
        let snapshot = Snapshot::from_html(
            "<a href=\"#\" style=\"display:inline-block;width:10px;height:10px\">x</a>\
             <p style=\"color:#777777\">low contrast</p>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let audit = capture.audit.unwrap_or_else(|| panic!("audit missing"));
        assert!(
            audit
                .issues
                .iter()
                .any(|issue| issue.kind == crate::IssueKind::Target)
        );
        assert!(
            audit
                .issues
                .iter()
                .any(|issue| issue.kind == crate::IssueKind::Contrast)
        );
        Ok(())
    }

    #[test]
    fn lays_out_a_flex_row_with_a_gap() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"row\" style=\"display:flex;gap:10px;width:320px\">\
             <div id=\"a\" style=\"width:50px;height:20px\"></div>\
             <div id=\"b\" style=\"width:60px;height:20px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let row = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("row"))
            .ok_or("row missing")?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        assert!((a.rect.y - b.rect.y).abs() < 0.5);
        assert!((b.rect.x - (a.rect.x + 60.0)).abs() < 0.5);
        assert!((row.rect.height - 20.0).abs() < 0.5);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn flex_grow_distributes_free_space() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"row\" style=\"display:flex;width:300px\">\
             <div id=\"a\" style=\"flex:1;height:10px\"></div>\
             <div id=\"b\" style=\"flex:1;height:10px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        assert!((a.rect.width - 150.0).abs() < 0.5);
        assert!((b.rect.width - 150.0).abs() < 0.5);
        assert!((b.rect.x - a.rect.right()).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn flex_justify_content_centers_items() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"row\" style=\"display:flex;justify-content:center;width:300px\">\
             <div id=\"a\" style=\"width:50px;height:10px\"></div>\
             <div id=\"b\" style=\"width:50px;height:10px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let row = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("row"))
            .ok_or("row missing")?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        assert!((a.rect.x - (row.rect.x + 100.0)).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn flex_align_items_centers_items() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"row\" style=\"display:flex;align-items:center;width:300px;height:100px\">\
             <div id=\"a\" style=\"width:50px;height:20px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let row = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("row"))
            .ok_or("row missing")?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        assert!((a.rect.y - (row.rect.y + 40.0)).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn flex_column_stacks_items_with_a_gap() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"column\" style=\"display:flex;flex-direction:column;gap:10px;width:200px\">\
             <div id=\"a\" style=\"height:20px\"></div>\
             <div id=\"b\" style=\"height:20px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        assert!((b.rect.y - a.rect.bottom() - 10.0).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn lays_out_a_repeat_grid() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"grid\" style=\"display:grid;grid-template-columns:repeat(3,1fr);gap:10px;width:320px\">\
             <div id=\"a\" style=\"height:20px\"></div>\
             <div id=\"b\" style=\"height:20px\"></div>\
             <div id=\"c\" style=\"height:20px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        let c = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("c"))
            .ok_or("c missing")?;
        assert!((a.rect.y - b.rect.y).abs() < 0.5);
        assert!((b.rect.y - c.rect.y).abs() < 0.5);
        assert!((a.rect.width - 100.0).abs() < 0.5);
        assert!((b.rect.x - (a.rect.x + 110.0)).abs() < 0.5);
        assert!((c.rect.x - (b.rect.x + 110.0)).abs() < 0.5);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn flex_column_shrinks_a_button_to_its_content() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"display:flex;flex-direction:column;width:300px\">\
             <button id=\"b\" style=\"align-self:start\">Open</button>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let button = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("button missing")?;
        assert!(button.rect.width < 100.0, "{}", button.rect.width);
        Ok(())
    }

    #[test]
    fn border_box_min_height_includes_padding() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"card\" style=\"box-sizing:border-box;min-height:150px;padding:20px\">Hi</div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let card = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("card"))
            .ok_or("card missing")?;
        assert!(
            (card.rect.height - 150.0).abs() < 0.5,
            "{}",
            card.rect.height
        );
        Ok(())
    }

    #[test]
    fn universal_box_sizing_applies_to_min_height() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<style>* { box-sizing: border-box; } .card { min-height: 150px; padding: 20px; }</style>\
             <div id=\"card\" class=\"card\">Hi</div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let card = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("card"))
            .ok_or("card missing")?;
        assert!(
            (card.rect.height - 150.0).abs() < 0.5,
            "{}",
            card.rect.height
        );
        Ok(())
    }

    #[test]
    fn supports_border_radius() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"width:100px;height:50px;background:#3157d5;border-radius:12px\">\
             </div>",
        )
        .with_viewport(Viewport::new(200, 120));
        let capture = engine.render(&snapshot)?;
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn renders_the_react_ssr_corpus() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(include_str!("../../../corpus/react-ssr.html"))
            .with_viewport(Viewport::new(900, 900));
        let capture = engine.render(&snapshot)?;
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        let cards = capture
            .elements
            .iter()
            .filter(|element| element.classes.iter().any(|class| class == "card"))
            .collect::<Vec<_>>();
        assert_eq!(cards.len(), 3);
        assert!(cards[0].rect.width > 200.0, "{:?}", cards[0].rect);
        assert!((cards[0].rect.y - cards[1].rect.y).abs() < 0.5);
        Ok(())
    }

    #[test]
    fn truncates_utf8_element_text_at_a_character_boundary()
    -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let text = "مرحبا بالعالم ".repeat(60);
        let snapshot = Snapshot::from_html(format!("<p id=\"long\">{text}</p>"))
            .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let element = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("long"))
            .ok_or("long text missing")?;
        let captured = element.text.as_deref().ok_or("text missing")?;
        assert!(captured.len() <= 512);
        assert!(captured.is_char_boundary(captured.len()));
        Ok(())
    }

    #[test]
    fn css_image_width_overrides_presentational_attributes()
    -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"width:100px\">\
             <img id=\"image\" src=\"dot.svg\" width=\"500\" height=\"500\" style=\"width:100%;height:auto\">\
             </div>",
        )
        .with_resource("dot.svg", include_bytes!("../../../corpus/dot.svg").to_vec())
        .with_viewport(Viewport::new(300, 300));
        let capture = engine.render(&snapshot)?;
        let image = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("image"))
            .ok_or("image missing")?;
        assert!((image.rect.width - 100.0).abs() < 0.5, "{:?}", image.rect);
        assert!((image.rect.height - 100.0).abs() < 0.5, "{:?}", image.rect);
        Ok(())
    }

    #[test]
    fn form_properties_are_visible_to_scripts() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let source = "<input id=\"name\"><p id=\"status\"></p>\
                      <script>\
                        const input = document.getElementById('name');\
                        input.value = 'Ada';\
                        document.getElementById('status').textContent =\
                          input.type + ':' + input.value + ':' + input.checked;\
                      </script>";
        let snapshot = Snapshot::from_html(source).with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let status = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("status"))
            .ok_or("status missing")?;
        assert_eq!(status.text.as_deref(), Some("text:Ada:false"));
        Ok(())
    }

    #[test]
    fn dom_wrappers_keep_identity_and_expando_properties() -> std::result::Result<(), Box<dyn Error>>
    {
        let engine = Engine::native(Config::default())?;
        let source = "<div id=\"node\"></div><p id=\"status\">pending</p>\
                      <script>\
                        const first = document.getElementById('node');\
                        first.__fiber = { value: 7 };\
                        const second = document.getElementById('node');\
                        document.getElementById('status').textContent =\
                          (first === second) + ':' + second.__fiber.value;\
                      </script>";
        let snapshot = Snapshot::from_html(source).with_viewport(Viewport::new(500, 300));
        let capture = engine.render(&snapshot)?;
        let status = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("status"))
            .ok_or("status missing")?;
        assert_eq!(status.text.as_deref(), Some("true:7"));
        Ok(())
    }

    #[test]
    fn hydration_style_script_preserves_server_dom() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let source = "<div id=\"root\"><span id=\"server\">server</span></div>\
                      <p id=\"status\">pending</p>\
                      <script>\
                        const root = document.getElementById('root');\
                        const first = root.firstChild;\
                        const last = root.lastChild;\
                        document.getElementById('status').textContent =\
                          (first && first.getAttribute('id') === 'server') + ' ' +\
                          (last && last.getAttribute('id') === 'server') + ' ' +\
                          (root.childNodes.length === 1) + ' ' +\
                          (root.attributes.length === 1) + ' ' +\
                          (root.attributes[0].name === 'id');\
                      </script>";
        let snapshot = Snapshot::from_html(source).with_viewport(Viewport::new(500, 300));
        let capture = engine.render(&snapshot)?;
        assert!(
            !capture.audit.as_ref().is_some_and(|audit| audit.blocked),
            "{:?}",
            capture.compat
        );
        let status = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("status"))
            .ok_or("status missing")?;
        assert_eq!(status.text.as_deref(), Some("true true true true true"));
        let root = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("root"))
            .ok_or("root missing")?;
        assert!(root.text.as_deref().unwrap_or_default().contains("server"));
        Ok(())
    }

    #[test]
    fn centers_a_block_with_auto_horizontal_margins() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"width:400px\">\
             <div id=\"box\" style=\"width:100px;margin:0 auto;height:10px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(500, 300));
        let capture = engine.render(&snapshot)?;
        let box_element = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("box"))
            .ok_or("box missing")?;
        assert!(
            (box_element.rect.x - 158.0).abs() < 0.5,
            "{:?}",
            box_element.rect
        );
        Ok(())
    }

    #[test]
    fn executes_a_script_that_builds_dom_nodes() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<style>.card{width:120px;height:20px}</style>\
             <div id=\"root\"></div>\
             <script>\
               const node = document.createElement('div');\
               node.className = 'card';\
               node.textContent = 'Hello';\
               node.style.setProperty('background-color', 'red');\
               document.getElementById('root').appendChild(node);\
             </script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let card = capture
            .elements
            .iter()
            .find(|element| element.classes.iter().any(|class| class == "card"))
            .ok_or("card missing")?;
        assert_eq!(card.text.as_deref(), Some("Hello"));
        assert!((card.rect.width - 120.0).abs() < 0.5, "{:?}", card.rect);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn parses_inner_html_from_a_script() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"root\"></div>\
             <script>document.getElementById('root').innerHTML = '<div id=\"child\">Hi</div>';</script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let child = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("child"))
            .ok_or("child missing")?;
        assert_eq!(child.text.as_deref(), Some("Hi"));
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn executes_a_local_script_before_layout() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot =
            Snapshot::from_html("<div id=\"root\">Loading…</div><script src=\"app.js\"></script>")
                .with_resource(
                    "app.js",
                    "document.getElementById('root').textContent = 'Hydrated by JavaScript';",
                )
                .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let root = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("root"))
            .ok_or("root missing")?;
        assert_eq!(root.text.as_deref(), Some("Hydrated by JavaScript"));
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn resolves_a_linear_calc_length() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"width:300px\"><div id=\"box\" style=\"width:calc(100% - 20px);height:10px\"></div></div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let box_element = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("box"))
            .ok_or("box missing")?;
        assert!(
            (box_element.rect.width - 280.0).abs() < 0.5,
            "{}",
            box_element.rect.width
        );
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn places_inline_flex_on_the_text_line() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div><span id=\"badge\" style=\"display:inline-flex;gap:4px\">\
             <span>A</span><span>B</span></span><span id=\"after\" style=\"display:inline-block;width:20px;height:10px\"></span></div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let badge = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("badge"))
            .ok_or("badge missing")?;
        let after = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("after"))
            .ok_or("after missing")?;
        assert!(
            after.rect.x >= badge.rect.right() - 0.5,
            "{:?} {:?}",
            badge.rect,
            after.rect
        );
        assert!(
            (after.rect.bottom() - badge.rect.bottom()).abs() < 0.5,
            "{:?} {:?}",
            badge.rect,
            after.rect
        );
        assert!(badge.rect.width > 20.0, "{:?}", badge.rect);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn relative_position_offsets_without_moving_the_flow() -> std::result::Result<(), Box<dyn Error>>
    {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div style=\"height:100px\">\
             <div id=\"a\" style=\"position:relative;top:10px;left:20px;height:20px\"></div>\
             <div id=\"b\" style=\"height:20px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        assert!((a.rect.x - 28.0).abs() < 0.5, "{:?}", a.rect);
        assert!((a.rect.y - 18.0).abs() < 0.5, "{:?}", a.rect);
        assert!((b.rect.x - 8.0).abs() < 0.5, "{:?}", b.rect);
        assert!((b.rect.y - 28.0).abs() < 0.5, "{:?}", b.rect);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn flex_wrap_moves_overflowing_items_to_the_next_line()
    -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"row\" style=\"display:flex;flex-wrap:wrap;width:200px\">\
             <div id=\"a\" style=\"width:80px;height:10px\"></div>\
             <div id=\"b\" style=\"width:80px;height:10px\"></div>\
             <div id=\"c\" style=\"width:80px;height:10px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        let c = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("c"))
            .ok_or("c missing")?;
        assert!((a.rect.y - b.rect.y).abs() < 0.5);
        assert!(c.rect.y > a.rect.y + 1.0, "a {:?}, c {:?}", a.rect, c.rect);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn lays_out_an_auto_fit_grid() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"grid\" style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(100px,1fr));gap:10px;width:320px\">\
             <div id=\"a\" style=\"height:20px\"></div>\
             <div id=\"b\" style=\"height:20px\"></div>\
             <div id=\"c\" style=\"height:20px\"></div>\
             </div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let a = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("a"))
            .ok_or("a missing")?;
        let b = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("b"))
            .ok_or("b missing")?;
        let c = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("c"))
            .ok_or("c missing")?;
        assert!((a.rect.y - b.rect.y).abs() < 0.5);
        assert!((b.rect.y - c.rect.y).abs() < 0.5);
        assert!((a.rect.width - 100.0).abs() < 0.5);
        assert!(
            capture.compat.unsupported.is_empty(),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn border_box_min_width_includes_padding() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"box\" style=\"box-sizing:border-box;width:100px;min-width:150px;padding:10px\">Hi</div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let box_element = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("box"))
            .ok_or("box missing")?;
        assert!(
            (box_element.rect.width - 150.0).abs() < 0.5,
            "{}",
            box_element.rect.width
        );
        Ok(())
    }

    #[test]
    fn audit_blocks_pages_with_unexecuted_scripts() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            elements: true,
            ..Config::default()
        })?;
        let snapshot =
            Snapshot::from_html("<div id=\"app\">Loading…</div><script src=\"app.js\"></script>")
                .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let audit = capture.audit.unwrap_or_else(|| panic!("audit missing"));
        assert!(audit.blocked);
        assert_eq!(audit.score, 0);
        assert!(
            capture
                .compat
                .unsupported
                .iter()
                .any(|feature| feature.contains("script")),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn audit_blocks_approximate_layout() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            elements: true,
            ..Config::default()
        })?;
        let snapshot = Snapshot::from_html(
            "<div style=\"position:absolute\"><span>one</span><span>two</span></div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let audit = capture.audit.unwrap_or_else(|| panic!("audit missing"));
        assert!(audit.blocked);
        assert_eq!(audit.score, 0);
        Ok(())
    }

    #[test]
    fn audit_ignores_intentional_clipping() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            elements: true,
            ..Config::default()
        })?;
        let snapshot = Snapshot::from_html(
            "<div style=\"width:100px;overflow:hidden\"><div style=\"width:200px;height:10px\"></div></div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let audit = capture.audit.unwrap_or_else(|| panic!("audit missing"));
        assert!(!audit.issues.iter().any(|issue| {
            matches!(
                issue.kind,
                crate::IssueKind::Overflow | crate::IssueKind::Clipped
            )
        }));
        Ok(())
    }

    #[test]
    fn audit_uses_the_container_background() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            elements: true,
            ..Config::default()
        })?;
        let snapshot = Snapshot::from_html(
            "<div style=\"background:#111111;color:#ffffff;padding:8px\">Readable</div>",
        )
        .with_viewport(Viewport::new(400, 300));
        let capture = engine.render(&snapshot)?;
        let audit = capture.audit.unwrap_or_else(|| panic!("audit missing"));
        assert!(
            !audit
                .issues
                .iter()
                .any(|issue| issue.kind == crate::IssueKind::Contrast)
        );
        Ok(())
    }

    #[test]
    fn draws_an_svg_resource() -> std::result::Result<(), Box<dyn Error>> {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#00ff00"/></svg>"##;
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html("<img src=\"dot.svg\" width=\"16\" height=\"16\">")
            .with_resource("dot.svg", svg.to_vec())
            .with_viewport(Viewport::new(80, 60));
        let capture = engine.render(&snapshot)?;
        let image = image::load_from_memory(&capture.png)?.to_rgba8();
        let pixel = image.get_pixel(16, 16);
        assert!(pixel[1] > 200);
        assert!(pixel[0] < 50);
        assert!(pixel[2] < 50);
        Ok(())
    }

    #[test]
    fn draws_an_image_resource() -> std::result::Result<(), Box<dyn Error>> {
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([220, 30, 30, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image).write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )?;
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html("<img src=\"dot.png\" width=\"8\" height=\"8\">")
            .with_resource("dot.png", bytes)
            .with_viewport(Viewport::new(200, 200));
        let capture = engine.render(&snapshot)?;
        assert!(capture.elements.iter().any(|element| element.tag == "img"));
        Ok(())
    }

    #[test]
    fn runs_microtasks_before_zero_timeout() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            r#"<div id="app">initial</div><script>
            const log = [];
            queueMicrotask(() => log.push("micro"));
            setTimeout(() => {
                log.push("timer");
                document.getElementById("app").textContent = log.join(",");
            }, 0);
            </script>"#,
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let app = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("app"))
            .ok_or("app missing")?;
        assert_eq!(app.text.as_deref(), Some("micro,timer"));
        Ok(())
    }

    #[test]
    fn events_drain_zero_timeout_tasks() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            r#"<button id="go">go</button><div id="app">before</div><script>
            document.getElementById("go").addEventListener("click", () => {
                setTimeout(() => {
                    document.getElementById("app").textContent = "after";
                }, 0);
            });
            </script>"#,
        )
        .with_viewport(Viewport::new(320, 200));
        let mut session = engine.open(snapshot)?;
        session.click(&crate::Target::Id("go".to_owned()))?;
        let capture = session.capture()?;
        let app = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("app"))
            .ok_or("app missing")?;
        assert_eq!(app.text.as_deref(), Some("after"));
        Ok(())
    }

    #[test]
    fn disabled_javascript_blocks_audit() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            javascript: false,
            ..Config::default()
        })?;
        let snapshot = Snapshot::from_html(
            "<div id=\"app\">loading</div><script>document.getElementById('app').textContent='ready';</script>",
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let app = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("app"))
            .ok_or("app missing")?;
        assert_eq!(app.text.as_deref(), Some("loading"));
        let audit = capture.audit.ok_or("audit missing")?;
        assert!(audit.blocked);
        assert_eq!(audit.score, 0);
        assert!(
            capture
                .compat
                .unsupported
                .iter()
                .any(|feature| feature.contains("disabled")),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn snapshot_warnings_block_audit() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            audit: true,
            ..Config::default()
        })?;
        let mut snapshot =
            Snapshot::from_html("<p>content</p>").with_viewport(Viewport::new(320, 200));
        snapshot
            .warnings
            .push("stylesheet unreadable: missing.css: not found".to_owned());
        let capture = engine.render(&snapshot)?;
        let audit = capture.audit.ok_or("audit missing")?;
        assert!(audit.blocked);
        assert_eq!(audit.score, 0);
        assert!(
            capture
                .compat
                .unsupported
                .iter()
                .any(|feature| feature.contains("resource:")),
            "{:?}",
            capture.compat.unsupported
        );
        Ok(())
    }

    #[test]
    fn clear_timeout_prevents_the_callback() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            r#"<div id="app">before</div><script>
            const cancelled = setTimeout(() => {
                document.getElementById("app").textContent = "cancelled";
            }, 0);
            clearTimeout(cancelled);
            setTimeout(() => {
                document.getElementById("app").textContent = "kept";
            }, 0);
            </script>"#,
        )
        .with_viewport(Viewport::new(320, 200));
        let capture = engine.render(&snapshot)?;
        let app = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("app"))
            .ok_or("app missing")?;
        assert_eq!(app.text.as_deref(), Some("kept"));
        Ok(())
    }

    #[test]
    fn session_advance_runs_delayed_timers_and_intervals() -> std::result::Result<(), Box<dyn Error>>
    {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            r#"<div id="app">before</div><script>
            let count = 0;
            const interval = setInterval(() => {
                count += 1;
                document.getElementById("app").textContent = String(count);
                if (count === 2) {
                    clearInterval(interval);
                }
            }, 10);
            </script>"#,
        )
        .with_viewport(Viewport::new(320, 200));
        let mut session = engine.open(snapshot)?;
        session.advance_time(Duration::from_millis(25))?;
        let capture = session.capture()?;
        let app = capture
            .elements
            .iter()
            .find(|element| element.id.as_deref() == Some("app"))
            .ok_or("app missing")?;
        assert_eq!(app.text.as_deref(), Some("2"));
        Ok(())
    }

    #[test]
    fn rejects_html_above_configured_limit() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config {
            max_html_bytes: 8,
            ..Config::default()
        })?;
        let error = engine
            .render(&Snapshot::from_html("<p>too large</p>"))
            .err()
            .ok_or("limit not enforced")?;
        assert!(matches!(error, crate::error::Error::Limit(_)));
        Ok(())
    }
}
