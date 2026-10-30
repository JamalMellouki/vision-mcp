//! Computed style values and the cascade.

use std::collections::{HashMap, HashSet};

use crate::color::Color;
use crate::compat::CompatReport;
use crate::document::css::{Declaration, Origin, Rule, parse_declarations};
use crate::geom::Edges;
use crate::snapshot::Viewport;

use super::dom::{Document, NodeId};

/// A CSS display value that affects layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Display {
    /// Inline content.
    #[default]
    Inline,
    /// A block container.
    Block,
    /// An inline-level block container.
    InlineBlock,
    /// A list item.
    ListItem,
    /// A flex container.
    Flex,
    /// An inline-level flex container.
    InlineFlex,
    /// A grid container.
    Grid,
    /// A table wrapper.
    Table,
    /// A table row.
    TableRow,
    /// A table cell.
    TableCell,
    /// The element generates no box.
    None,
}

impl Display {
    /// Returns true for boxes that participate in block layout.
    pub(crate) fn is_block_level(self) -> bool {
        matches!(
            self,
            Self::Block
                | Self::InlineBlock
                | Self::ListItem
                | Self::Flex
                | Self::Grid
                | Self::Table
                | Self::TableRow
                | Self::TableCell
        )
    }
}

/// The main axis of a flex container.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FlexDirection {
    /// Left to right in horizontal writing modes.
    #[default]
    Row,
    /// Right to left.
    RowReverse,
    /// Top to bottom.
    Column,
    /// Bottom to top.
    ColumnReverse,
}

impl FlexDirection {
    /// Returns true when the main axis is horizontal.
    pub(crate) fn is_row(self) -> bool {
        matches!(self, Self::Row | Self::RowReverse)
    }
}

/// Whether flex items stay on one line or wrap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FlexWrap {
    /// Keep all items on one line.
    #[default]
    NoWrap,
    /// Wrap items onto additional lines.
    Wrap,
    /// Wrap items in reverse cross-axis order.
    WrapReverse,
}

/// Distribution of free space along a flex container's main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum JustifyContent {
    /// Pack items at the start.
    #[default]
    FlexStart,
    /// Pack items at the end.
    FlexEnd,
    /// Center items.
    Center,
    /// Put equal space between items.
    SpaceBetween,
    /// Put equal space around items.
    SpaceAround,
    /// Put equal space between and around items.
    SpaceEvenly,
}

/// A text case transformation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextTransform {
    /// Leave text unchanged.
    #[default]
    None,
    /// Render upper case.
    Uppercase,
    /// Render lower case.
    Lowercase,
    /// Capitalize each word.
    Capitalize,
}

/// A single track in a grid template.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GridTrack {
    /// A fixed track size.
    Px(f32),
    /// A share of the remaining space.
    Fr(f32),
    /// An automatically sized repeat track.
    AutoFit {
        /// Minimum track size.
        min: f32,
        /// Fractional share of the remaining space.
        fraction: f32,
    },
}

/// Alignment of flex items on the cross axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AlignItems {
    /// Stretch items with an automatic cross size.
    #[default]
    Stretch,
    /// Align at the cross-start edge.
    FlexStart,
    /// Align at the cross-end edge.
    FlexEnd,
    /// Center on the cross axis.
    Center,
    /// Align baselines.
    Baseline,
}

/// A CSS position value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Position {
    /// Normal flow.
    #[default]
    Static,
    /// Offset from the normal position.
    Relative,
    /// Out of flow, relative to an ancestor.
    Absolute,
    /// Out of flow, relative to the viewport.
    Fixed,
    /// Sticky positioning.
    Sticky,
}

/// A CSS box sizing value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BoxSizing {
    /// Width and height refer to the content box.
    #[default]
    ContentBox,
    /// Width and height include padding and border.
    BorderBox,
}

/// A CSS length before layout resolves percentages.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Length {
    /// The property is auto.
    #[default]
    Auto,
    /// A fixed length in CSS pixels.
    Px(f32),
    /// A percentage of the containing block.
    Percent(f32),
    /// A linear combination of pixels and a percentage.
    Calc {
        /// Fixed pixel component.
        px: f32,
        /// Percentage component.
        percent: f32,
    },
}

impl Length {
    /// Resolves the length against a containing block size.
    #[must_use]
    pub(crate) fn resolve(self, base: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Px(value) => Some(value),
            Self::Percent(value) => Some(base * value / 100.0),
            Self::Calc { px, percent } => Some(px + base * percent / 100.0),
        }
    }

    /// Parses a CSS length.
    pub(crate) fn parse(
        raw: &str,
        font_size: f32,
        root_font_size: f32,
        viewport: Viewport,
    ) -> Option<Self> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("auto") {
            return Some(Self::Auto);
        }
        let lower = raw.to_ascii_lowercase();
        if lower.starts_with("calc(") && raw.ends_with(')') {
            let inner = &raw["calc(".len()..raw.len() - 1];
            let (px, percent) = parse_calc_expression(inner, font_size, root_font_size, viewport)?;
            return Some(Self::Calc { px, percent });
        }
        if raw == "0" {
            return Some(Self::Px(0.0));
        }
        let number = |suffix: &str| {
            raw.strip_suffix(suffix)
                .and_then(|value| value.parse::<f32>().ok())
        };
        if let Some(value) = number("px") {
            return Some(Self::Px(value));
        }
        if let Some(value) = number("em") {
            return Some(Self::Px(value * font_size));
        }
        if let Some(value) = number("rem") {
            return Some(Self::Px(value * root_font_size));
        }
        if let Some(value) = number("pt") {
            return Some(Self::Px(value * 96.0 / 72.0));
        }
        if let Some(value) = number("pc") {
            return Some(Self::Px(value * 16.0));
        }
        if let Some(value) = number("in") {
            return Some(Self::Px(value * 96.0));
        }
        if let Some(value) = number("cm") {
            return Some(Self::Px(value * 96.0 / 2.54));
        }
        if let Some(value) = number("mm") {
            return Some(Self::Px(value * 96.0 / 25.4));
        }
        if let Some(value) = number("vw") {
            return Some(Self::Px(value * viewport.css_width() / 100.0));
        }
        if let Some(value) = number("vh") {
            return Some(Self::Px(value * viewport.css_height() / 100.0));
        }
        if let Some(value) = number("%") {
            return Some(Self::Percent(value));
        }
        None
    }
}

fn parse_calc_expression(
    expression: &str,
    font_size: f32,
    root_font_size: f32,
    viewport: Viewport,
) -> Option<(f32, f32)> {
    let expression = expression.trim();
    if expression.is_empty() {
        return None;
    }
    if let Some(inner) = strip_outer_parentheses(expression) {
        return parse_calc_expression(inner, font_size, root_font_size, viewport);
    }
    let terms = split_calc_terms(expression)?;
    let mut px = 0.0;
    let mut percent = 0.0;
    for (sign, term) in terms {
        let (term_px, term_percent) = parse_calc_term(term, font_size, root_font_size, viewport)?;
        px += sign * term_px;
        percent += sign * term_percent;
    }
    Some((px, percent))
}

