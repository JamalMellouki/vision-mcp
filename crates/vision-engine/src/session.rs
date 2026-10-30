//! Interactive sessions over a prepared page.
//!
//! A session keeps the parsed document, computed styles, layout tree, and scroll
//! offset. Actions update the document or session state and repaint the same
//! page. When scripts registered listeners, the retained QuickJS context
//! dispatches the corresponding DOM event before the default action.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::api::{Capture, Element, Engine, Prepared};
use crate::document::NodeId;
use crate::document::selector::parse_selector_list;
use crate::document::style::StyleTree;
use crate::error::Result;
use crate::geom::{Point, Rect};
use crate::layout::build_layout;
use crate::paint::paint;
use crate::render::author_rules;
use crate::snapshot::Snapshot;

/// A way to address an element in a session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Target {
    /// The element under a page coordinate.
    Point(Point),
    /// The first element matched by a CSS selector.
    Selector(String),
    /// The element with the given id.
    Id(String),
    /// The first element whose rendered text contains the string.
    Text(String),
}

/// The result of a session action.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// A click was delivered to an element.
    Click {
        /// The element that received the click.
        target: Element,
    },
    /// An element received focus.
    Focus {
        /// The focused element.
        target: Element,
    },
    /// Text was written into an editable control.
    Type {
        /// The control that received the text.
        target: Element,
        /// The new value.
        value: String,
    },
    /// The scroll offset changed.
    Scroll {
        /// Horizontal offset in CSS pixels.
        x: f32,
        /// Vertical offset in CSS pixels.
        y: f32,
    },
    /// The target did not resolve to a rendered element.
    Missing,
}

/// A page with retained layout and interaction state.
pub struct Session {
    engine: Engine,
    snapshot: Snapshot,
    prepared: Prepared,
    scroll: Point,
}

impl Session {
    pub(crate) fn new(engine: Engine, snapshot: Snapshot, prepared: Prepared) -> Self {
        let scroll = snapshot.state.scroll;
        Self {
            engine,
            snapshot,
            prepared,
            scroll,
        }
    }

    /// Resolves a target and returns its current border box.
    #[must_use]
    pub fn find(&self, target: &Target) -> Option<Element> {
        match target {
            Target::Point(point) => self.find_point(*point),
            Target::Selector(raw) => {
                let selectors = parse_selector_list(raw);
                if selectors.is_empty() {
                    return None;
                }
                let document = self.prepared.document.borrow();
                document
                    .descendants()
                    .find(|node| {
                        selectors
                            .iter()
                            .any(|selector| selector.matches(&document, *node))
                    })
                    .and_then(|node| self.element_for_node(node))
            }
            Target::Id(id) => {
                let document = self.prepared.document.borrow();
                document
                    .descendants()
                    .find(|node| document.attr(*node, "id") == Some(id.as_str()))
                    .and_then(|node| self.element_for_node(node))
            }
            Target::Text(text) => {
                let document = self.prepared.document.borrow();
                document
                    .descendants()
                    .filter(|node| {
                        document
                            .element(*node)
                            .is_some_and(|_| document.visible_text(*node).contains(text))
                    })
                    .filter_map(|node| self.element_for_node(node))
                    .min_by(|left, right| area(left).total_cmp(&area(right)))
            }
        }
    }

    /// Clicks the target, dispatches `click`, and applies the default action.
    pub fn click(&mut self, target: &Target) -> Result<Action> {
        let Some(element) = self.find(target) else {
            return Ok(Action::Missing);
        };
        let node = NodeId(element.node);
        let prevented = self.dispatch_event(node, "click")?;
        let mut refresh = self.prepared.script.is_some();
        if element.tag == "input" && !prevented {
            let kind = self
                .prepared
                .document
                .borrow()
                .attr(node, "type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if matches!(kind.as_str(), "checkbox" | "radio") {
                if self
                    .prepared
                    .document
                    .borrow()
                    .attr(node, "checked")
                    .is_some()
                {
                    self.prepared
                        .document
                        .borrow_mut()
                        .remove_attr(node, "checked");
                } else {
                    self.prepared
                        .document
                        .borrow_mut()
                        .set_attr(node, "checked", "");
                }
                refresh = true;
            }
        }
        self.snapshot.state.focus = Some(element_key(&element));
        if refresh {
            self.refresh()?;
        }
        Ok(Action::Click { target: element })
    }

