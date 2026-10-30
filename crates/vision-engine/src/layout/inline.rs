//! Inline formatting and line breaking.

use std::sync::Arc;

use crate::compat::CompatReport;
use crate::document::style::{ComputedStyle, TextAlign, WhiteSpace};
use crate::text::FontBook;

use super::tree::{AtomicFragment, BoxId, LineBox, LineItem, TextFragment};

/// An item in an inline formatting context.
#[derive(Clone, Debug)]
pub(crate) enum InlineItem {
    /// Text with its computed style.
    Text(String, Box<ComputedStyle>),
    /// A forced line break.
    Break,
    /// An atomic inline box.
    Atomic(AtomicItem),
}

/// The geometry needed to place an atomic inline box on a line.
#[derive(Clone, Debug)]
pub(crate) struct AtomicItem {
    /// The laid-out box.
    pub(crate) box_id: BoxId,
    /// Width including horizontal margins.
    pub(crate) outer_width: f32,
    /// Distance from the line baseline to the top of the margin box.
    pub(crate) ascent: f32,
    /// Distance from the line baseline to the bottom of the margin box.
    pub(crate) descent: f32,
    /// Left margin.
    pub(crate) margin_left: f32,
    /// Top margin.
    pub(crate) margin_top: f32,
    /// Font size used for a preceding word space.
    pub(crate) font_size: f32,
}

impl InlineItem {
    /// Creates a text item.
    pub(crate) fn text(text: impl Into<String>, style: ComputedStyle) -> Self {
        Self::Text(text.into(), Box::new(style))
    }
}

#[derive(Clone, Debug)]
enum TokenKind {
    Text(String),
    Atomic(AtomicItem),
    Break,
}

#[derive(Clone)]
struct Token {
    kind: TokenKind,
    style: ComputedStyle,
    space_before: bool,
    break_before: bool,
    break_after: bool,
}

/// Lays out an inline formatting context.
pub(crate) fn layout(
    items: &[InlineItem],
    x: f32,
    width: f32,
    y: f32,
    fonts: &FontBook,
    compat: &mut CompatReport,
) -> Vec<LineBox> {
    let tokens = tokenize(items);
    let mut builder = LineBuilder::new(x, width, y, fonts, compat);
    for token in tokens {
        if token.break_before {
            builder.flush();
        }
        builder.push(&token);
        if token.break_after {
            builder.flush();
        }
    }
    builder.flush();
    builder.lines
}

fn tokenize(items: &[InlineItem]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut pending_space = false;
    for item in items {
        match item {
            InlineItem::Break => {
                if let Some(last) = tokens.last_mut() {
                    last.break_after = true;
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Break,
                        style: ComputedStyle::default(),
                        space_before: false,
                        break_before: true,
                        break_after: true,
                    });
                }
                pending_space = false;
            }
            InlineItem::Atomic(atomic) => {
                tokens.push(Token {
                    kind: TokenKind::Atomic(atomic.clone()),
                    style: ComputedStyle::default(),
                    space_before: pending_space,
                    break_before: false,
                    break_after: false,
                });
                pending_space = false;
            }
            InlineItem::Text(text, style) => {
                let style = style.as_ref();
                if style.white_space == WhiteSpace::Pre {
                    let mut lines = text.split('\n').peekable();
                    while let Some(line) = lines.next() {
                        if !line.is_empty() {
                            tokens.push(Token {
                                kind: TokenKind::Text(line.to_owned()),
                                style: style.clone(),
                                space_before: false,
                                break_before: false,
                                break_after: false,
                            });
                        }
                        if lines.peek().is_some() {
                            if let Some(last) = tokens.last_mut() {
                                last.break_after = true;
                            }
                        }
                    }
                    pending_space = false;
                    continue;
                }

                let mut word = String::new();
                let mut word_space = pending_space;
                for ch in text.chars() {
                    if ch.is_whitespace() {
                        if !word.is_empty() {
                            tokens.push(Token {
                                kind: TokenKind::Text(std::mem::take(&mut word)),
                                style: style.clone(),
                                space_before: word_space,
                                break_before: false,
                                break_after: false,
                            });
                        }
                        word_space = true;
                    } else {
                        if word.is_empty() {
                            word_space = pending_space || word_space;
                            pending_space = false;
                        }
                        word.push(ch);
                    }
                }
                if !word.is_empty() {
                    tokens.push(Token {
                        kind: TokenKind::Text(word),
                        style: style.clone(),
                        space_before: word_space,
                        break_before: false,
                        break_after: false,
                    });
                }
                pending_space = text.chars().last().is_some_and(char::is_whitespace);
            }
        }
    }
    tokens
}

struct LineBuilder<'a> {
    lines: Vec<LineBox>,
    x: f32,
    width: f32,
    y: f32,
    current: Vec<LineItem>,
    line_width: f32,
    max_ascent: f32,
    max_descent: f32,
    max_height: f32,
    fonts: &'a FontBook,
    compat: &'a mut CompatReport,
}