fn strip_outer_parentheses(expression: &str) -> Option<&str> {
    if !expression.starts_with('(') || !expression.ends_with(')') {
        return None;
    }
    let mut depth = 0_usize;
    for (index, character) in expression.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index != expression.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }
    (depth == 0).then_some(&expression[1..expression.len() - 1])
}

fn split_calc_terms(expression: &str) -> Option<Vec<(f32, &str)>> {
    let mut terms = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0_usize;
    let mut sign = 1.0;
    for (index, character) in expression.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '+' | '-' if depth == 0 => {
                if index == start {
                    if character == '-' {
                        sign = -sign;
                    }
                    start = index + character.len_utf8();
                } else {
                    terms.push((sign, expression[start..index].trim()));
                    sign = if character == '-' { -1.0 } else { 1.0 };
                    start = index + character.len_utf8();
                }
            }
            _ => {}
        }
    }
    if start < expression.len() {
        terms.push((sign, expression[start..].trim()));
    }
    if terms.is_empty() || terms.iter().any(|(_, term)| term.is_empty()) {
        None
    } else {
        Some(terms)
    }
}

fn parse_calc_term(
    term: &str,
    font_size: f32,
    root_font_size: f32,
    viewport: Viewport,
) -> Option<(f32, f32)> {
    if let Some(inner) = strip_outer_parentheses(term) {
        return parse_calc_expression(inner, font_size, root_font_size, viewport);
    }
    let mut depth = 0_usize;
    for (index, character) in term.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '*' | '/' if depth == 0 => {
                let left = term[..index].trim();
                let right = term[index + 1..].trim();
                let (left_px, left_percent) =
                    parse_calc_expression(left, font_size, root_font_size, viewport)?;
                let factor = right.parse::<f32>().ok()?;
                if character == '/' && factor == 0.0 {
                    return None;
                }
                let factor = if character == '/' {
                    1.0 / factor
                } else {
                    factor
                };
                return Some((left_px * factor, left_percent * factor));
            }
            _ => {}
        }
    }
    let length = Length::parse(term, font_size, root_font_size, viewport)?;
    match length {
        Length::Px(value) => Some((value, 0.0)),
        Length::Percent(value) => Some((0.0, value)),
        Length::Calc { px, percent } => Some((px, percent)),
        Length::Auto => None,
    }
}

/// A font style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FontStyle {
    /// Upright.
    #[default]
    Normal,
    /// Italic.
    Italic,
    /// Oblique.
    Oblique,
}

/// A line height value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum LineHeight {
    /// The font's natural line height.
    #[default]
    Normal,
    /// A multiple of the font size.
    Number(f32),
    /// A fixed length.
    Px(f32),
}

impl LineHeight {
    /// Resolves the line height given a normal line height.
    #[must_use]
    pub(crate) fn resolve(self, font_size: f32, normal: f32) -> f32 {
        match self {
            Self::Normal => normal,
            Self::Number(value) => font_size * value,
            Self::Px(value) => value,
        }
    }
}

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextAlign {
    /// Left edge.
    #[default]
    Left,
    /// Right edge.
    Right,
    /// Center.
    Center,
    /// Both edges.
    Justify,
}

/// Whitespace handling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WhiteSpace {
    /// Collapse whitespace and wrap lines.
    #[default]
    Normal,
    /// Collapse whitespace but do not wrap.
    Nowrap,
    /// Preserve whitespace and newlines.
    Pre,
}

/// Overflow behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Overflow {
    /// Content is visible outside the box.
    #[default]
    Visible,
    /// Content is clipped.
    Hidden,
}

/// A border line style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BorderStyle {
    /// No border.
    #[default]
    None,
    /// A solid line.
    Solid,
    /// A dashed line.
    Dashed,
    /// A dotted line.
    Dotted,
    /// A double line.
    Double,
}

/// A border edge.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Border {
    /// Border width in CSS pixels.
    pub(crate) width: f32,
    /// Border style.
    pub(crate) style: BorderStyle,
    /// Border color; `None` means `currentColor`.
    pub(crate) color: Option<Color>,
}

/// Four border edges.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BorderEdges {
    /// Top border.
    pub(crate) top: Border,
    /// Right border.
    pub(crate) right: Border,
    /// Bottom border.
    pub(crate) bottom: Border,
    /// Left border.
    pub(crate) left: Border,
}

impl BorderEdges {
    /// Returns the border widths.
    #[must_use]
    pub(crate) fn widths(self) -> Edges {
        Edges::new(
            self.top.width,
            self.right.width,
            self.bottom.width,
            self.left.width,
        )
    }
}

/// The four corner radii of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CornerRadii {
    /// Top-left radius.
    pub(crate) top_left: f32,
    /// Top-right radius.
    pub(crate) top_right: f32,
    /// Bottom-right radius.
    pub(crate) bottom_right: f32,
    /// Bottom-left radius.
    pub(crate) bottom_left: f32,
}

impl CornerRadii {
    /// Returns true when all corners are square.
    #[must_use]
    pub(crate) fn is_zero(self) -> bool {
        self.top_left == 0.0
            && self.top_right == 0.0
            && self.bottom_right == 0.0
            && self.bottom_left == 0.0
    }
}

/// Text decoration flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextDecoration {
    /// Underline.
    pub(crate) underline: bool,
    /// Line through.
    pub(crate) line_through: bool,
    /// Overline.
    pub(crate) overline: bool,
}

/// Visibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Visibility {
    /// Visible.
    #[default]
    Visible,
    /// Occupies space but is not painted.
    Hidden,
}