    /// Focuses the target, dispatches `focus`, and repaints if needed.
    pub fn focus(&mut self, target: &Target) -> Result<Action> {
        let Some(element) = self.find(target) else {
            return Ok(Action::Missing);
        };
        let node = NodeId(element.node);
        let refresh = self.prepared.script.is_some();
        self.dispatch_event(node, "focus")?;
        self.snapshot.state.focus = Some(element_key(&element));
        if refresh {
            self.refresh()?;
        }
        Ok(Action::Focus { target: element })
    }

    /// Writes text into an input or textarea and re-runs style and layout.
    pub fn type_text(&mut self, target: &Target, value: &str) -> Result<Action> {
        let Some(element) = self.find(target) else {
            return Ok(Action::Missing);
        };
        if !matches!(element.tag.as_str(), "input" | "textarea") {
            return Ok(Action::Missing);
        }
        let node = NodeId(element.node);
        if element.tag == "input" {
            let kind = self
                .prepared
                .document
                .borrow()
                .attr(node, "type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if matches!(
                kind.as_str(),
                "checkbox" | "radio" | "hidden" | "submit" | "button" | "reset" | "file"
            ) {
                return Ok(Action::Missing);
            }
        }
        self.prepared
            .document
            .borrow_mut()
            .set_attr(node, "value", value);
        self.snapshot
            .state
            .values
            .insert(element_key(&element), value.to_owned());
        self.dispatch_event(node, "input")?;
        self.refresh()?;
        Ok(Action::Type {
            target: element,
            value: value.to_owned(),
        })
    }

    /// Scrolls the page by a delta and clamps it to the document.
    ///
    /// Horizontal scrolling is not implemented; the offset stays at zero.
    pub fn scroll(&mut self, dx: f32, dy: f32) -> Action {
        let max_y = (self.prepared.layout.height() - self.snapshot.viewport.css_height()).max(0.0);
        let x = (self.scroll.x + dx).clamp(0.0, 0.0);
        let y = (self.scroll.y + dy).clamp(0.0, max_y);
        self.scroll = Point { x, y };
        Action::Scroll { x, y }
    }

    /// Returns the current scroll offset.
    #[must_use]
    pub fn scroll_position(&self) -> Point {
        self.scroll
    }

    /// Advances the timer clock and runs due `setTimeout`, `setInterval`, and
    /// animation-frame callbacks.
    ///
    /// Time is virtual for deterministic tests and headless rendering: the
    /// runtime starts at zero, then the supplied duration is added before due
    /// callbacks are drained. The document is restyled and laid out afterwards.
    pub fn advance_time(&mut self, duration: Duration) -> Result<()> {
        let milliseconds = duration.as_millis().min(u128::from(u64::MAX)) as u64;
        if let Some(script) = self.prepared.script.as_mut() {
            script.advance_time(milliseconds, &mut self.prepared.compat);
        }
        self.refresh()
    }

    /// Paints the current state.
    pub fn capture(&mut self) -> Result<Capture> {
        let paint_start = Instant::now();
        let pixmap = paint(
            &self.prepared.layout,
            self.snapshot.viewport,
            self.scroll,
            &self.prepared.document.borrow(),
            &mut self.prepared.compat,
        )?;
        let paint_ms = paint_start.elapsed().as_secs_f64() * 1000.0;
        self.prepared.capture(pixmap, &self.engine.config, paint_ms)
    }

    fn dispatch_event(&mut self, node: NodeId, event_type: &str) -> Result<bool> {
        let Some(script) = self.prepared.script.as_mut() else {
            return Ok(false);
        };
        Ok(script.dispatch(node, event_type, &mut self.prepared.compat))
    }

    fn refresh(&mut self) -> Result<()> {
        let mut compat = self.prepared.compat.clone();
        let style_start = Instant::now();
        let author_rules = author_rules(
            &self.prepared.document.borrow(),
            &self.snapshot,
            &mut compat,
        );
        let styles = StyleTree::build(
            &self.prepared.document.borrow(),
            &self.engine.ua_rules,
            &author_rules,
            self.snapshot.viewport,
            &mut compat,
        );
        let style_ms = style_start.elapsed().as_secs_f64() * 1000.0;
        let layout_start = Instant::now();
        let layout = build_layout(
            &self.prepared.document.borrow(),
            &styles,
            &self.engine.fonts,
            self.snapshot.viewport,
            &self.prepared.images,
            &mut compat,
        );
        let layout_ms = layout_start.elapsed().as_secs_f64() * 1000.0;
        self.prepared.author_rules = author_rules;
        self.prepared.styles = styles;
        self.prepared.layout = layout;
        self.prepared.compat = compat;
        self.prepared.timings.style_ms = style_ms;
        self.prepared.timings.layout_ms = layout_ms;
        self.prepared.timings.total_ms = self.prepared.timings.parse_ms + style_ms + layout_ms;
        Ok(())
    }

