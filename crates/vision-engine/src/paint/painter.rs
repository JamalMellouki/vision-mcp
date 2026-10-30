//! Painting the layout tree into a pixmap.

use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Transform,
};

use crate::color::Color;
use crate::compat::CompatReport;
use crate::document::Document;
use crate::document::style::{BorderStyle, CornerRadii, Overflow, Visibility};
use crate::error::{Error, Result};
use crate::geom::{Point, Rect};
use crate::layout::{BoxId, LayoutTree, LineBox, LineItem, TextFragment};
use crate::snapshot::Viewport;
use crate::text::GlyphRasterizer;

/// Paints the layout tree.
pub(crate) fn paint(
    layout: &LayoutTree,
    viewport: Viewport,
    scroll: Point,
    document: &Document,
    compat: &mut CompatReport,
) -> Result<Pixmap> {
    const MAX_PAGE_HEIGHT: f32 = 16_384.0;

    let width = (viewport.width as f32 * viewport.dpr).round().max(1.0) as u32;
    let mut page_height = layout.height().max(viewport.css_height());
    if page_height > MAX_PAGE_HEIGHT {
        compat.unsupported(format!("page height above {MAX_PAGE_HEIGHT}px"));
        page_height = MAX_PAGE_HEIGHT;
    }
    let page_pixels = (page_height * viewport.dpr).round().max(1.0) as u32;
    let Some(mut pixmap) = Pixmap::new(width, page_pixels) else {
        return Err(Error::Render("cannot allocate pixmap".to_owned()));
    };
    pixmap.fill(Color::WHITE.to_tiny_skia());

    let mut painter = Painter {
        pixmap,
        rasterizer: GlyphRasterizer::new(),
        dpr: viewport.dpr,
        document,
        compat,
    };

    let root = layout.get(layout.root());
    if root.style.background.a > 0 {
        painter.fill(
            Rect::new(0.0, 0.0, viewport.css_width(), page_height),
            root.style.background,
            None,
        );
    }
    let clip = Rect::new(0.0, 0.0, viewport.css_width(), page_height);
    painter.paint_box(layout, layout.root(), clip);

    let view_pixels = (viewport.height as f32 * viewport.dpr).round().max(1.0) as u32;
    if page_pixels == view_pixels && scroll.x == 0.0 && scroll.y == 0.0 {
        return Ok(painter.pixmap);
    }
    let Some(mut output) = Pixmap::new(width, view_pixels) else {
        return Err(Error::Render("cannot allocate output pixmap".to_owned()));
    };
    let max_scroll = (page_height - viewport.css_height()).max(0.0);
    let x = -(scroll.x.clamp(0.0, 0.0) * viewport.dpr).round() as i32;
    let y = -(scroll.y.clamp(0.0, max_scroll) * viewport.dpr).round() as i32;
    output.draw_pixmap(
        x,
        y,
        painter.pixmap.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Ok(output)
}

struct Painter<'a> {
    pixmap: Pixmap,
    rasterizer: GlyphRasterizer,
    dpr: f32,
    document: &'a Document,
    compat: &'a mut CompatReport,
}

