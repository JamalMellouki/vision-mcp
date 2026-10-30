//! Text shaping and glyph rasterization.

mod font;
mod shaping;

pub(crate) use font::FontBook;
pub(crate) use shaping::{GlyphImage, GlyphRasterizer, ShapedText};