    fn find_point(&self, point: Point) -> Option<Element> {
        self.prepared
            .layout
            .boxes()
            .iter()
            .filter(|layout_box| layout_box.rect.contains(point))
            .min_by(|left, right| {
                let left_area = left.rect.width * left.rect.height;
                let right_area = right.rect.width * right.rect.height;
                left_area.total_cmp(&right_area)
            })
            .and_then(|layout_box| self.element_for_node(layout_box.node))
    }

    fn element_for_node(&self, node: NodeId) -> Option<Element> {
        let document = self.prepared.document.borrow();
        let element = document.element(node)?;
        let rect = self.box_for_node(node)?;
        let mut text = if element.name == "input" {
            document
                .attr(node, "value")
                .or_else(|| document.attr(node, "placeholder"))
                .unwrap_or_default()
                .to_owned()
        } else if element.name == "textarea" {
            document
                .attr(node, "value")
                .map_or_else(|| document.text_content(node), str::to_owned)
        } else {
            document.visible_text(node)
        };
        text = text.trim().to_owned();
        if text.len() > 512 {
            text.truncate(512);
        }
        Some(Element {
            node: node.0,
            tag: element.name.clone(),
            id: document.attr(node, "id").map(str::to_owned),
            classes: document
                .classes(node)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            rect,
            text: (!text.is_empty()).then_some(text),
        })
    }

    fn box_for_node(&self, node: NodeId) -> Option<Rect> {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if let Some(layout_box) = self
                .prepared
                .layout
                .boxes()
                .iter()
                .find(|layout_box| layout_box.node == candidate)
            {
                return Some(layout_box.rect);
            }
            current = self.prepared.document.borrow().parent(candidate);
        }
        None
    }
}

fn area(element: &Element) -> f32 {
    element.rect.width * element.rect.height
}