impl Painter<'_> {
    fn paint_box(&mut self, layout: &LayoutTree, id: BoxId, outer_clip: Rect) {
        let layout_box = layout.get(id);
        if layout_box.style.visibility == Visibility::Hidden {
            return;
        }
        let background = with_opacity(layout_box.style.background, layout_box.style.opacity);
        if background.a > 0 {
            if layout_box.style.border_radius.is_zero() {
                self.fill(layout_box.rect, background, Some(outer_clip));
            } else {
                self.fill_rounded(
                    layout_box.rect,
                    background,
                    layout_box.style.border_radius,
                    Some(outer_clip),
                );
            }
        }
        self.borders(
            layout_box.rect,
            &layout_box.style.border,
            layout_box.style.color,
            outer_clip,
            layout_box.style.border_radius,
        );
        self.control(layout_box);

        if let Some(image) = &layout_box.image {
            self.image(image, layout_box.content, layout_box.style.opacity);
        }

        let child_clip = if layout_box.style.overflow == Overflow::Hidden {
            outer_clip
                .intersection(layout_box.rect)
                .unwrap_or(outer_clip)
        } else {
            outer_clip
        };

        for line in &layout_box.lines {
            self.line(layout, line, layout_box.style.opacity, child_clip);
        }
        for child in &layout_box.children {
            if !layout.get(*child).atomic_inline {
                self.paint_box(layout, *child, child_clip);
            }
        }
    }

    fn control(&mut self, layout_box: &crate::layout::LayoutBox) {
        let Some(element) = self.document.element(layout_box.node) else {
            return;
        };
        if element.name != "input" {
            return;
        }
        let kind = self
            .document
            .attr(layout_box.node, "type")
            .unwrap_or("text")
            .to_ascii_lowercase();
        if !matches!(kind.as_str(), "checkbox" | "radio") {
            return;
        }
        if self.document.attr(layout_box.node, "checked").is_none() {
            return;
        }
        let rect = layout_box.content;
        if rect.is_empty() {
            return;
        }
        let color = layout_box.style.color;
        if kind == "radio" {
            let cx = (rect.x + rect.width / 2.0) * self.dpr;
            let cy = (rect.y + rect.height / 2.0) * self.dpr;
            let radius = rect.width.min(rect.height) * self.dpr * 0.28;
            let Some(path) = PathBuilder::from_circle(cx, cy, radius) else {
                return;
            };
            self.fill_path(&path, color);
        } else {
            let x = rect.x * self.dpr;
            let y = rect.y * self.dpr;
            let width = rect.width * self.dpr;
            let height = rect.height * self.dpr;
            let mut path = PathBuilder::new();
            path.move_to(x + width * 0.20, y + height * 0.55);
            path.line_to(x + width * 0.42, y + height * 0.78);
            path.line_to(x + width * 0.82, y + height * 0.25);
            path.line_to(x + width * 0.70, y + height * 0.12);
            path.line_to(x + width * 0.42, y + height * 0.55);
            path.line_to(x + width * 0.28, y + height * 0.40);
            path.close();
            if let Some(path) = path.finish() {
                self.fill_path(&path, color);
            }
        }
    }

    fn fill_path(&mut self, path: &tiny_skia::Path, color: Color) {
        let mut paint = Paint::default();
        paint.set_color(color.to_tiny_skia());
        self.pixmap
            .fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    fn fill(&mut self, rect: Rect, color: Color, clip: Option<Rect>) {
        let Some(rect) = clip.map_or(Some(rect), |clip| rect.intersection(clip)) else {
            return;
        };
        if rect.is_empty() || color.a == 0 {
            return;
        }
        let Some(device) = device_rect(rect, self.dpr) else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color(color.to_tiny_skia());
        self.pixmap
            .fill_rect(device, &paint, Transform::identity(), None);
    }

    fn rounded_mask(&self, rect: Rect, radii: CornerRadii) -> Option<tiny_skia::Mask> {
        let path = rounded_rect_path(rect, radii, self.dpr)?;
        let mut mask = tiny_skia::Mask::new(self.pixmap.width(), self.pixmap.height())?;
        mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        Some(mask)
    }

    fn fill_masked(&mut self, rect: Rect, color: Color, clip: Rect, mask: &tiny_skia::Mask) {
        let Some(rect) = rect.intersection(clip) else {
            return;
        };
        if rect.is_empty() || color.a == 0 {
            return;
        }
        let Some(device) = device_rect(rect, self.dpr) else {
            return;
        };
        let path = PathBuilder::from_rect(device);
        let mut paint = Paint::default();
        paint.set_color(color.to_tiny_skia());
        self.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            Some(mask),
        );
    }

    fn fill_rounded(&mut self, rect: Rect, color: Color, radii: CornerRadii, clip: Option<Rect>) {
        let Some(visible) = clip.map_or(Some(rect), |clip| rect.intersection(clip)) else {
            return;
        };
        if visible.is_empty() || color.a == 0 {
            return;
        }
        let Some(device_clip) = device_rect(visible, self.dpr) else {
            return;
        };
        let Some(path) = rounded_rect_path(rect, radii, self.dpr) else {
            return;
        };
        let mask = if visible == rect {
            None
        } else {
            let Some(mut mask) = tiny_skia::Mask::new(self.pixmap.width(), self.pixmap.height())
            else {
                return;
            };
            let clip_path = PathBuilder::from_rect(device_clip);
            mask.fill_path(&clip_path, FillRule::Winding, true, Transform::identity());
            Some(mask)
        };
        let mut paint = Paint::default();
        paint.set_color(color.to_tiny_skia());
        self.pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            mask.as_ref(),
        );
    }

    fn image(&mut self, image: &crate::image::DecodedImage, rect: Rect, opacity: f32) {
        if rect.is_empty() {
            return;
        }
        let paint = PixmapPaint {
            opacity: opacity.clamp(0.0, 1.0),
            blend_mode: BlendMode::SourceOver,
            quality: FilterQuality::Bilinear,
        };
        let x = (rect.x * self.dpr).round() as i32;
        let y = (rect.y * self.dpr).round() as i32;
        let width = (rect.width * self.dpr).round().max(1.0) as u32;
        let height = (rect.height * self.dpr).round().max(1.0) as u32;
        let Some(scaled) = image.resized(width, height) else {
            return;
        };
        self.pixmap
            .draw_pixmap(x, y, scaled.as_ref(), &paint, Transform::identity(), None);
    }

    fn borders(
        &mut self,
        rect: Rect,
        border: &crate::document::style::BorderEdges,
        color: Color,
        clip: Rect,
        radii: CornerRadii,
    ) {
        let mask = if radii.is_zero() {
            None
        } else {
            self.rounded_mask(rect, radii)
        };
        let edges = [
            (
                Rect::new(rect.x, rect.y, rect.width, border.top.width),
                border.top,
            ),
            (
                Rect::new(
                    rect.right() - border.right.width,
                    rect.y,
                    border.right.width,
                    rect.height,
                ),
                border.right,
            ),
            (
                Rect::new(
                    rect.x,
                    rect.bottom() - border.bottom.width,
                    rect.width,
                    border.bottom.width,
                ),
                border.bottom,
            ),
            (
                Rect::new(rect.x, rect.y, border.left.width, rect.height),
                border.left,
            ),
        ];
        for (edge, border) in edges {
            let color = border.color.unwrap_or(color);
            match border.style {
                BorderStyle::None => {}
                BorderStyle::Solid | BorderStyle::Double => {
                    if let Some(mask) = mask.as_ref() {
                        self.fill_masked(edge, color, clip, mask);
                    } else {
                        self.fill(edge, color, Some(clip));
                    }
                }
                BorderStyle::Dashed | BorderStyle::Dotted => {
                    self.dashed(edge, color, clip, border.style);
                }
            }
        }
    }

    fn dashed(&mut self, rect: Rect, color: Color, clip: Rect, style: BorderStyle) {
        let dash = if style == BorderStyle::Dotted {
            rect.height.max(rect.width).max(1.0) * 0.5
        } else {
            rect.height.max(rect.width).max(1.0) * 2.0
        };
        if rect.width > rect.height {
            let mut x = rect.x;
            while x < rect.right() {
                self.fill(
                    Rect::new(x, rect.y, dash.min(rect.right() - x), rect.height),
                    color,
                    Some(clip),
                );
                x += dash * 2.0;
            }
        } else {
            let mut y = rect.y;
            while y < rect.bottom() {
                self.fill(
                    Rect::new(rect.x, y, rect.width, dash.min(rect.bottom() - y)),
                    color,
                    Some(clip),
                );
                y += dash * 2.0;
            }
        }
    }

    fn line(&mut self, layout: &LayoutTree, line: &LineBox, opacity: f32, clip: Rect) {
        for item in &line.items {
            match item {
                LineItem::Text(fragment) => self.fragment(fragment, line.baseline, opacity, clip),
                LineItem::Atomic(fragment) => self.paint_box(layout, fragment.box_id, clip),
            }
        }
    }

    fn fragment(&mut self, fragment: &TextFragment, baseline: f32, opacity: f32, clip: Rect) {
        let Some(shaped) = &fragment.shaped else {
            return;
        };
        let color = with_opacity(fragment.style.color, opacity * fragment.style.opacity);
        let size = shaped.font_size * self.dpr;
        let mut pen_x = fragment.x;
        let clip_device = device_rect(clip, self.dpr);

        for segment in &shaped.segments {
            for glyph in &segment.glyphs {
                let x = pen_x + glyph.offset_x;
                let y = baseline - glyph.offset_y;
                if let Some(image) = self.rasterizer.render(&segment.face, glyph.id, size) {
                    let Some(clip_device) = clip_device else {
                        pen_x += glyph.advance;
                        continue;
                    };
                    let origin_x = (x * self.dpr).round() as i32 + image.left;
                    let origin_y = (y * self.dpr).round() as i32 - image.top;
                    self.blend_glyph(&image, origin_x, origin_y, color, clip_device);
                } else {
                    self.compat.unsupported("glyph rasterization".to_owned());
                }
                pen_x += glyph.advance;
            }
        }

        self.decorations(fragment, baseline, color, clip);
    }

    fn blend_glyph(
        &mut self,
        image: &crate::text::GlyphImage,
        origin_x: i32,
        origin_y: i32,
        color: Color,
        clip: tiny_skia::Rect,
    ) {
        let width = self.pixmap.width() as i32;
        let height = self.pixmap.height() as i32;
        let clip_left = clip.left().floor() as i32;
        let clip_top = clip.top().floor() as i32;
        let clip_right = clip.right().ceil() as i32;
        let clip_bottom = clip.bottom().ceil() as i32;
        let data = self.pixmap.data_mut();

        for row in 0..image.height as i32 {
            let y = origin_y + row;
            if y < 0 || y >= height || y < clip_top || y >= clip_bottom {
                continue;
            }
            for column in 0..image.width as i32 {
                let x = origin_x + column;
                if x < 0 || x >= width || x < clip_left || x >= clip_right {
                    continue;
                }
                let index = (row * image.width as i32 + column) as usize;
                if image.color {
                    let offset = index * 4;
                    if offset + 3 >= image.data.len() {
                        continue;
                    }
                    blend_rgba(
                        data,
                        (y * width + x) as usize,
                        &image.data[offset..offset + 4],
                    );
                } else {
                    let alpha = image.data.get(index).copied().unwrap_or(0);
                    if alpha == 0 {
                        continue;
                    }
                    blend_mask(data, (y * width + x) as usize, color, alpha);
                }
            }
        }
    }

    fn decorations(&mut self, fragment: &TextFragment, baseline: f32, color: Color, clip: Rect) {
        let decoration = fragment.style.text_decoration;
        if !decoration.underline && !decoration.line_through && !decoration.overline {
            return;
        }
        let thickness = (fragment.style.font_size * 0.06).max(1.0);
        if decoration.underline {
            self.fill(
                Rect::new(
                    fragment.x,
                    baseline + fragment.style.font_size * 0.1,
                    fragment.width,
                    thickness,
                ),
                color,
                Some(clip),
            );
        }
        if decoration.line_through {
            self.fill(
                Rect::new(
                    fragment.x,
                    baseline - fragment.style.font_size * 0.3,
                    fragment.width,
                    thickness,
                ),
                color,
                Some(clip),
            );
        }
        if decoration.overline {
            self.fill(
                Rect::new(
                    fragment.x,
                    baseline - fragment.style.font_size * 0.8,
                    fragment.width,
                    thickness,
                ),
                color,
                Some(clip),
            );
        }
    }
}

