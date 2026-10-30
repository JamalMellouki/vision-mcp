//! Block layout.

use crate::compat::CompatReport;
use crate::document::style::{
    AlignItems, BoxSizing, ComputedStyle, Display, FlexWrap, GridTrack, JustifyContent, Length,
    StyleTree,
};
use crate::document::{Document, NodeId, NodeKind};
use crate::geom::Rect;
use crate::image::ImageStore;
use crate::snapshot::Viewport;
use crate::text::FontBook;

use super::inline::{self, AtomicItem, InlineItem};
use super::tree::{BoxId, LayoutBox, LayoutTree, LineItem};

/// Builds the layout tree.
pub(crate) fn build_layout(
    document: &Document,
    styles: &StyleTree,
    fonts: &FontBook,
    viewport: Viewport,
    images: &ImageStore,
    compat: &mut CompatReport,
) -> LayoutTree {
    let root_node = document
        .find_first_element("html")
        .or_else(|| document.find_first_element("body"))
        .unwrap_or_else(|| document.root());
    let mut root_style = styles.get(root_node).clone();
    root_style.display = Display::Block;
    let mut builder = LayoutBuilder {
        document,
        styles,
        fonts,
        images,
        compat,
        boxes: Vec::new(),
    };
    let root = builder.layout_block(root_node, root_style, 0.0, 0.0, viewport.css_width());
    if builder.boxes[root.index()].style.background.a == 0 {
        if let Some(body) = document.find_first_element("body") {
            if let Some(body_box) = builder
                .boxes
                .iter()
                .find(|layout_box| layout_box.node == body)
            {
                if body_box.style.background.a > 0 {
                    builder.boxes[root.index()].style.background = body_box.style.background;
                }
            }
        }
    }
    builder.apply_relative_offsets(root, viewport.css_width());
    let height = builder.boxes[root.index()]
        .rect
        .bottom()
        .max(viewport.css_height());
    LayoutTree::new(builder.boxes, root, height)
}

struct LayoutBuilder<'a> {
    document: &'a Document,
    styles: &'a StyleTree,
    fonts: &'a FontBook,
    images: &'a ImageStore,
    compat: &'a mut CompatReport,
    boxes: Vec<LayoutBox>,
}