/// A fully computed style.
#[derive(Clone, Debug)]
pub(crate) struct ComputedStyle {
    pub(crate) display: Display,
    pub(crate) position: Position,
    pub(crate) top: Length,
    pub(crate) right: Length,
    pub(crate) bottom: Length,
    pub(crate) left: Length,
    pub(crate) box_sizing: BoxSizing,
    pub(crate) width: Length,
    pub(crate) height: Length,
    pub(crate) min_width: Length,
    pub(crate) min_height: Length,
    pub(crate) max_width: Length,
    pub(crate) max_height: Length,
    pub(crate) margin: Edges,
    pub(crate) margin_auto: [bool; 4],
    pub(crate) padding: Edges,
    pub(crate) border: BorderEdges,
    pub(crate) color: Color,
    pub(crate) background: Color,
    pub(crate) font_family: Vec<String>,
    pub(crate) font_size: f32,
    pub(crate) font_weight: u16,
    pub(crate) font_style: FontStyle,
    pub(crate) line_height: LineHeight,
    pub(crate) text_align: TextAlign,
    pub(crate) white_space: WhiteSpace,
    pub(crate) overflow: Overflow,
    pub(crate) opacity: f32,
    pub(crate) text_decoration: TextDecoration,
    pub(crate) visibility: Visibility,
    pub(crate) custom_properties: HashMap<String, String>,
    pub(crate) flex_direction: FlexDirection,
    pub(crate) flex_wrap: FlexWrap,
    pub(crate) justify_content: JustifyContent,
    pub(crate) align_items: AlignItems,
    pub(crate) align_self: Option<AlignItems>,
    pub(crate) row_gap: f32,
    pub(crate) column_gap: f32,
    pub(crate) flex_grow: f32,
    pub(crate) flex_shrink: f32,
    pub(crate) flex_basis: Length,
    pub(crate) grid_columns: Vec<GridTrack>,
    pub(crate) letter_spacing: f32,
    pub(crate) text_transform: TextTransform,
    pub(crate) border_radius: CornerRadii,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            top: Length::Auto,
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::Auto,
            box_sizing: BoxSizing::ContentBox,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Px(0.0),
            min_height: Length::Px(0.0),
            max_width: Length::Px(f32::INFINITY),
            max_height: Length::Px(f32::INFINITY),
            margin: Edges::default(),
            margin_auto: [false; 4],
            padding: Edges::default(),
            border: BorderEdges::default(),
            color: Color::BLACK,
            background: Color::TRANSPARENT,
            font_family: vec!["sans-serif".to_owned()],
            font_size: 16.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Left,
            white_space: WhiteSpace::Normal,
            overflow: Overflow::Visible,
            opacity: 1.0,
            text_decoration: TextDecoration::default(),
            visibility: Visibility::Visible,
            custom_properties: HashMap::new(),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_self: None,
            row_gap: 0.0,
            column_gap: 0.0,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            grid_columns: Vec::new(),
            letter_spacing: 0.0,
            text_transform: TextTransform::None,
            border_radius: CornerRadii::default(),
        }
    }
}

impl ComputedStyle {
    /// Creates the style for a child element before declarations are applied.
    pub(crate) fn inherited(parent: &Self) -> Self {
        Self {
            color: parent.color,
            font_family: parent.font_family.clone(),
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            font_style: parent.font_style,
            line_height: parent.line_height,
            text_align: parent.text_align,
            white_space: parent.white_space,
            custom_properties: parent.custom_properties.clone(),
            letter_spacing: parent.letter_spacing,
            text_transform: parent.text_transform,
            ..Self::default()
        }
    }