impl<'a> LineBuilder<'a> {
    fn new(x: f32, width: f32, y: f32, fonts: &'a FontBook, compat: &'a mut CompatReport) -> Self {
        Self {
            lines: Vec::new(),
            x,
            width,
            y,
            current: Vec::new(),
            line_width: 0.0,
            max_ascent: 0.0,
            max_descent: 0.0,
            max_height: 0.0,
            fonts,
            compat,
        }
    }

    fn push(&mut self, token: &Token) {
        if matches!(token.kind, TokenKind::Break) {
            self.flush();
            return;
        }
        if let TokenKind::Atomic(atomic) = &token.kind {
            let space = if token.space_before && !self.current.is_empty() {
                atomic.font_size * 0.25
            } else {
                0.0
            };
            if !self.current.is_empty() && self.line_width + space + atomic.outer_width > self.width
            {
                self.flush();
            }
            let space = if token.space_before && !self.current.is_empty() {
                atomic.font_size * 0.25
            } else {
                0.0
            };
            if !self.current.is_empty() {
                self.line_width += space;
            }
            let x = self.x + self.line_width + atomic.margin_left;
            self.current.push(LineItem::Atomic(AtomicFragment {
                box_id: atomic.box_id,
                x,
                y: 0.0,
                ascent: atomic.ascent,
                margin_top: atomic.margin_top,
            }));
            self.line_width += atomic.outer_width;
            self.max_ascent = self.max_ascent.max(atomic.ascent);
            self.max_descent = self.max_descent.max(atomic.descent);
            self.max_height = self.max_height.max(atomic.ascent + atomic.descent);
            return;
        }

        let TokenKind::Text(text) = &token.kind else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let shaped = self.fonts.shape(text, &token.style);
        let (word_width, ascent, descent, line_gap) = match &shaped {
            Some(shaped) => (shaped.width, shaped.ascent, shaped.descent, shaped.line_gap),
            None => {
                self.compat
                    .unsupported(format!("font for: {}", token.style.font_family.join(", ")));
                (
                    text.chars().count() as f32 * token.style.font_size * 0.5,
                    token.style.font_size * 0.8,
                    token.style.font_size * 0.2,
                    0.0,
                )
            }
        };
        let normal_height = ascent + descent + line_gap;
        let line_height = token.style.line_height_px(normal_height);
        let space = if token.space_before && !self.current.is_empty() {
            token.style.font_size * 0.25
        } else {
            0.0
        };

        let can_wrap = !matches!(token.style.white_space, WhiteSpace::Nowrap);
        if can_wrap && !self.current.is_empty() && self.line_width + space + word_width > self.width
        {
            self.flush();
        }

        if !self.current.is_empty() {
            self.line_width += space;
        }

        let fragment_x = self.x + self.line_width;
        self.current.push(LineItem::Text(TextFragment {
            style: token.style.clone(),
            shaped: shaped.map(Arc::new),
            x: fragment_x,
            width: word_width,
        }));
        self.line_width += word_width;
        self.max_ascent = self.max_ascent.max(ascent);
        self.max_descent = self.max_descent.max(descent);
        self.max_height = self.max_height.max(line_height);
    }

    fn flush(&mut self) {
        if self.current.is_empty() {
            self.y += self.max_height.max(0.0);
            self.max_height = 0.0;
            self.max_ascent = 0.0;
            self.max_descent = 0.0;
            return;
        }

        let height = self.max_height.max(self.max_ascent + self.max_descent);
        let leading = (height - (self.max_ascent + self.max_descent)).max(0.0);
        let baseline = self.y + self.max_ascent + leading / 2.0;
        let align = self
            .current
            .iter()
            .find_map(|item| match item {
                LineItem::Text(fragment) => Some(fragment.style.text_align),
                LineItem::Atomic(_) => None,
            })
            .unwrap_or(TextAlign::Left);
        let free = (self.width - self.line_width).max(0.0);
        let offset = match align {
            TextAlign::Left | TextAlign::Justify => 0.0,
            TextAlign::Center => free / 2.0,
            TextAlign::Right => free,
        };
        if offset != 0.0 {
            for item in &mut self.current {
                match item {
                    LineItem::Text(fragment) => fragment.x += offset,
                    LineItem::Atomic(fragment) => fragment.x += offset,
                }
            }
        }
        for item in &mut self.current {
            if let LineItem::Atomic(fragment) = item {
                fragment.y = baseline - fragment.ascent + fragment.margin_top;
            }
        }
        self.lines.push(LineBox {
            items: std::mem::take(&mut self.current),
            height,
            baseline,
        });
        self.y += height;
        self.line_width = 0.0;
        self.max_ascent = 0.0;
        self.max_descent = 0.0;
        self.max_height = 0.0;
    }
}