impl LayoutBuilder<'_> {
    fn layout_block(
        &mut self,
        node: NodeId,
        mut style: ComputedStyle,
        border_x: f32,
        border_y: f32,
        containing_width: f32,
    ) -> BoxId {
        let element_name = self
            .document
            .element(node)
            .map(|element| element.name.as_str())
            .unwrap_or_default();
        let decoded = if element_name == "img" {
            let src = self.document.attr(node, "src").unwrap_or_default();
            let decoded = self.images.get(src);
            if decoded.is_none() && !src.is_empty() {
                self.compat.unsupported(format!("image: {src}"));
            }
            decoded
        } else {
            None
        };
        if let Some(image) = &decoded {
            let attribute_width = self
                .document
                .attr(node, "width")
                .and_then(|value| value.parse::<f32>().ok());
            let attribute_height = self
                .document
                .attr(node, "height")
                .and_then(|value| value.parse::<f32>().ok());
            if style.width == Length::Auto && style.height == Length::Auto {
                if let Some(width) = attribute_width {
                    style.width = Length::Px(width);
                }
                if let Some(height) = attribute_height {
                    style.height = Length::Px(height);
                }
            }
            let natural_width = image.width as f32;
            let natural_height = image.height as f32;
            if natural_width > 0.0 && natural_height > 0.0 {
                match (style.width, style.height) {
                    (Length::Auto, Length::Auto) => {
                        style.width = Length::Px(natural_width);
                        style.height = Length::Px(natural_height);
                    }
                    (Length::Px(width), Length::Auto) => {
                        style.height = Length::Px(width * natural_height / natural_width);
                    }
                    (Length::Auto, Length::Px(height)) => {
                        style.width = Length::Px(height * natural_width / natural_height);
                    }
                    _ => {}
                }
            }
        }
        if style.display == Display::Table {
            return self.layout_table(node, style, border_x, border_y, containing_width);
        }
        let padding = style.padding;
        let border = style.border.widths();
        let margin = style.margin;
        let horizontal = margin.horizontal() + padding.horizontal() + border.horizontal();
        let mut content_width = match style.width.resolve(containing_width) {
            Some(width) if style.box_sizing == BoxSizing::BorderBox => {
                (width - padding.horizontal() - border.horizontal()).max(0.0)
            }
            Some(width) => width,
            None => (containing_width - horizontal).max(0.0),
        };
        let width_extra = padding.horizontal() + border.horizontal();
        let border_box = style.box_sizing == BoxSizing::BorderBox;
        let min_width = style
            .min_width
            .resolve(containing_width)
            .map_or(0.0, |width| {
                if border_box {
                    (width - width_extra).max(0.0)
                } else {
                    width
                }
            });
        let max_width = style
            .max_width
            .resolve(containing_width)
            .map_or(f32::INFINITY, |width| {
                if border_box {
                    (width - width_extra).max(0.0)
                } else {
                    width
                }
            });
        content_width = content_width.clamp(min_width, max_width);
        if let Some(image) = &decoded {
            if style.height == Length::Auto && style.width != Length::Auto {
                let natural_width = image.width as f32;
                let natural_height = image.height as f32;
                if natural_width > 0.0 && natural_height > 0.0 {
                    style.height =
                        Length::Px((content_width * natural_height / natural_width).max(0.0));
                }
            }
        }

        let content_x = border_x + border.left + padding.left;
        let content_y = border_y + border.top + padding.top;

        let id = self.push(LayoutBox {
            node,
            parent: None,
            style: style.clone(),
            rect: Rect::default(),
            content: Rect::default(),
            children: Vec::new(),
            lines: Vec::new(),
            image: decoded.clone(),
            atomic_inline: false,
        });

        let has_direct_text = self.has_direct_text(node);
        if matches!(style.display, Display::Flex | Display::InlineFlex) && !has_direct_text {
            let auto_height =
                self.layout_flex_children(id, node, &style, content_x, content_y, content_width);
            return self.finish_block(
                id,
                &style,
                border_x,
                border_y,
                content_x,
                content_y,
                content_width,
                auto_height,
            );
        }
        if style.display == Display::Grid && !has_direct_text {
            let auto_height =
                self.layout_grid_children(id, node, &style, content_x, content_y, content_width);
            return self.finish_block(
                id,
                &style,
                border_x,
                border_y,
                content_x,
                content_y,
                content_width,
                auto_height,
            );
        }

        let mut cursor_y = content_y;
        let mut previous_margin_bottom: f32 = 0.0;
        let mut has_block = false;
        let mut inline_items = Vec::new();
        if element_name == "li" {
            inline_items.push(InlineItem::text(self.list_marker(node), style.clone()));
        }

        let form_text = if matches!(element_name, "input" | "select" | "textarea") {
            Some(self.form_text(node, element_name))
        } else {
            None
        };
        if let Some(text) = &form_text {
            if !text.is_empty() {
                inline_items.push(InlineItem::text(text.clone(), style.clone()));
            }
        }

        let children = self.document.children(node).collect::<Vec<_>>();
        for child in children {
            if form_text.is_some() {
                break;
            }
            if let NodeKind::Text(text) = &self.document.node(child).kind {
                let child_style = self.styles.get(child).clone();
                if child_style.display != Display::None {
                    inline_items.push(InlineItem::text(text.clone(), child_style));
                }
                continue;
            }
            let Some(element) = self.document.element(child) else {
                continue;
            };
            let child_style = self.styles.get(child).clone();
            if child_style.display == Display::None {
                continue;
            }
            if element.name == "br" {
                inline_items.push(InlineItem::Break);
                continue;
            }
            let block_level =
                child_style.display.is_block_level() && child_style.display != Display::InlineBlock;
            if block_level {
                let height =
                    self.flush_inline(id, &mut inline_items, content_x, content_width, cursor_y);
                cursor_y += height;
                let collapsed = if has_block {
                    previous_margin_bottom.max(child_style.margin.top)
                } else {
                    child_style.margin.top
                };
                let child_y = cursor_y + collapsed;
                let child_x = content_x + child_style.margin.left;
                let child_id =
                    self.layout_block(child, child_style.clone(), child_x, child_y, content_width);
                let child_rect = self.boxes[child_id.index()].rect;
                let left_auto = child_style.margin_auto[3];
                let right_auto = child_style.margin_auto[1];
                if left_auto || right_auto {
                    let free = (content_width - child_rect.width).max(0.0);
                    let dx = if left_auto && right_auto {
                        free / 2.0
                    } else if left_auto {
                        free
                    } else {
                        0.0
                    };
                    if dx > 0.0 {
                        self.shift_subtree(child_id, dx, 0.0);
                    }
                }
                cursor_y = self.boxes[child_id.index()].rect.bottom();
                previous_margin_bottom = child_style.margin.bottom;
                has_block = true;
                self.boxes[child_id.index()].parent = Some(id);
                self.boxes[id.index()].children.push(child_id);
            } else {
                self.collect_inline(id, child, &mut inline_items, content_width);
            }
        }

        let inline_height =
            self.flush_inline(id, &mut inline_items, content_x, content_width, cursor_y);
        cursor_y += inline_height;
        if has_block {
            cursor_y += previous_margin_bottom;
        }

        let auto_height = (cursor_y - content_y).max(0.0);
        self.finish_block(
            id,
            &style,
            border_x,
            border_y,
            content_x,
            content_y,
            content_width,
            auto_height,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_block(
        &mut self,
        id: BoxId,
        style: &ComputedStyle,
        border_x: f32,
        border_y: f32,
        content_x: f32,
        content_y: f32,
        content_width: f32,
        auto_height: f32,
    ) -> BoxId {
        let padding = style.padding;
        let border = style.border.widths();
        let border_box = style.box_sizing == BoxSizing::BorderBox;
        let vertical_extra = padding.vertical() + border.vertical();
        let mut content_height = match style.height.resolve(0.0) {
            Some(height) if border_box => (height - vertical_extra).max(0.0),
            Some(height) => height,
            None => auto_height,
        };
        let min_height = style.min_height.resolve(0.0).map_or(0.0, |height| {
            if border_box {
                (height - vertical_extra).max(0.0)
            } else {
                height
            }
        });
        let max_height = style
            .max_height
            .resolve(0.0)
            .map_or(f32::INFINITY, |height| {
                if border_box {
                    (height - vertical_extra).max(0.0)
                } else {
                    height
                }
            });
        content_height = content_height.clamp(min_height, max_height);

        let border_width = content_width + padding.horizontal() + border.horizontal();
        let border_height = content_height + padding.vertical() + border.vertical();
        let layout_box = &mut self.boxes[id.index()];
        layout_box.rect = Rect::new(border_x, border_y, border_width, border_height);
        layout_box.content = Rect::new(content_x, content_y, content_width, content_height);
        id
    }

    fn has_direct_text(&self, node: NodeId) -> bool {
        self.document.children(node).any(|child| {
            matches!(
                &self.document.node(child).kind,
                NodeKind::Text(text) if !text.trim().is_empty()
            )
        })
    }

    fn layout_flex_children(
        &mut self,
        container: BoxId,
        node: NodeId,
        style: &ComputedStyle,
        content_x: f32,
        content_y: f32,
        content_width: f32,
    ) -> f32 {
        let mut items = Vec::new();
        for child in self.document.children(node).collect::<Vec<_>>() {
            if let NodeKind::Text(text) = &self.document.node(child).kind {
                if text.trim().is_empty() {
                    continue;
                }
                self.compat.unsupported("text in flex container");
                continue;
            }
            let Some(element) = self.document.element(child) else {
                continue;
            };
            if element.name == "br" {
                continue;
            }
            let child_style = self.styles.get(child).clone();
            if child_style.display == Display::None {
                continue;
            }
            items.push((child, child_style));
        }
        if items.is_empty() {
            return 0.0;
        }
        if style.flex_direction.is_row() {
            self.layout_flex_row(
                container,
                style,
                content_x,
                content_y,
                content_width,
                &items,
            )
        } else {
            if style.flex_wrap != FlexWrap::NoWrap {
                self.compat.unsupported("flex-wrap on a column container");
            }
            self.layout_flex_column(
                container,
                style,
                content_x,
                content_y,
                content_width,
                &items,
            )
        }
    }

    fn layout_flex_row(
        &mut self,
        container: BoxId,
        style: &ComputedStyle,
        content_x: f32,
        content_y: f32,
        content_width: f32,
        items: &[(NodeId, ComputedStyle)],
    ) -> f32 {
        let gap = style.column_gap;
        let mut bases = Vec::with_capacity(items.len());
        let mut minimums = Vec::with_capacity(items.len());
        for (node, child_style) in items {
            let (base, minimum) = self.flex_item_width(*node, child_style, content_width);
            bases.push(base);
            minimums.push(minimum);
        }

        let mut lines = Vec::new();
        if style.flex_wrap == FlexWrap::Wrap {
            let mut start = 0;
            let mut line_width = 0.0;
            for (index, base) in bases.iter().enumerate() {
                let extra = base + if index > start { gap } else { 0.0 };
                if index > start && line_width + extra > content_width {
                    lines.push(start..index);
                    start = index;
                    line_width = *base;
                } else {
                    line_width += extra;
                }
            }
            if start < items.len() {
                lines.push(start..items.len());
            }
        } else {
            lines.push(0..items.len());
        }

        let single_line = lines.len() == 1;
        let preferred_cross = if single_line {
            style.height.resolve(0.0).unwrap_or(0.0)
        } else {
            0.0
        };
        let mut y = content_y;
        for range in &lines {
            let line_items = &items[range.clone()];
            let line_bases = &bases[range.clone()];
            let line_minimums = &minimums[range.clone()];
            let height = self.layout_flex_row_line(
                container,
                style,
                content_x,
                y,
                content_width,
                line_items,
                line_bases,
                line_minimums,
                preferred_cross,
            );
            y += height + style.row_gap;
        }
        (y - content_y - style.row_gap).max(0.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_flex_row_line(
        &mut self,
        container: BoxId,
        style: &ComputedStyle,
        content_x: f32,
        content_y: f32,
        content_width: f32,
        items: &[(NodeId, ComputedStyle)],
        bases: &[f32],
        minimums: &[f32],
        preferred_cross: f32,
    ) -> f32 {
        let gap = style.column_gap;
        let gap_total = gap * items.len().saturating_sub(1) as f32;
        let mut free = content_width - bases.iter().sum::<f32>() - gap_total;
        let mut widths = bases.to_vec();
        if free > 0.0 {
            let grow = items
                .iter()
                .map(|(_, child_style)| child_style.flex_grow)
                .sum::<f32>();
            if grow > 0.0 {
                for (index, (_, child_style)) in items.iter().enumerate() {
                    widths[index] += free * child_style.flex_grow / grow;
                }
                free = 0.0;
            }
        } else if free < 0.0 {
            let shrink = items
                .iter()
                .enumerate()
                .map(|(index, (_, child_style))| child_style.flex_shrink * bases[index].max(1.0))
                .sum::<f32>();
            if shrink > 0.0 {
                for (index, (_, child_style)) in items.iter().enumerate() {
                    let reduction =
                        -free * child_style.flex_shrink * bases[index].max(1.0) / shrink;
                    widths[index] = (bases[index] - reduction).max(minimums[index]);
                }
            }
            free = (content_width - widths.iter().sum::<f32>() - gap_total).max(0.0);
        }
        let (start, between) = justify_offsets(style.justify_content, free, items.len());
        let mut x = content_x + start;
        let mut placed = Vec::with_capacity(items.len());
        for (index, (node, child_style)) in items.iter().enumerate() {
            let child_id = self.layout_block(
                *node,
                child_style.clone(),
                x + child_style.margin.left,
                content_y + child_style.margin.top,
                widths[index],
            );
            let rect = self.boxes[child_id.index()].rect;
            placed.push((
                child_id,
                rect.height + child_style.margin.vertical(),
                child_style.align_self.unwrap_or(style.align_items),
            ));
            self.boxes[child_id.index()].parent = Some(container);
            self.boxes[container.index()].children.push(child_id);
            x += widths[index] + gap + between;
        }
        let cross = placed
            .iter()
            .map(|(_, height, _)| *height)
            .fold(preferred_cross, f32::max);
        for (child_id, height, align) in placed {
            let offset = align_offset(align, cross, height);
            if offset != 0.0 {
                self.shift_subtree(child_id, 0.0, offset);
            }
        }
        cross
    }

    fn layout_flex_column(
        &mut self,
        container: BoxId,
        style: &ComputedStyle,
        content_x: f32,
        content_y: f32,
        content_width: f32,
        items: &[(NodeId, ComputedStyle)],
    ) -> f32 {
        let gap = style.row_gap;
        let mut y = content_y;
        for (node, child_style) in items {
            let align = child_style.align_self.unwrap_or(style.align_items);
            let item_width = if align == AlignItems::Stretch {
                content_width
            } else {
                self.flex_item_width(*node, child_style, content_width)
                    .0
                    .min(content_width)
            };
            let child_id = self.layout_block(
                *node,
                child_style.clone(),
                content_x + child_style.margin.left,
                y + child_style.margin.top,
                item_width,
            );
            let rect = self.boxes[child_id.index()].rect;
            let outer_height = rect.height + child_style.margin.vertical();
            if align != AlignItems::Stretch {
                let offset = align_offset(align, content_width, item_width);
                if offset != 0.0 {
                    self.shift_subtree(child_id, offset, 0.0);
                }
            }
            self.boxes[child_id.index()].parent = Some(container);
            self.boxes[container.index()].children.push(child_id);
            y += outer_height + gap;
        }
        (y - content_y - gap).max(0.0)
    }

    fn flex_item_width(&self, node: NodeId, style: &ComputedStyle, available: f32) -> (f32, f32) {
        let padding_border = style.padding.horizontal() + style.border.widths().horizontal();
        let margin = style.margin.horizontal();
        let width_from = |content: f32, border_box: bool| {
            if border_box {
                content + margin
            } else {
                content + padding_border + margin
            }
        };
        if let Some(basis) = style.flex_basis.resolve(available) {
            return (
                width_from(basis, style.box_sizing == BoxSizing::BorderBox),
                width_from(basis, style.box_sizing == BoxSizing::BorderBox),
            );
        }
        if let Some(width) = style.width.resolve(available) {
            let base = width_from(width, style.box_sizing == BoxSizing::BorderBox);
            return (base, base);
        }
        let (min, max) = if self.is_atomic_inline(node, style.display) {
            let width = self.atomic_width(node, style, available);
            (width, width)
        } else {
            let (min, max) = self.intrinsic_width(node, style, available);
            (
                min + padding_border + margin,
                max.max(min) + padding_border + margin,
            )
        };
        (max, min)
    }

    fn layout_grid_children(
        &mut self,
        container: BoxId,
        node: NodeId,
        style: &ComputedStyle,
        content_x: f32,
        content_y: f32,
        content_width: f32,
    ) -> f32 {
        let mut items = Vec::new();
        for child in self.document.children(node).collect::<Vec<_>>() {
            if let NodeKind::Text(text) = &self.document.node(child).kind {
                if text.trim().is_empty() {
                    continue;
                }
                continue;
            }
            let Some(element) = self.document.element(child) else {
                continue;
            };
            if element.name == "br" {
                continue;
            }
            let child_style = self.styles.get(child).clone();
            if child_style.display == Display::None {
                continue;
            }
            items.push((child, child_style));
        }
        if items.is_empty() {
            return 0.0;
        }
        let tracks = if style.grid_columns.is_empty() {
            vec![GridTrack::Fr(1.0)]
        } else {
            style.grid_columns.clone()
        };
        let widths = resolve_grid_tracks(&tracks, content_width, style.column_gap);
        let columns = widths.len().max(1);
        let mut column_x = Vec::with_capacity(columns);
        let mut x = content_x;
        for width in &widths {
            column_x.push(x);
            x += width + style.column_gap;
        }
        let mut row_y = content_y;
        let mut row_height = 0.0_f32;
        for (index, (node, child_style)) in items.iter().enumerate() {
            let column = index % columns;
            if column == 0 && index > 0 {
                row_y += row_height + style.row_gap;
                row_height = 0.0;
            }
            let child_id = self.layout_block(
                *node,
                child_style.clone(),
                column_x[column] + child_style.margin.left,
                row_y + child_style.margin.top,
                widths[column],
            );
            let rect = self.boxes[child_id.index()].rect;
            row_height = row_height.max(rect.height + child_style.margin.vertical());
            self.boxes[child_id.index()].parent = Some(container);
            self.boxes[container.index()].children.push(child_id);
        }
        (row_y + row_height - content_y).max(0.0)
    }

    fn flush_inline(
        &mut self,
        box_id: BoxId,
        items: &mut Vec<InlineItem>,
        x: f32,
        width: f32,
        y: f32,
    ) -> f32 {
        if items.is_empty() {
            return 0.0;
        }
        let lines = inline::layout(items, x, width, y, self.fonts, self.compat);
        let height = lines.iter().map(|line| line.height).sum();
        let mut shifts = Vec::new();
        for line in &lines {
            for item in &line.items {
                if let LineItem::Atomic(fragment) = item {
                    let current = self.boxes[fragment.box_id.index()].rect;
                    shifts.push((
                        fragment.box_id,
                        fragment.x - current.x,
                        fragment.y - current.y,
                    ));
                }
            }
        }
        for (box_id, dx, dy) in shifts {
            self.shift_subtree(box_id, dx, dy);
        }
        self.boxes[box_id.index()].lines.extend(lines);
        items.clear();
        height
    }

    fn collect_inline(
        &mut self,
        container: BoxId,
        node: NodeId,
        items: &mut Vec<InlineItem>,
        available_width: f32,
    ) {
        if let NodeKind::Text(text) = &self.document.node(node).kind {
            items.push(InlineItem::text(
                text.clone(),
                self.styles.get(node).clone(),
            ));
            return;
        }
        let Some(element) = self.document.element(node) else {
            return;
        };
        let style = self.styles.get(node).clone();
        if style.display == Display::None {
            return;
        }
        if element.name == "br" {
            items.push(InlineItem::Break);
            return;
        }
        if self.is_atomic_inline(node, style.display) {
            let atomic = self.layout_atomic_inline(container, node, style, available_width);
            items.push(InlineItem::Atomic(atomic));
            return;
        }
        if style.display.is_block_level() {
            self.compat
                .unsupported(format!("block inside inline: <{}>", element.name));
            return;
        }
        let children = self.document.children(node).collect::<Vec<_>>();
        for child in children {
            self.collect_inline(container, child, items, available_width);
        }
    }

    fn is_atomic_inline(&self, node: NodeId, display: Display) -> bool {
        matches!(display, Display::InlineBlock | Display::InlineFlex)
            || matches!(
                self.document.element_name(node),
                Some("img" | "input" | "select" | "textarea" | "button")
            )
    }

    fn layout_atomic_inline(
        &mut self,
        container: BoxId,
        node: NodeId,
        mut style: ComputedStyle,
        available_width: f32,
    ) -> AtomicItem {
        let name = self.document.element_name(node).unwrap_or_default();
        let margin = style.margin;
        let padding = style.padding;
        let border = style.border.widths();
        let horizontal = margin.horizontal() + padding.horizontal() + border.horizontal();
        let available_content = (available_width - horizontal).max(0.0);
        let replaced = matches!(name, "img" | "input" | "select" | "textarea");
        if !replaced && style.width == Length::Auto {
            let (min, max) = self.intrinsic_width(node, &style, available_content);
            let min = min.max(0.0);
            let max = max.max(min);
            let content_width = if max <= available_content {
                max
            } else {
                available_content.max(min).min(max)
            };
            style.width = if style.box_sizing == BoxSizing::BorderBox {
                Length::Px(content_width + padding.horizontal() + border.horizontal())
            } else {
                Length::Px(content_width)
            };
        }
        let id = self.layout_block(node, style, 0.0, 0.0, available_width);
        let layout_box = &self.boxes[id.index()];
        let rect = layout_box.rect;
        let style = layout_box.style.clone();
        let (ascent, descent) = self.atomic_metrics(id, margin);
        self.boxes[id.index()].atomic_inline = true;
        self.boxes[id.index()].parent = Some(container);
        self.boxes[container.index()].children.push(id);
        AtomicItem {
            box_id: id,
            outer_width: rect.width + margin.horizontal(),
            ascent,
            descent,
            margin_left: margin.left,
            margin_top: margin.top,
            font_size: style.font_size,
        }
    }

    fn atomic_metrics(&self, id: BoxId, margin: crate::geom::Edges) -> (f32, f32) {
        let layout_box = &self.boxes[id.index()];
        let outer_height = margin.top + layout_box.rect.height + margin.bottom;
        let Some(line) = layout_box.lines.last() else {
            return (outer_height, 0.0);
        };
        let baseline = (line.baseline - layout_box.rect.y + margin.top).max(0.0);
        (baseline, (outer_height - baseline).max(0.0))
    }

    fn intrinsic_width(&self, node: NodeId, style: &ComputedStyle, available: f32) -> (f32, f32) {
        if let Some(width) = style.width.resolve(available) {
            return (width, width);
        }
        if matches!(style.display, Display::Flex | Display::InlineFlex)
            && style.flex_direction.is_row()
        {
            return self.flex_row_intrinsic_width(node, style, available);
        }
        let mut min = 0.0_f32;
        let mut max = 0.0_f32;
        let children = self.document.children(node).collect::<Vec<_>>();
        let mut line_max = 0.0_f32;
        let mut line_min = 0.0_f32;
        for child in children {
            self.measure_inline(
                child,
                available,
                &mut line_max,
                &mut line_min,
                &mut min,
                &mut max,
            );
        }
        min = min.max(line_min);
        max = max.max(line_max);
        if style.white_space == crate::document::style::WhiteSpace::Nowrap {
            min = max;
        }
        (min, max)
    }

    fn flex_row_intrinsic_width(
        &self,
        node: NodeId,
        style: &ComputedStyle,
        available: f32,
    ) -> (f32, f32) {
        let mut min = 0.0_f32;
        let mut max = 0.0_f32;
        let mut count = 0_usize;
        for child in self.document.children(node) {
            if let NodeKind::Text(text) = &self.document.node(child).kind {
                if text.trim().is_empty() {
                    continue;
                }
                let child_style = self.styles.get(child);
                max += self.text_width(text, child_style);
                min = min.max(self.min_text_width(text, child_style));
                count += 1;
                continue;
            }
            let Some(element) = self.document.element(child) else {
                continue;
            };
            if element.name == "br" {
                continue;
            }
            let child_style = self.styles.get(child);
            if child_style.display == Display::None {
                continue;
            }
            let (child_min, child_max) = if self.is_atomic_inline(child, child_style.display) {
                let width = self.atomic_width(child, child_style, available);
                (width, width)
            } else {
                let padding_border =
                    child_style.padding.horizontal() + child_style.border.widths().horizontal();
                let margin = child_style.margin.horizontal();
                let (min, max) = self.intrinsic_width(child, child_style, available);
                (
                    min + padding_border + margin,
                    max.max(min) + padding_border + margin,
                )
            };
            max += child_max;
            min = min.max(child_min);
            count += 1;
        }
        let gaps = style.column_gap * count.saturating_sub(1) as f32;
        (min + gaps, max + gaps)
    }

    fn measure_inline(
        &self,
        node: NodeId,
        available: f32,
        line_max: &mut f32,
        line_min: &mut f32,
        block_min: &mut f32,
        block_max: &mut f32,
    ) {
        if let NodeKind::Text(text) = &self.document.node(node).kind {
            let style = self.styles.get(node);
            let width = self.text_width(text, style);
            *line_max += width;
            *line_min = line_min.max(self.min_text_width(text, style));
            return;
        }
        let Some(element) = self.document.element(node) else {
            return;
        };
        let style = self.styles.get(node).clone();
        if style.display == Display::None {
            return;
        }
        if element.name == "br" {
            *block_max = block_max.max(*line_max);
            *block_min = block_min.max(*line_min);
            *line_max = 0.0;
            *line_min = 0.0;
            return;
        }
        if self.is_atomic_inline(node, style.display) {
            let width = self.atomic_width(node, &style, available);
            *line_max += width;
            *line_min = line_min.max(width);
            return;
        }
        if style.display.is_block_level() && style.display != Display::InlineBlock {
            *block_max = block_max.max(*line_max);
            *block_min = block_min.max(*line_min);
            *line_max = 0.0;
            *line_min = 0.0;
            let (min, max) = self.intrinsic_width(node, &style, available);
            *block_min = block_min.max(min);
            *block_max = block_max.max(max);
            return;
        }
        let children = self.document.children(node).collect::<Vec<_>>();
        for child in children {
            self.measure_inline(child, available, line_max, line_min, block_min, block_max);
        }
    }

    fn atomic_width(&self, node: NodeId, style: &ComputedStyle, available: f32) -> f32 {
        let margin = style.margin;
        let padding = style.padding;
        let border = style.border.widths();
        let horizontal = margin.horizontal() + padding.horizontal() + border.horizontal();
        let content_available = (available - horizontal).max(0.0);
        let name = self.document.element_name(node).unwrap_or_default();
        let content = if let Some(width) = style.width.resolve(content_available) {
            width
        } else if name == "img" {
            let src = self.document.attr(node, "src").unwrap_or_default();
            self.images.get(src).map_or(0.0, |image| image.width as f32)
        } else {
            let (min, max) = self.intrinsic_width(node, style, content_available);
            if max <= content_available {
                max
            } else {
                content_available.max(min).min(max)
            }
        };
        content + horizontal
    }

    fn text_width(&self, text: &str, style: &ComputedStyle) -> f32 {
        self.fonts.shape(text, style).map_or(
            text.chars().count() as f32 * style.font_size * 0.5,
            |shaped| shaped.width,
        )
    }

    fn min_text_width(&self, text: &str, style: &ComputedStyle) -> f32 {
        if style.white_space == crate::document::style::WhiteSpace::Pre {
            return text
                .split('\n')
                .map(|line| self.text_width(line, style))
                .fold(0.0, f32::max);
        }
        text.split_whitespace()
            .map(|word| self.text_width(word, style))
            .fold(0.0, f32::max)
    }

    fn apply_relative_offsets(&mut self, id: BoxId, containing_width: f32) {
        let (position, dx, dy) = {
            let style = &self.boxes[id.index()].style;
            if style.position != crate::document::style::Position::Relative {
                (style.position, 0.0, 0.0)
            } else {
                let dx = style
                    .left
                    .resolve(containing_width)
                    .or_else(|| style.right.resolve(containing_width).map(|value| -value))
                    .unwrap_or(0.0);
                let dy = style
                    .top
                    .resolve(0.0)
                    .or_else(|| style.bottom.resolve(0.0).map(|value| -value))
                    .unwrap_or(0.0);
                (style.position, dx, dy)
            }
        };
        if position == crate::document::style::Position::Relative && (dx != 0.0 || dy != 0.0) {
            self.shift_subtree(id, dx, dy);
        }
        let children = self.boxes[id.index()].children.clone();
        for child in children {
            let width = self.boxes[child.index()].content.width;
            self.apply_relative_offsets(child, width);
        }
    }

    fn shift_subtree(&mut self, id: BoxId, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let children = {
            let layout_box = &mut self.boxes[id.index()];
            layout_box.rect = layout_box.rect.translate(dx, dy);
            layout_box.content = layout_box.content.translate(dx, dy);
            for line in &mut layout_box.lines {
                line.baseline += dy;
                for item in &mut line.items {
                    match item {
                        LineItem::Text(fragment) => fragment.x += dx,
                        LineItem::Atomic(fragment) => {
                            fragment.x += dx;
                            fragment.y += dy;
                        }
                    }
                }
            }
            layout_box.children.clone()
        };
        for child in children {
            self.shift_subtree(child, dx, dy);
        }
    }

    fn layout_table(
        &mut self,
        node: NodeId,
        style: ComputedStyle,
        border_x: f32,
        border_y: f32,
        containing_width: f32,
    ) -> BoxId {
        let padding = style.padding;
        let border = style.border.widths();
        let margin = style.margin;
        let horizontal = margin.horizontal() + padding.horizontal() + border.horizontal();
        let mut content_width = match style.width.resolve(containing_width) {
            Some(width) if style.box_sizing == BoxSizing::BorderBox => {
                (width - padding.horizontal() - border.horizontal()).max(0.0)
            }
            Some(width) => width,
            None => (containing_width - horizontal).max(0.0),
        };
        let width_extra = padding.horizontal() + border.horizontal();
        let border_box = style.box_sizing == BoxSizing::BorderBox;
        let min_width = style
            .min_width
            .resolve(containing_width)
            .map_or(0.0, |width| {
                if border_box {
                    (width - width_extra).max(0.0)
                } else {
                    width
                }
            });
        let max_width = style
            .max_width
            .resolve(containing_width)
            .map_or(f32::INFINITY, |width| {
                if border_box {
                    (width - width_extra).max(0.0)
                } else {
                    width
                }
            });
        content_width = content_width.clamp(min_width, max_width);
        let content_x = border_x + border.left + padding.left;
        let content_y = border_y + border.top + padding.top;

        let id = self.push(LayoutBox {
            node,
            parent: None,
            style: style.clone(),
            rect: Rect::default(),
            content: Rect::default(),
            children: Vec::new(),
            lines: Vec::new(),
            image: None,
            atomic_inline: false,
        });

        let mut rows: Vec<NodeId> = Vec::new();
        let mut caption = None;
        for child in self.document.children(node) {
            match self.document.element_name(child) {
                Some("tr") => rows.push(child),
                Some("thead" | "tbody" | "tfoot") => {
                    for row in self.document.children(child) {
                        if self.document.element_name(row) == Some("tr") {
                            rows.push(row);
                        }
                    }
                }
                Some("caption") => caption = Some(child),
                _ => {}
            }
        }

        let mut row_cells: Vec<Vec<NodeId>> = Vec::with_capacity(rows.len());
        let mut columns = 0;
        for row in &rows {
            let cells = self
                .document
                .children(*row)
                .filter(|cell| matches!(self.document.element_name(*cell), Some("td" | "th")))
                .collect::<Vec<_>>();
            let row_columns = cells.iter().map(|cell| self.colspan(*cell)).sum::<usize>();
            columns = columns.max(row_columns);
            row_cells.push(cells);
        }
        let column_width = if columns == 0 {
            content_width
        } else {
            content_width / columns as f32
        };

        let mut row_y = content_y;
        if let Some(caption) = caption {
            let caption_style = self.styles.get(caption).clone();
            let caption_x = content_x + caption_style.margin.left;
            let caption_y = row_y + caption_style.margin.top;
            let caption_id = self.layout_block(
                caption,
                caption_style.clone(),
                caption_x,
                caption_y,
                content_width,
            );
            row_y = self.boxes[caption_id.index()].rect.bottom() + caption_style.margin.bottom;
            self.boxes[caption_id.index()].parent = Some(id);
            self.boxes[id.index()].children.push(caption_id);
        }
        for (row, cells) in rows.iter().zip(row_cells.iter()) {
            let row_style = self.styles.get(*row).clone();
            let row_id = self.push(LayoutBox {
                node: *row,
                parent: None,
                style: row_style,
                rect: Rect::new(content_x, row_y, content_width, 0.0),
                content: Rect::new(content_x, row_y, content_width, 0.0),
                children: Vec::new(),
                lines: Vec::new(),
                image: None,
                atomic_inline: false,
            });
            let mut row_height: f32 = 0.0;
            let mut column = 0;
            for cell in cells {
                let cell_style = self.styles.get(*cell).clone();
                let span = self.colspan(*cell);
                let cell_width = column_width * span as f32;
                let cell_x = content_x + column as f32 * column_width + cell_style.margin.left;
                let cell_y = row_y + cell_style.margin.top;
                if self.rowspan(*cell) > 1 {
                    self.compat.unsupported("table rowspan");
                }
                let cell_id =
                    self.layout_block(*cell, cell_style.clone(), cell_x, cell_y, cell_width);
                row_height = row_height.max(
                    self.boxes[cell_id.index()].rect.bottom() - row_y + cell_style.margin.bottom,
                );
                column += span;
                self.boxes[cell_id.index()].parent = Some(row_id);
                self.boxes[row_id.index()].children.push(cell_id);
            }
            let row_box = &mut self.boxes[row_id.index()];
            row_box.rect.height = row_height;
            row_box.content.height = row_height;
            row_y += row_height;
            self.boxes[row_id.index()].parent = Some(id);
            self.boxes[id.index()].children.push(row_id);
        }

        let content_height = (row_y - content_y).max(0.0);
        let border_width = content_width + padding.horizontal() + border.horizontal();
        let border_height = content_height + padding.vertical() + border.vertical();
        let table_box = &mut self.boxes[id.index()];
        table_box.rect = Rect::new(border_x, border_y, border_width, border_height);
        table_box.content = Rect::new(content_x, content_y, content_width, content_height);
        id
    }

    fn colspan(&self, cell: NodeId) -> usize {
        self.document
            .attr(cell, "colspan")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1)
            .min(128)
    }

    fn rowspan(&self, cell: NodeId) -> usize {
        self.document
            .attr(cell, "rowspan")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1)
    }

    fn list_marker(&self, node: NodeId) -> String {
        let parent = self.document.parent(node);
        let ordered = parent
            .and_then(|parent| self.document.element_name(parent))
            .is_some_and(|name| name == "ol");
        if !ordered {
            return "\u{2022} ".to_owned();
        }
        let mut index = 1;
        if let Some(parent) = parent {
            for sibling in self.document.children(parent) {
                if sibling == node {
                    break;
                }
                if self.document.element_name(sibling) == Some("li") {
                    index += 1;
                }
            }
        }
        format!("{index}. ")
    }

    fn form_text(&self, node: NodeId, name: &str) -> String {
        match name {
            "input" => self
                .document
                .attr(node, "value")
                .or_else(|| self.document.attr(node, "placeholder"))
                .unwrap_or_default()
                .to_owned(),
            "textarea" => self
                .document
                .attr(node, "value")
                .map(str::to_owned)
                .unwrap_or_else(|| self.document.text_content(node).trim().to_owned()),
            "select" => {
                let mut first = String::new();
                for candidate in self.document.descendants() {
                    if self.document.element_name(candidate) != Some("option") {
                        continue;
                    }
                    if !self.is_descendant(candidate, node) {
                        continue;
                    }
                    let text = self.document.visible_text(candidate);
                    if self.document.attr(candidate, "selected").is_some() {
                        return text.trim().to_owned();
                    }
                    if first.is_empty() {
                        first = text.trim().to_owned();
                    }
                }
                first
            }
            _ => String::new(),
        }
    }

    fn is_descendant(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut current = self.document.parent(node);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.document.parent(parent);
        }
        false
    }

    fn push(&mut self, layout_box: LayoutBox) -> BoxId {
        let id = BoxId(self.boxes.len() as u32);
        self.boxes.push(layout_box);
        id
    }
}