fn with_opacity(color: Color, opacity: f32) -> Color {
    let alpha = (color.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
    color.with_alpha(alpha)
}

fn rounded_rect_path(rect: Rect, radii: CornerRadii, dpr: f32) -> Option<tiny_skia::Path> {
    let x = rect.x * dpr;
    let y = rect.y * dpr;
    let width = rect.width * dpr;
    let height = rect.height * dpr;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let max_radius = width.min(height) / 2.0;
    let top_left = (radii.top_left * dpr).clamp(0.0, max_radius);
    let top_right = (radii.top_right * dpr).clamp(0.0, max_radius);
    let bottom_right = (radii.bottom_right * dpr).clamp(0.0, max_radius);
    let bottom_left = (radii.bottom_left * dpr).clamp(0.0, max_radius);
    let mut path = PathBuilder::new();
    path.move_to(x + top_left, y);
    path.line_to(x + width - top_right, y);
    path.quad_to(x + width, y, x + width, y + top_right);
    path.line_to(x + width, y + height - bottom_right);
    path.quad_to(x + width, y + height, x + width - bottom_right, y + height);
    path.line_to(x + bottom_left, y + height);
    path.quad_to(x, y + height, x, y + height - bottom_left);
    path.line_to(x, y + top_left);
    path.quad_to(x, y, x + top_left, y);
    path.close();
    path.finish()
}

fn device_rect(rect: Rect, dpr: f32) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_ltrb(
        rect.x * dpr,
        rect.y * dpr,
        rect.right() * dpr,
        rect.bottom() * dpr,
    )
}

