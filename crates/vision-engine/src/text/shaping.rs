//! Shaping and glyph rasterization.

use std::sync::Arc;

use rustybuzz::UnicodeBuffer;
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;

use super::font::FontFace;

/// A positioned glyph.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Glyph {
    /// Glyph identifier.
    pub(crate) id: u16,
    /// Horizontal advance.
    pub(crate) advance: f32,
    /// Horizontal offset.
    pub(crate) offset_x: f32,
    /// Vertical offset.
    pub(crate) offset_y: f32,
}

/// A shaped run that uses one font face.
#[derive(Clone, Debug)]
pub(crate) struct ShapedSegment {
    /// The face used for shaping and rasterization.
    pub(crate) face: Arc<FontFace>,
    /// Shaped glyphs in visual order.
    pub(crate) glyphs: Vec<Glyph>,
}

/// Metrics reported by a shaped segment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SegmentMetrics {
    /// Distance from the baseline to the top of the em box.
    pub(crate) ascent: f32,
    /// Distance from the baseline to the bottom of the em box.
    pub(crate) descent: f32,
    /// Recommended extra spacing between lines.
    pub(crate) line_gap: f32,
}

/// A shaped text run, possibly spanning several fonts.
#[derive(Clone, Debug)]
pub(crate) struct ShapedText {
    /// Shaped segments in visual order.
    pub(crate) segments: Vec<ShapedSegment>,
    /// Total advance width.
    pub(crate) width: f32,
    /// Distance from the baseline to the top of the em box.
    pub(crate) ascent: f32,
    /// Distance from the baseline to the bottom of the em box.
    pub(crate) descent: f32,
    /// Recommended extra spacing between lines.
    pub(crate) line_gap: f32,
    /// Font size used for shaping.
    pub(crate) font_size: f32,
}

/// Shapes a text run with one font face.
pub(crate) fn shape_segment(
    text: &str,
    face: &FontFace,
    size: f32,
) -> Option<(ShapedSegment, SegmentMetrics)> {
    if text.is_empty() || size <= 0.0 {
        return None;
    }
    let font = rustybuzz::Face::from_slice(&face.data, face.index)?;
    let units = font.units_per_em() as f32;
    if units <= 0.0 {
        return None;
    }
    let scale = size / units;
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(&font, &[], buffer);
    let mut glyphs = Vec::with_capacity(shaped.len());
    for (info, position) in shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions().iter())
    {
        glyphs.push(Glyph {
            id: info.glyph_id as u16,
            advance: position.x_advance as f32 * scale,
            offset_x: position.x_offset as f32 * scale,
            offset_y: position.y_offset as f32 * scale,
        });
    }
    let metrics = SegmentMetrics {
        ascent: font.ascender() as f32 * scale,
        descent: -font.descender() as f32 * scale,
        line_gap: font.line_gap() as f32 * scale,
    };
    Some((
        ShapedSegment {
            face: Arc::new(FontFace {
                data: Arc::clone(&face.data),
                index: face.index,
                family: face.family.clone(),
            }),
            glyphs,
        },
        metrics,
    ))
}

/// A rasterized glyph mask.
#[derive(Clone, Debug)]
pub(crate) struct GlyphImage {
    /// Horizontal offset from the pen position.
    pub(crate) left: i32,
    /// Vertical offset from the baseline.
    pub(crate) top: i32,
    /// Mask width.
    pub(crate) width: u32,
    /// Mask height.
    pub(crate) height: u32,
    /// Alpha or RGBA data.
    pub(crate) data: Vec<u8>,
    /// True when the data is RGBA rather than an alpha mask.
    pub(crate) color: bool,
}

/// Reusable glyph rasterizer.
pub(crate) struct GlyphRasterizer {
    context: ScaleContext,
}

impl GlyphRasterizer {
    /// Creates a rasterizer with its own scale context.
    pub(crate) fn new() -> Self {
        Self {
            context: ScaleContext::new(),
        }
    }

    /// Rasterizes a glyph at the given size.
    pub(crate) fn render(
        &mut self,
        face: &FontFace,
        glyph_id: u16,
        size: f32,
    ) -> Option<GlyphImage> {
        if size <= 0.0 {
            return None;
        }
        let font = swash::FontRef::from_index(&face.data, face.index as usize)?;
        let mut scaler = self.context.builder(font).size(size).hint(false).build();
        let mut render = Render::new(&[Source::Outline]);
        render.format(Format::Alpha);
        let image = render.render(&mut scaler, glyph_id)?;
        Some(GlyphImage {
            left: image.placement.left,
            top: image.placement.top,
            width: image.placement.width,
            height: image.placement.height,
            data: image.data,
            color: image.content == Content::Color,
        })
    }
}