fn element_key(element: &Element) -> String {
    if let Some(id) = &element.id {
        format!("#{id}")
    } else {
        element.tag.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Config, Engine};
    use crate::snapshot::Viewport;

    use std::error::Error;

    #[test]
    fn click_type_and_scroll() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot =
            Snapshot::from_html("<input id=\"name\"><div style=\"height:1200px\"></div>")
                .with_viewport(Viewport::new(320, 240));
        let mut session = engine.open(snapshot)?;

        let id = Target::Id("name".to_owned());
        let action = session.click(&id)?;
        assert!(matches!(action, Action::Click { .. }));
        let action = session.focus(&id)?;
        assert!(matches!(action, Action::Focus { .. }));
        let action = session.type_text(&id, "hello")?;
        assert!(matches!(action, Action::Type { value, .. } if value == "hello"));
        let found = session
            .find(&id)
            .unwrap_or_else(|| panic!("input disappeared"));
        assert_eq!(found.text.as_deref(), Some("hello"));

        match session.scroll(0.0, 100.0) {
            Action::Scroll { y, .. } => assert!(y > 0.0),
            _ => panic!("expected scroll action"),
        }
        let capture = session.capture()?;
        assert!(capture.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(capture.height, 240);
        Ok(())
    }

    #[test]
    fn session_starts_at_the_snapshot_scroll() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let mut snapshot = Snapshot::from_html("<div style=\"height:1200px\"></div>")
            .with_viewport(Viewport::new(320, 240));
        snapshot.state.scroll = Point { x: 0.0, y: 50.0 };
        let session = engine.open(snapshot)?;
        assert_eq!(session.scroll_position().y, 50.0);
        Ok(())
    }

    #[test]
    fn click_dispatches_retained_event_listener() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<button id=\"button\">zero</button><p id=\"status\">waiting</p>\
             <script>\
               let count = 0;\
               const button = document.getElementById('button');\
               button.addEventListener('click', function () {\
                 count += 1;\
                 document.getElementById('status').textContent = 'clicked:' + count;\
               });\
             </script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let mut session = engine.open(snapshot)?;
        session.click(&Target::Id("button".to_owned()))?;
        session.click(&Target::Id("button".to_owned()))?;
        let status = session
            .find(&Target::Id("status".to_owned()))
            .unwrap_or_else(|| panic!("status disappeared"));
        assert_eq!(status.text.as_deref(), Some("clicked:2"));
        Ok(())
    }

    #[test]
    fn onclick_property_handler_receives_the_event() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<button id=\"button\">go</button><p id=\"status\"></p>\
             <script>\
               document.getElementById('button').onclick = function (event) {\
                 document.getElementById('status').textContent = 'property:' + event.target.id;\
               };\
             </script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let mut session = engine.open(snapshot)?;
        session.click(&Target::Id("button".to_owned()))?;
        let status = session
            .find(&Target::Id("status".to_owned()))
            .unwrap_or_else(|| panic!("status disappeared"));
        assert_eq!(status.text.as_deref(), Some("property:button"));
        Ok(())
    }

    #[test]
    fn prevented_click_skips_the_checkbox_default_action() -> std::result::Result<(), Box<dyn Error>>
    {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<input id=\"flag\" type=\"checkbox\">\
             <script>\
               document.getElementById('flag').addEventListener('click', function (event) {\
                 event.preventDefault();\
               });\
             </script>",
        )
        .with_viewport(Viewport::new(320, 240));
        let mut session = engine.open(snapshot)?;
        session.click(&Target::Id("flag".to_owned()))?;
        let node = session
            .prepared
            .document
            .borrow()
            .descendants()
            .find(|node| session.prepared.document.borrow().attr(*node, "id") == Some("flag"))
            .unwrap_or_else(|| panic!("checkbox disappeared"));
        assert!(
            session
                .prepared
                .document
                .borrow()
                .attr(node, "checked")
                .is_none(),
            "preventDefault must suppress the checkbox toggle"
        );
        Ok(())
    }

    #[test]
    fn document_listener_receives_a_bubbled_click() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<button id=\"button\">go</button><p id=\"status\"></p>\
             <script>\
               document.addEventListener('click', function (event) {\
                 document.getElementById('status').textContent = 'doc:' + event.target.id;\
               });\
             </script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let mut session = engine.open(snapshot)?;
        session.click(&Target::Id("button".to_owned()))?;
        let status = session
            .find(&Target::Id("status".to_owned()))
            .unwrap_or_else(|| panic!("status disappeared"));
        assert_eq!(status.text.as_deref(), Some("doc:button"));
        Ok(())
    }

    #[test]
    fn input_event_updates_a_sibling() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<input id=\"name\"><p id=\"preview\"></p>\
             <script>\
               const input = document.getElementById('name');\
               input.addEventListener('input', function () {\
                 document.getElementById('preview').textContent = this.value;\
               });\
             </script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let mut session = engine.open(snapshot)?;
        session.type_text(&Target::Id("name".to_owned()), "hello")?;
        let preview = session
            .find(&Target::Id("preview".to_owned()))
            .unwrap_or_else(|| panic!("preview disappeared"));
        assert_eq!(preview.text.as_deref(), Some("hello"));
        Ok(())
    }

    #[test]
    fn click_event_bubbles_to_an_ancestor() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<div id=\"outer\"><button id=\"inner\">go</button></div><p id=\"status\"></p>\
             <script>\
               document.getElementById('outer').addEventListener('click', function (event) {\
                 document.getElementById('status').textContent = event.target.id + ':' + this.id;\
               });\
             </script>",
        )
        .with_viewport(Viewport::new(400, 300));
        let mut session = engine.open(snapshot)?;
        session.click(&Target::Id("inner".to_owned()))?;
        let status = session
            .find(&Target::Id("status".to_owned()))
            .unwrap_or_else(|| panic!("status disappeared"));
        assert_eq!(status.text.as_deref(), Some("inner:outer"));
        Ok(())
    }

    #[test]
    fn click_toggles_checkbox() -> std::result::Result<(), Box<dyn Error>> {
        let engine = Engine::native(Config::default())?;
        let snapshot = Snapshot::from_html(
            "<input id=\"flag\" type=\"checkbox\"><div style=\"height:200px\"></div>",
        )
        .with_viewport(Viewport::new(320, 240));
        let mut session = engine.open(snapshot)?;
        let id = Target::Id("flag".to_owned());
        session.click(&id)?;
        let node = session
            .prepared
            .document
            .borrow()
            .descendants()
            .find(|node| session.prepared.document.borrow().attr(*node, "id") == Some("flag"))
            .unwrap_or_else(|| panic!("checkbox disappeared"));
        assert!(
            session
                .prepared
                .document
                .borrow()
                .attr(node, "checked")
                .is_some()
        );
        session.click(&id)?;
        assert!(
            session
                .prepared
                .document
                .borrow()
                .attr(node, "checked")
                .is_none()
        );
        Ok(())
    }
}