fn resolve_grid_tracks(tracks: &[GridTrack], width: f32, gap: f32) -> Vec<f32> {
    if let [GridTrack::AutoFit { min, .. }] = tracks {
        let min = min.max(1.0);
        let count = ((width + gap) / (min + gap)).floor().max(1.0) as usize;
        let track_width = ((width - gap * count.saturating_sub(1) as f32) / count as f32).max(0.0);
        return vec![track_width; count];
    }
    let gaps = gap * tracks.len().saturating_sub(1) as f32;
    let fixed = tracks
        .iter()
        .map(|track| match track {
            GridTrack::Px(value) => *value,
            GridTrack::Fr(_) | GridTrack::AutoFit { .. } => 0.0,
        })
        .sum::<f32>();
    let fractions = tracks
        .iter()
        .map(|track| match track {
            GridTrack::Fr(value) => value.max(0.0),
            GridTrack::Px(_) | GridTrack::AutoFit { .. } => 0.0,
        })
        .sum::<f32>();
    let remaining = (width - fixed - gaps).max(0.0);
    tracks
        .iter()
        .map(|track| match track {
            GridTrack::Px(value) => *value,
            GridTrack::Fr(value) if fractions > 0.0 => remaining * value.max(0.0) / fractions,
            GridTrack::Fr(_) | GridTrack::AutoFit { .. } => 0.0,
        })
        .collect()
}

fn justify_offsets(justify: JustifyContent, free: f32, count: usize) -> (f32, f32) {
    let free = free.max(0.0);
    match justify {
        JustifyContent::FlexStart => (0.0, 0.0),
        JustifyContent::FlexEnd => (free, 0.0),
        JustifyContent::Center => (free / 2.0, 0.0),
        JustifyContent::SpaceBetween if count > 1 => (0.0, free / (count - 1) as f32),
        JustifyContent::SpaceAround if count > 0 => {
            let between = free / count as f32;
            (between / 2.0, between)
        }
        JustifyContent::SpaceEvenly if count > 0 => {
            let between = free / (count + 1) as f32;
            (between, between)
        }
        _ => (0.0, 0.0),
    }
}

fn align_offset(align: AlignItems, cross: f32, size: f32) -> f32 {
    match align {
        AlignItems::FlexEnd => (cross - size).max(0.0),
        AlignItems::Center => ((cross - size) / 2.0).max(0.0),
        AlignItems::Stretch | AlignItems::FlexStart | AlignItems::Baseline => 0.0,
    }
}