fn blend_mask(data: &mut [u8], pixel: usize, color: Color, alpha: u8) {
    let index = pixel * 4;
    if index + 3 >= data.len() {
        return;
    }
    let source_alpha = alpha as f32 / 255.0;
    let inverse = 1.0 - source_alpha;
    let destination_alpha = data[index + 3] as f32 / 255.0;
    let output_alpha = source_alpha + destination_alpha * inverse;
    if output_alpha <= 0.0 {
        return;
    }
    let blend = |source: u8, destination: u8| -> u8 {
        let source = source as f32 * source_alpha;
        let destination = destination as f32 * inverse;
        ((source + destination) / output_alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    data[index] = blend(color.r, data[index]);
    data[index + 1] = blend(color.g, data[index + 1]);
    data[index + 2] = blend(color.b, data[index + 2]);
    data[index + 3] = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn blend_rgba(data: &mut [u8], pixel: usize, source: &[u8]) {
    let index = pixel * 4;
    if index + 3 >= data.len() {
        return;
    }
    let source_alpha = source[3] as f32 / 255.0;
    let inverse = 1.0 - source_alpha;
    let destination_alpha = data[index + 3] as f32 / 255.0;
    let output_alpha = source_alpha + destination_alpha * inverse;
    if output_alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let value = source[channel] as f32 + data[index + channel] as f32 * inverse;
        data[index + channel] = (value / output_alpha).round().clamp(0.0, 255.0) as u8;
    }
    data[index + 3] = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}