    /// Applies a longhand declaration.
    pub(crate) fn apply(
        &mut self,
        name: &str,
        value: &str,
        parent: &Self,
        root_font_size: f32,
        viewport: Viewport,
        compat: &mut CompatReport,
    ) {
        let value = value.trim();
        match name {
            "display" => {
                self.display = match value.to_ascii_lowercase().as_str() {
                    "block" | "flow-root" => Display::Block,
                    "inline" => Display::Inline,
                    "inline-block" => Display::InlineBlock,
                    "list-item" => Display::ListItem,
                    "flex" => Display::Flex,
                    "inline-flex" => Display::InlineFlex,
                    "grid" => Display::Grid,
                    "inline-grid" => {
                        compat.unsupported("display: inline-grid");
                        Display::Grid
                    }
                    "table" | "inline-table" => Display::Table,
                    "table-row" => Display::TableRow,
                    "table-cell" => Display::TableCell,
                    "table-row-group" | "table-header-group" | "table-footer-group" => {
                        Display::Block
                    }
                    "none" => Display::None,
                    "contents" => Display::Block,
                    _ => {
                        compat.unsupported(format!("display: {value}"));
                        return;
                    }
                };
            }
            "position" => {
                self.position = match value.to_ascii_lowercase().as_str() {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => Position::Static,
                };
                if matches!(
                    self.position,
                    Position::Absolute | Position::Fixed | Position::Sticky
                ) {
                    compat.unsupported(format!("position: {value}"));
                }
            }
            "top" => self.top = self.parse_length(value, parent, root_font_size, viewport, compat),
            "right" => {
                self.right = self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "bottom" => {
                self.bottom = self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "left" => {
                self.left = self.parse_length(value, parent, root_font_size, viewport, compat)
            }
            "box-sizing" => {
                self.box_sizing = if value.eq_ignore_ascii_case("border-box") {
                    BoxSizing::BorderBox
                } else {
                    BoxSizing::ContentBox
                };
            }
            "flex-direction" => {
                self.flex_direction = match value.to_ascii_lowercase().as_str() {
                    "row" => FlexDirection::Row,
                    "row-reverse" => {
                        compat.unsupported("flex-direction: row-reverse");
                        FlexDirection::RowReverse
                    }
                    "column" => FlexDirection::Column,
                    "column-reverse" => {
                        compat.unsupported("flex-direction: column-reverse");
                        FlexDirection::ColumnReverse
                    }
                    _ => FlexDirection::Row,
                };
            }
            "flex-wrap" => {
                self.flex_wrap = match value.to_ascii_lowercase().as_str() {
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => {
                        compat.unsupported("flex-wrap: wrap-reverse");
                        FlexWrap::WrapReverse
                    }
                    _ => FlexWrap::NoWrap,
                };
            }
            "justify-content" => {
                self.justify_content = match value.to_ascii_lowercase().as_str() {
                    "flex-end" | "end" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => JustifyContent::FlexStart,
                };
            }
            "align-items" => {
                self.align_items = parse_align_items(value).unwrap_or(AlignItems::Stretch);
            }
            "align-self" => {
                self.align_self = parse_align_items(value);
            }
            "gap" | "row-gap" | "column-gap" => {
                let parts = value
                    .split_ascii_whitespace()
                    .filter_map(|part| {
                        Length::parse(part, self.font_size, root_font_size, viewport)
                            .and_then(|length| length.resolve(0.0))
                    })
                    .collect::<Vec<_>>();
                match (name, parts.as_slice()) {
                    ("row-gap", [row]) => self.row_gap = *row,
                    ("column-gap", [column]) => self.column_gap = *column,
                    ("gap", [all]) => {
                        self.row_gap = *all;
                        self.column_gap = *all;
                    }
                    ("gap", [row, column]) => {
                        self.row_gap = *row;
                        self.column_gap = *column;
                    }
                    _ => {}
                }
            }
            "flex-grow" => self.flex_grow = value.parse::<f32>().unwrap_or(0.0).max(0.0),
            "flex-shrink" => self.flex_shrink = value.parse::<f32>().unwrap_or(1.0).max(0.0),
            "flex-basis" => {
                self.flex_basis =
                    self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "flex" => apply_flex_shorthand(self, value, parent, root_font_size, viewport, compat),
            "grid-template-columns" => {
                self.grid_columns =
                    parse_grid_columns(value, self.font_size, root_font_size, viewport, compat);
            }
            "width" => {
                self.width = self.parse_length(value, parent, root_font_size, viewport, compat)
            }
            "height" => {
                self.height = self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "min-width" => {
                self.min_width = self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "min-height" => {
                self.min_height =
                    self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "max-width" => {
                self.max_width = self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "max-height" => {
                self.max_height =
                    self.parse_length(value, parent, root_font_size, viewport, compat);
            }
            "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
                apply_edges(
                    &mut self.margin,
                    Some(&mut self.margin_auto),
                    name,
                    value,
                    parent,
                    root_font_size,
                    viewport,
                    compat,
                );
            }
            "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
                apply_edges(
                    &mut self.padding,
                    None,
                    name,
                    value,
                    parent,
                    root_font_size,
                    viewport,
                    compat,
                );
            }
            "border" => {
                let border = parse_border(value, parent, root_font_size, viewport, compat);
                self.border = BorderEdges {
                    top: border,
                    right: border,
                    bottom: border,
                    left: border,
                };
            }
            "border-top" | "border-right" | "border-bottom" | "border-left" => {
                let border = parse_border(value, parent, root_font_size, viewport, compat);
                match name {
                    "border-top" => self.border.top = border,
                    "border-right" => self.border.right = border,
                    "border-bottom" => self.border.bottom = border,
                    _ => self.border.left = border,
                }
            }
            "border-width" => {
                if let Some(values) = parse_four_lengths(value, parent, root_font_size, viewport) {
                    self.border.top.width = values[0];
                    self.border.right.width = values[1];
                    self.border.bottom.width = values[2];
                    self.border.left.width = values[3];
                }
            }
            "border-style" => {
                let styles = value
                    .split_ascii_whitespace()
                    .map(parse_border_style)
                    .collect::<Vec<_>>();
                if let Some(style) = styles.first() {
                    let sides = [styles.first(), styles.get(1), styles.get(2), styles.get(3)];
                    let edge = |index: usize| sides[index].copied().unwrap_or(*style);
                    self.border.top.style = edge(0);
                    self.border.right.style = edge(1);
                    self.border.bottom.style = edge(2);
                    self.border.left.style = edge(3);
                }
            }
            "border-color" => {
                if let Some(color) = Color::parse(value) {
                    self.border.top.color = Some(color);
                    self.border.right.color = Some(color);
                    self.border.bottom.color = Some(color);
                    self.border.left.color = Some(color);
                }
            }
            "border-radius" => {
                if value.contains('/') {
                    compat.unsupported(format!("border-radius ellipse: {value}"));
                }
                let horizontal = value.split('/').next().unwrap_or(value).trim();
                if let Some([top_left, top_right, bottom_right, bottom_left]) =
                    parse_radius_values(horizontal, parent, root_font_size, viewport)
                {
                    self.border_radius = CornerRadii {
                        top_left,
                        top_right,
                        bottom_right,
                        bottom_left,
                    };
                } else {
                    compat.unsupported(format!("border-radius: {value}"));
                }
            }
            "border-top-width" => {
                if let Some(width) = parse_px(value, parent.font_size, root_font_size, viewport) {
                    self.border.top.width = width;
                }
            }
            "border-right-width" => {
                if let Some(width) = parse_px(value, parent.font_size, root_font_size, viewport) {
                    self.border.right.width = width;
                }
            }
            "border-bottom-width" => {
                if let Some(width) = parse_px(value, parent.font_size, root_font_size, viewport) {
                    self.border.bottom.width = width;
                }
            }
            "border-left-width" => {
                if let Some(width) = parse_px(value, parent.font_size, root_font_size, viewport) {
                    self.border.left.width = width;
                }
            }
            "border-top-style" => self.border.top.style = parse_border_style(value),
            "border-right-style" => self.border.right.style = parse_border_style(value),
            "border-bottom-style" => self.border.bottom.style = parse_border_style(value),
            "border-left-style" => self.border.left.style = parse_border_style(value),
            "border-top-color" => self.border.top.color = Color::parse(value),
            "border-right-color" => self.border.right.color = Color::parse(value),
            "border-bottom-color" => self.border.bottom.color = Color::parse(value),
            "border-left-color" => self.border.left.color = Color::parse(value),
            "color" => {
                if value.trim().eq_ignore_ascii_case("inherit") {
                    self.color = parent.color;
                } else if let Some(color) = Color::parse(value) {
                    self.color = color;
                } else {
                    compat.unsupported(format!("color: {value}"));
                }
            }
            "background" => {
                if value.contains("url(") || value.contains("gradient(") {
                    compat.unsupported("background image");
                } else if let Some(color) = Color::parse(value) {
                    self.background = color;
                }
            }
            "background-color" => {
                if let Some(color) = Color::parse(value) {
                    self.background = color;
                }
            }
            "font-family" => self.font_family = parse_font_family(value),
            "font-size" => {
                if let Some(length) =
                    Length::parse(value, parent.font_size, root_font_size, viewport)
                {
                    if let Some(size) = length.resolve(parent.font_size) {
                        self.font_size = size.max(0.0);
                    }
                }
            }
            "font-weight" => {
                self.font_weight = match value.to_ascii_lowercase().as_str() {
                    "normal" => 400,
                    "bold" => 700,
                    "bolder" => (parent.font_weight + 300).min(900),
                    "lighter" => parent.font_weight.saturating_sub(300).max(100),
                    other => other.parse().unwrap_or(400),
                };
            }
            "font-style" => {
                self.font_style = match value.to_ascii_lowercase().as_str() {
                    "italic" => FontStyle::Italic,
                    "oblique" => FontStyle::Oblique,
                    _ => FontStyle::Normal,
                };
            }
            "line-height" => {
                self.line_height = if value.eq_ignore_ascii_case("normal") {
                    LineHeight::Normal
                } else if let Ok(number) = value.parse::<f32>() {
                    LineHeight::Number(number)
                } else if let Some(length) =
                    Length::parse(value, self.font_size, root_font_size, viewport)
                {
                    length
                        .resolve(self.font_size)
                        .map_or(LineHeight::Normal, LineHeight::Px)
                } else {
                    LineHeight::Normal
                };
            }
            "text-align" => {
                self.text_align = match value.to_ascii_lowercase().as_str() {
                    "left" | "start" => TextAlign::Left,
                    "right" | "end" => TextAlign::Right,
                    "center" => TextAlign::Center,
                    "justify" => TextAlign::Justify,
                    _ => TextAlign::Left,
                };
            }
            "letter-spacing" => {
                let value = value.trim();
                if value.eq_ignore_ascii_case("normal") {
                    self.letter_spacing = 0.0;
                } else if let Some(length) =
                    Length::parse(value, self.font_size, root_font_size, viewport)
                {
                    self.letter_spacing = length.resolve(0.0).unwrap_or(0.0);
                } else {
                    compat.unsupported(format!("property: letter-spacing ({value})"));
                }
            }
            "text-transform" => {
                self.text_transform = match value.to_ascii_lowercase().as_str() {
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    "capitalize" => TextTransform::Capitalize,
                    "none" => TextTransform::None,
                    _ => TextTransform::None,
                };
            }
            "text-decoration" => {
                self.text_decoration = TextDecoration {
                    underline: value.contains("underline"),
                    line_through: value.contains("line-through"),
                    overline: value.contains("overline"),
                };
            }
            "white-space" => {
                self.white_space = match value.to_ascii_lowercase().as_str() {
                    "nowrap" => WhiteSpace::Nowrap,
                    "pre" | "pre-wrap" | "pre-line" | "break-spaces" => WhiteSpace::Pre,
                    _ => WhiteSpace::Normal,
                };
            }
            "overflow" | "overflow-x" | "overflow-y" => {
                self.overflow = if value.eq_ignore_ascii_case("visible") {
                    Overflow::Visible
                } else {
                    Overflow::Hidden
                };
            }
            "opacity" => {
                self.opacity = value.parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);
            }
            "visibility" => {
                self.visibility = if value.eq_ignore_ascii_case("hidden") {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                };
            }
            "vertical-align" | "list-style" | "list-style-type" => {}
            _ => {}
        }
    }

    fn parse_length(
        &self,
        value: &str,
        parent: &Self,
        root_font_size: f32,
        viewport: Viewport,
        compat: &mut CompatReport,
    ) -> Length {
        Length::parse(value, parent.font_size, root_font_size, viewport).unwrap_or_else(|| {
            if !matches!(value.to_ascii_lowercase().as_str(), "auto" | "none") {
                compat.unsupported(format!("length: {value}"));
            }
            Length::Auto
        })
    }

    /// Resolves the line height.
    #[must_use]
    pub(crate) fn line_height_px(&self, normal: f32) -> f32 {
        self.line_height.resolve(self.font_size, normal)
    }
}

fn parse_grid_columns(
    value: &str,
    font_size: f32,
    root_font_size: f32,
    viewport: Viewport,
    compat: &mut CompatReport,
) -> Vec<GridTrack> {
    if value.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let tokens = split_top_level_whitespace(value);
    if tokens.len() == 1 {
        if let Some(track) =
            parse_auto_fit_repeat(tokens[0], font_size, root_font_size, viewport, compat)
        {
            return vec![track];
        }
    }
    let mut tracks = Vec::new();
    for token in tokens {
        if !parse_grid_track(
            token,
            font_size,
            root_font_size,
            viewport,
            &mut tracks,
            compat,
        ) {
            compat.unsupported(format!("grid-template-columns: {value}"));
            return Vec::new();
        }
    }
    tracks
}

fn parse_auto_fit_repeat(
    token: &str,
    font_size: f32,
    root_font_size: f32,
    viewport: Viewport,
    compat: &mut CompatReport,
) -> Option<GridTrack> {
    let lower = token.to_ascii_lowercase();
    if !lower.starts_with("repeat(") || !token.ends_with(')') {
        return None;
    }
    let inner = &token["repeat(".len()..token.len() - 1];
    let parts = split_top_level(inner, ',');
    let [count, track_value] = parts.as_slice() else {
        return None;
    };
    let count = count.trim().to_ascii_lowercase();
    if count != "auto-fit" && count != "auto-fill" {
        return None;
    }
    let track_value = track_value.trim();
    let lower_track = track_value.to_ascii_lowercase();
    if !lower_track.starts_with("minmax(") || !track_value.ends_with(')') {
        compat.unsupported(format!("grid-template-columns: {token}"));
        return None;
    }
    let inner = &track_value["minmax(".len()..track_value.len() - 1];
    let parts = split_top_level(inner, ',');
    let [minimum, maximum] = parts.as_slice() else {
        compat.unsupported(format!("grid-template-columns: {token}"));
        return None;
    };
    let min = Length::parse(minimum.trim(), font_size, root_font_size, viewport)
        .and_then(|length| length.resolve(0.0))
        .unwrap_or(0.0);
    let fraction = maximum
        .trim()
        .strip_suffix("fr")
        .and_then(|value| value.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .max(0.1);
    Some(GridTrack::AutoFit { min, fraction })
}

fn parse_grid_track(
    token: &str,
    font_size: f32,
    root_font_size: f32,
    viewport: Viewport,
    tracks: &mut Vec<GridTrack>,
    compat: &mut CompatReport,
) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("repeat(") && token.ends_with(')') {
        let inner = &token["repeat(".len()..token.len() - 1];
        let parts = split_top_level(inner, ',');
        let [count, tracks_value] = parts.as_slice() else {
            return false;
        };
        let Ok(count) = count.trim().parse::<usize>() else {
            compat.unsupported("grid repeat: auto-fit");
            return false;
        };
        let mut repeated = Vec::new();
        for token in split_top_level_whitespace(tracks_value.trim()) {
            if !parse_grid_track(
                token,
                font_size,
                root_font_size,
                viewport,
                &mut repeated,
                compat,
            ) {
                return false;
            }
        }
        for _ in 0..count {
            tracks.extend_from_slice(&repeated);
        }
        return count > 0 && !repeated.is_empty();
    }
    if lower.starts_with("minmax(") && token.ends_with(')') {
        let inner = &token["minmax(".len()..token.len() - 1];
        let parts = split_top_level(inner, ',');
        let Some(max) = parts.get(1) else {
            return false;
        };
        let mut parsed = Vec::new();
        if parse_grid_track(
            max.trim(),
            font_size,
            root_font_size,
            viewport,
            &mut parsed,
            compat,
        ) && parsed.len() == 1
        {
            tracks.extend(parsed);
            return true;
        }
        return false;
    }
    if let Some(value) = token.strip_suffix("fr") {
        if let Ok(value) = value.trim().parse::<f32>() {
            tracks.push(GridTrack::Fr(value.max(0.0)));
            return true;
        }
        return false;
    }
    if matches!(lower.as_str(), "auto" | "min-content" | "max-content") {
        tracks.push(GridTrack::Fr(1.0));
        return true;
    }
    if let Some(length) = Length::parse(token, font_size, root_font_size, viewport) {
        if let Some(value) = length.resolve(0.0) {
            tracks.push(GridTrack::Px(value.max(0.0)));
            return true;
        }
    }
    false
}

fn split_top_level_whitespace(value: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => {
                if start < index {
                    result.push(value[start..index].trim());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < value.len() {
        result.push(value[start..].trim());
    }
    result.into_iter().filter(|part| !part.is_empty()).collect()
}

fn split_top_level(value: &str, separator: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            character if character == separator && depth == 0 => {
                result.push(value[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    result.push(value[start..].trim());
    result
}

fn parse_align_items(value: &str) -> Option<AlignItems> {
    match value.to_ascii_lowercase().as_str() {
        "stretch" => Some(AlignItems::Stretch),
        "flex-start" | "start" => Some(AlignItems::FlexStart),
        "flex-end" | "end" => Some(AlignItems::FlexEnd),
        "center" => Some(AlignItems::Center),
        "baseline" => Some(AlignItems::Baseline),
        _ => None,
    }
}

fn apply_flex_shorthand(
    style: &mut ComputedStyle,
    value: &str,
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: Viewport,
    compat: &mut CompatReport,
) {
    match value.to_ascii_lowercase().as_str() {
        "none" => {
            style.flex_grow = 0.0;
            style.flex_shrink = 0.0;
            style.flex_basis = Length::Auto;
            return;
        }
        "auto" => {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = Length::Auto;
            return;
        }
        "initial" => {
            style.flex_grow = 0.0;
            style.flex_shrink = 1.0;
            style.flex_basis = Length::Auto;
            return;
        }
        _ => {}
    }

    let mut grow = None;
    let mut shrink = None;
    let mut basis = None;
    for part in value.split_ascii_whitespace() {
        if part.eq_ignore_ascii_case("auto") {
            basis = Some(Length::Auto);
        } else if let Ok(number) = part.parse::<f32>() {
            if grow.is_none() {
                grow = Some(number.max(0.0));
            } else if shrink.is_none() {
                shrink = Some(number.max(0.0));
            } else if basis.is_none() {
                basis = Some(Length::Px(number));
            } else {
                compat.unsupported(format!("flex: {value}"));
                return;
            }
        } else if let Some(length) = Length::parse(part, parent.font_size, root_font_size, viewport)
        {
            basis = Some(length);
        } else {
            compat.unsupported(format!("flex: {value}"));
            return;
        }
    }
    style.flex_grow = grow.unwrap_or(1.0);
    style.flex_shrink = shrink.unwrap_or(1.0);
    style.flex_basis = basis.unwrap_or(Length::Percent(0.0));
}

#[allow(clippy::too_many_arguments)]
fn apply_edges(
    edges: &mut Edges,
    auto: Option<&mut [bool; 4]>,
    name: &str,
    value: &str,
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: Viewport,
    compat: &mut CompatReport,
) {
    if name != "margin" && name != "padding" {
        if let Some(auto) = auto {
            let index = match name {
                "margin-top" => Some(0),
                "margin-right" => Some(1),
                "margin-bottom" => Some(2),
                "margin-left" => Some(3),
                _ => None,
            };
            if let Some(index) = index {
                if value.trim().eq_ignore_ascii_case("auto") {
                    auto[index] = true;
                    match index {
                        0 => edges.top = 0.0,
                        1 => edges.right = 0.0,
                        2 => edges.bottom = 0.0,
                        _ => edges.left = 0.0,
                    }
                    return;
                }
                auto[index] = false;
            }
        }
        let Some(length) = Length::parse(value, parent.font_size, root_font_size, viewport) else {
            compat.unsupported(format!("length: {value}"));
            return;
        };
        let value = length.resolve(0.0).unwrap_or(0.0);
        match name {
            "margin-top" => edges.top = value,
            "margin-right" => edges.right = value,
            "margin-bottom" => edges.bottom = value,
            "margin-left" => edges.left = value,
            "padding-top" => edges.top = value,
            "padding-right" => edges.right = value,
            "padding-bottom" => edges.bottom = value,
            "padding-left" => edges.left = value,
            _ => {}
        }
        return;
    }

    let values = value
        .split_ascii_whitespace()
        .filter_map(|part| Length::parse(part, parent.font_size, root_font_size, viewport))
        .map(|length| length.resolve(0.0).unwrap_or(0.0))
        .collect::<Vec<_>>();
    let [top, right, bottom, left] = match values.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => {
            compat.unsupported(format!("{name}: {value}"));
            return;
        }
    };
    *edges = Edges::new(top, right, bottom, left);
}

fn parse_radius_values(
    value: &str,
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: Viewport,
) -> Option<[f32; 4]> {
    let values = value
        .split_ascii_whitespace()
        .filter_map(|part| Length::parse(part, parent.font_size, root_font_size, viewport))
        .map(|length| length.resolve(0.0).unwrap_or(0.0))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [all] => Some([*all, *all, *all, *all]),
        [first, second] => Some([*first, *second, *first, *second]),
        [first, second, third] => Some([*first, *second, *third, *second]),
        [first, second, third, fourth] => Some([*first, *second, *third, *fourth]),
        _ => None,
    }
}

fn parse_four_lengths(
    value: &str,
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: Viewport,
) -> Option<[f32; 4]> {
    let values = value
        .split_ascii_whitespace()
        .filter_map(|part| Length::parse(part, parent.font_size, root_font_size, viewport))
        .map(|length| length.resolve(0.0).unwrap_or(0.0))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [all] => Some([*all, *all, *all, *all]),
        [vertical, horizontal] => Some([*vertical, *horizontal, *vertical, *horizontal]),
        [top, horizontal, bottom] => Some([*top, *horizontal, *bottom, *horizontal]),
        [top, right, bottom, left] => Some([*top, *right, *bottom, *left]),
        _ => None,
    }
}

fn parse_px(value: &str, font_size: f32, root_font_size: f32, viewport: Viewport) -> Option<f32> {
    Length::parse(value, font_size, root_font_size, viewport)?.resolve(0.0)
}

fn parse_border(
    value: &str,
    parent: &ComputedStyle,
    root_font_size: f32,
    viewport: Viewport,
    _compat: &mut CompatReport,
) -> Border {
    let mut border = Border::default();
    for part in value.split_ascii_whitespace() {
        if let Some(width) = parse_px(part, parent.font_size, root_font_size, viewport) {
            border.width = width;
        } else if let Some(style) = parse_border_style_opt(part) {
            border.style = style;
        } else if let Some(color) = Color::parse(part) {
            border.color = Some(color);
        }
    }
    border
}

fn parse_border_style(value: &str) -> BorderStyle {
    parse_border_style_opt(value).unwrap_or(BorderStyle::None)
}

fn parse_border_style_opt(value: &str) -> Option<BorderStyle> {
    match value.to_ascii_lowercase().as_str() {
        "none" | "hidden" => Some(BorderStyle::None),
        "solid" => Some(BorderStyle::Solid),
        "dashed" => Some(BorderStyle::Dashed),
        "dotted" => Some(BorderStyle::Dotted),
        "double" => Some(BorderStyle::Double),
        _ => None,
    }
}

fn parse_font_family(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|family| family.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|family| !family.is_empty())
        .collect()
}

/// Expands a shorthand into longhands before the cascade is resolved.
pub(crate) fn expand_declaration(name: &str, value: &str) -> Vec<(String, String)> {
    match name {
        "margin" | "padding" => {
            let parts: Vec<&str> = value.split_ascii_whitespace().collect();
            let [top, right, bottom, left] = match parts.as_slice() {
                [all] => [*all, *all, *all, *all],
                [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
                [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
                [top, right, bottom, left] => [*top, *right, *bottom, *left],
                _ => return vec![(name.to_owned(), value.to_owned())],
            };
            vec![
                (format!("{name}-top"), top.to_owned()),
                (format!("{name}-right"), right.to_owned()),
                (format!("{name}-bottom"), bottom.to_owned()),
                (format!("{name}-left"), left.to_owned()),
            ]
        }
        "border" => {
            let parts: Vec<&str> = value.split_ascii_whitespace().collect();
            let (width, style, color) = split_border_parts(&parts);
            vec![
                ("border-top-width".to_owned(), width.clone()),
                ("border-right-width".to_owned(), width.clone()),
                ("border-bottom-width".to_owned(), width.clone()),
                ("border-left-width".to_owned(), width),
                ("border-top-style".to_owned(), style.clone()),
                ("border-right-style".to_owned(), style.clone()),
                ("border-bottom-style".to_owned(), style.clone()),
                ("border-left-style".to_owned(), style),
                ("border-top-color".to_owned(), color.clone()),
                ("border-right-color".to_owned(), color.clone()),
                ("border-bottom-color".to_owned(), color.clone()),
                ("border-left-color".to_owned(), color),
            ]
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let suffix = name.trim_start_matches("border-");
            let parts: Vec<&str> = value.split_ascii_whitespace().collect();
            let (width, style, color) = split_border_parts(&parts);
            vec![
                (format!("border-{suffix}-width"), width),
                (format!("border-{suffix}-style"), style),
                (format!("border-{suffix}-color"), color),
            ]
        }
        "font" => {
            let parts: Vec<&str> = value.split_ascii_whitespace().collect();
            let mut result = Vec::new();
            let mut index = 0;
            if let Some(style) = parts
                .get(index)
                .and_then(|part| matches!(*part, "italic" | "oblique").then_some(*part))
            {
                result.push(("font-style".to_owned(), style.to_owned()));
                index += 1;
            }
            if let Some(weight) = parts.get(index).and_then(|part| {
                matches!(*part, "bold" | "bolder" | "lighter")
                    .then_some(*part)
                    .or_else(|| part.parse::<u16>().ok().map(|_| *part))
            }) {
                result.push(("font-weight".to_owned(), weight.to_owned()));
                index += 1;
            }
            if let Some(size) = parts.get(index) {
                result.push(("font-size".to_owned(), (*size).to_owned()));
                index += 1;
            }
            if let Some(line_height) = parts.get(index).and_then(|part| part.strip_prefix('/')) {
                result.push(("line-height".to_owned(), line_height.to_owned()));
                index += 1;
            }
            if index < parts.len() {
                result.push(("font-family".to_owned(), parts[index..].join(" ")));
            }
            if result.is_empty() {
                vec![(name.to_owned(), value.to_owned())]
            } else {
                result
            }
        }
        "background" => {
            let color = value
                .split_ascii_whitespace()
                .find(|part| Color::parse(part).is_some())
                .unwrap_or(value);
            vec![("background-color".to_owned(), color.to_owned())]
        }
        _ => vec![(name.to_owned(), value.to_owned())],
    }
}

fn split_border_parts(parts: &[&str]) -> (String, String, String) {
    let mut width = "3px".to_owned();
    let mut style = "none".to_owned();
    let mut color = "currentColor".to_owned();
    for part in parts {
        if part.parse::<f32>().is_ok()
            || part.ends_with("px")
            || part.ends_with("em")
            || part.ends_with("rem")
        {
            width = (*part).to_owned();
        } else if parse_border_style_opt(part).is_some() {
            style = (*part).to_owned();
        } else if Color::parse(part).is_some() {
            color = (*part).to_owned();
        }
    }
    (width, style, color)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Priority {
    important: bool,
    origin: Origin,
    specificity: u32,
    order: u32,
}

/// Computed styles for every node in a document.
#[derive(Clone, Debug)]
pub(crate) struct StyleTree {
    styles: Vec<ComputedStyle>,
}

impl StyleTree {
    /// Returns the style for a node.
    pub(crate) fn get(&self, node: NodeId) -> &ComputedStyle {
        &self.styles[node.index()]
    }

    /// Builds the style tree from the user-agent and author rules.
    pub(crate) fn build(
        document: &Document,
        ua_rules: &[Rule],
        author_rules: &[Rule],
        viewport: Viewport,
        compat: &mut CompatReport,
    ) -> Self {
        let mut tree = Self {
            styles: vec![ComputedStyle::default(); document.len()],
        };
        let initial = ComputedStyle::default();
        tree.build_node(
            document,
            document.root(),
            &initial,
            ua_rules,
            author_rules,
            viewport,
            compat,
        );
        tree
    }

    #[allow(clippy::too_many_arguments)]
    fn build_node(
        &mut self,
        document: &Document,
        node: NodeId,
        parent: &ComputedStyle,
        ua_rules: &[Rule],
        author_rules: &[Rule],
        viewport: Viewport,
        compat: &mut CompatReport,
    ) {
        let style = if document.element(node).is_some() {
            self.compute_element_style(
                document,
                node,
                parent,
                ua_rules,
                author_rules,
                viewport,
                compat,
            )
        } else {
            parent.clone()
        };
        self.styles[node.index()] = style;
        let parent_style = self.styles[node.index()].clone();
        let children = document.children(node).collect::<Vec<_>>();
        for child in children {
            self.build_node(
                document,
                child,
                &parent_style,
                ua_rules,
                author_rules,
                viewport,
                compat,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_element_style(
        &self,
        document: &Document,
        node: NodeId,
        parent: &ComputedStyle,
        ua_rules: &[Rule],
        author_rules: &[Rule],
        viewport: Viewport,
        compat: &mut CompatReport,
    ) -> ComputedStyle {
        let mut properties: HashMap<String, (Priority, String)> = HashMap::new();
        let mut collect = |rule: &Rule, compat: &mut CompatReport| {
            if !rule.applies(viewport) {
                return;
            }
            for selector in &rule.selectors {
                if !selector.matches(document, node) {
                    continue;
                }
                for declaration in &rule.declarations {
                    let priority = Priority {
                        important: declaration.important,
                        origin: rule.origin,
                        specificity: selector.specificity(),
                        order: rule.order,
                    };
                    insert_declaration(&mut properties, declaration, priority, compat);
                }
            }
        };
        for rule in ua_rules {
            collect(rule, compat);
        }
        for rule in author_rules {
            collect(rule, compat);
        }
        if let Some(raw) = document.attr(node, "style") {
            let declarations = parse_declarations(raw, compat);
            for declaration in declarations {
                let priority = Priority {
                    important: declaration.important,
                    origin: Origin::Inline,
                    specificity: 1000,
                    order: u32::MAX,
                };
                insert_declaration(&mut properties, &declaration, priority, compat);
            }
        }

        let mut style = ComputedStyle::inherited(parent);
        let mut custom = parent.custom_properties.clone();
        for (name, (_, value)) in &properties {
            if name.starts_with("--") {
                custom.insert(name.clone(), value.clone());
            }
        }
        let mut resolved_custom = HashMap::new();
        for name in custom.keys() {
            let mut stack = HashSet::new();
            if let Some(value) = resolve_custom_property(name, &custom, &mut stack) {
                resolved_custom.insert(name.clone(), value);
            }
        }
        style.custom_properties.clone_from(&resolved_custom);

        let root_font_size = 16.0;
        for (name, (_, value)) in properties {
            if name.starts_with("--") {
                continue;
            }
            let mut stack = HashSet::new();
            let Some(value) = resolve_value(&value, &resolved_custom, &mut stack) else {
                compat.unsupported(format!("unresolved variable in {name}"));
                continue;
            };
            style.apply(&name, &value, parent, root_font_size, viewport, compat);
        }
        style
    }
}

fn resolve_custom_property(
    name: &str,
    properties: &HashMap<String, String>,
    stack: &mut HashSet<String>,
) -> Option<String> {
    if !stack.insert(name.to_owned()) {
        return None;
    }
    let value = properties.get(name)?;
    let resolved = resolve_value(value, properties, stack);
    stack.remove(name);
    resolved
}

fn resolve_value(
    value: &str,
    properties: &HashMap<String, String>,
    stack: &mut HashSet<String>,
) -> Option<String> {
    let mut current = value.to_owned();
    loop {
        let lower = current.to_ascii_lowercase();
        let Some(start) = lower.find("var(") else {
            return Some(current);
        };
        let open = start + "var".len();
        let mut depth = 1_usize;
        let mut close = None;
        for (offset, byte) in current.as_bytes()[open + 1..].iter().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + 1 + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let inner = &current[open + 1..close];
        let (name, fallback) = split_variable_arguments(inner)?;
        let replacement = if properties.contains_key(name) {
            resolve_custom_property(name, properties, stack)
        } else {
            None
        }
        .or_else(|| fallback.and_then(|fallback| resolve_value(fallback, properties, stack)))?;
        current.replace_range(start..=close, &replacement);
    }
}

fn split_variable_arguments(value: &str) -> Option<(&str, Option<&str>)> {
    let mut depth = 0_usize;
    for (index, byte) in value.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let name = value[..index].trim();
                if !name.starts_with("--") {
                    return None;
                }
                return Some((name, Some(value[index + 1..].trim())));
            }
            _ => {}
        }
    }
    let name = value.trim();
    name.starts_with("--").then_some((name, None))
}

fn insert_declaration(
    properties: &mut HashMap<String, (Priority, String)>,
    declaration: &Declaration,
    priority: Priority,
    _compat: &mut CompatReport,
) {
    for (name, value) in expand_declaration(&declaration.name, &declaration.value) {
        match properties.get(&name) {
            Some((current, _)) if *current >= priority => {}
            _ => {
                properties.insert(name, (priority, value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::UA_CSS;
    use crate::document::css::{Origin, parse_stylesheet};
    use crate::error::Result;

    fn find(document: &Document, tag: &str) -> NodeId {
        document
            .descendants()
            .find(|node| document.element_name(*node) == Some(tag))
            .unwrap_or_else(|| panic!("missing <{tag}>"))
    }

    #[test]
    fn applies_cascade_media_and_inheritance() -> Result<()> {
        let document = Document::parse(
            "<style>p { color: #ff0000; } @media (max-width: 600px) { p { color: #00ff00; } }</style><p>Hi</p>",
        )?;
        let mut compat = CompatReport::default();
        let ua = parse_stylesheet(UA_CSS, Origin::UserAgent, &mut compat);
        let author = parse_stylesheet(
            "p { color: #ff0000; } @media (max-width: 600px) { p { color: #00ff00; } }",
            Origin::Author,
            &mut compat,
        );

        let p = find(&document, "p");
        let wide = StyleTree::build(
            &document,
            &ua,
            &author,
            Viewport::new(800, 600),
            &mut compat,
        );
        assert_eq!(wide.get(p).color, Color::rgb(255, 0, 0));
        assert_eq!(wide.get(p).font_size, 16.0);
        assert_eq!(wide.get(p).display, Display::Block);

        let narrow = StyleTree::build(
            &document,
            &ua,
            &author,
            Viewport::new(390, 600),
            &mut compat,
        );
        assert_eq!(narrow.get(p).color, Color::rgb(0, 255, 0));

        let style = find(&document, "style");
        assert_eq!(narrow.get(style).display, Display::None);
        Ok(())
    }

    #[test]
    fn resolves_custom_properties_and_fallbacks() -> Result<()> {
        let document = Document::parse(
            "<style>:root { --brand: #3157d5; --space: 12px; } .box { color: var(--brand); padding: var(--space); background: var(--missing, #ffeeaa); }</style><div class=\"box\">Hi</div>",
        )?;
        let mut compat = CompatReport::default();
        let ua = parse_stylesheet(UA_CSS, Origin::UserAgent, &mut compat);
        let author = parse_stylesheet(
            ":root { --brand: #3157d5; --space: 12px; } .box { color: var(--brand); padding: var(--space); background: var(--missing, #ffeeaa); }",
            Origin::Author,
            &mut compat,
        );
        let styles = StyleTree::build(
            &document,
            &ua,
            &author,
            Viewport::new(800, 600),
            &mut compat,
        );
        let div = find(&document, "div");
        let style = styles.get(div);
        assert_eq!(style.color, Color::rgb(49, 87, 213));
        assert_eq!(style.padding.left, 12.0);
        assert_eq!(style.background, Color::rgb(255, 238, 170));
        Ok(())
    }

    #[test]
    fn inherits_custom_properties_and_uses_fallbacks_for_cycles() -> Result<()> {
        let document = Document::parse(
            "<style>:root { --base: 8px; --space: var(--base); --a: var(--b); --b: var(--a); } .box { padding: var(--space); color: var(--a, #ff0000); }</style><div class=\"box\">Hi</div>",
        )?;
        let mut compat = CompatReport::default();
        let ua = parse_stylesheet(UA_CSS, Origin::UserAgent, &mut compat);
        let author = parse_stylesheet(
            ":root { --base: 8px; --space: var(--base); --a: var(--b); --b: var(--a); } .box { padding: var(--space); color: var(--a, #ff0000); }",
            Origin::Author,
            &mut compat,
        );
        let styles = StyleTree::build(
            &document,
            &ua,
            &author,
            Viewport::new(800, 600),
            &mut compat,
        );
        let div = find(&document, "div");
        let style = styles.get(div);
        assert_eq!(style.padding.left, 8.0);
        assert_eq!(style.color, Color::rgb(255, 0, 0));
        Ok(())
    }

    #[test]
    fn parses_text_spacing_and_transform() -> Result<()> {
        let document = Document::parse(
            "<style>.box { letter-spacing: 2px; text-transform: uppercase; }</style><div class=\"box\">Hi</div>",
        )?;
        let mut compat = CompatReport::default();
        let ua = parse_stylesheet(UA_CSS, Origin::UserAgent, &mut compat);
        let author = parse_stylesheet(
            ".box { letter-spacing: 2px; text-transform: uppercase; }",
            Origin::Author,
            &mut compat,
        );
        let styles = StyleTree::build(
            &document,
            &ua,
            &author,
            Viewport::new(800, 600),
            &mut compat,
        );
        let div = find(&document, "div");
        let style = styles.get(div);
        assert_eq!(style.letter_spacing, 2.0);
        assert_eq!(style.text_transform, TextTransform::Uppercase);
        assert!(compat.unsupported.is_empty(), "{:?}", compat.unsupported);
        Ok(())
    }

    #[test]
    fn inline_style_wins_over_author_rule() -> Result<()> {
        let document = Document::parse("<p style='color: #0000ff'>Hi</p>")?;
        let mut compat = CompatReport::default();
        let ua = parse_stylesheet(UA_CSS, Origin::UserAgent, &mut compat);
        let author = parse_stylesheet("p { color: #ff0000; }", Origin::Author, &mut compat);
        let styles = StyleTree::build(
            &document,
            &ua,
            &author,
            Viewport::new(800, 600),
            &mut compat,
        );
        let p = find(&document, "p");
        assert_eq!(styles.get(p).color, Color::rgb(0, 0, 255));
        Ok(())
    }
}
