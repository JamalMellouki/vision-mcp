//! Font discovery, selection, and script fallback.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fontdb::{Database, Family, Query, Stretch, Style as DbStyle, Weight};

use crate::document::style::{ComputedStyle, FontStyle, TextTransform};
use crate::error::{Error, Result};

use super::shaping::{SegmentMetrics, ShapedSegment, ShapedText, shape_segment};

/// A loaded font face.
#[derive(Debug)]
pub(crate) struct FontFace {
    /// Font file bytes.
    pub(crate) data: Arc<[u8]>,
    /// Face index in the file.
    pub(crate) index: u32,
    /// Primary family name.
    pub(crate) family: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Ltr,
    Rtl,
}

fn transform_text(text: &str, transform: TextTransform) -> Cow<'_, str> {
    match transform {
        TextTransform::None => Cow::Borrowed(text),
        TextTransform::Uppercase => Cow::Owned(text.to_uppercase()),
        TextTransform::Lowercase => Cow::Owned(text.to_lowercase()),
        TextTransform::Capitalize => {
            let mut result = String::with_capacity(text.len());
            let mut at_word_start = true;
            for character in text.chars() {
                if character.is_alphanumeric() {
                    if at_word_start {
                        result.extend(character.to_uppercase());
                    } else {
                        result.push(character);
                    }
                    at_word_start = false;
                } else {
                    result.push(character);
                    at_word_start = true;
                }
            }
            Cow::Owned(result)
        }
    }
}

/// A font database with a face cache.
pub(crate) struct FontBook {
    database: Database,
    cache: Mutex<HashMap<fontdb::ID, Arc<FontFace>>>,
}

impl FontBook {
    /// Loads the system fonts.
    pub(crate) fn new() -> Result<Self> {
        let mut database = Database::new();
        database.load_system_fonts();
        if database.faces().next().is_none() {
            return Err(Error::Unsupported(
                "no system fonts found; install a font package".to_owned(),
            ));
        }
        Ok(Self {
            database,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Shapes text using the best matching faces for each script run.
    pub(crate) fn shape(&self, text: &str, style: &ComputedStyle) -> Option<ShapedText> {
        if text.is_empty() {
            return None;
        }
        let text = transform_text(text, style.text_transform);
        let text = text.as_ref();
        let mut segments: Vec<ShapedSegment> = Vec::new();
        let mut metrics: Option<SegmentMetrics> = None;
        let mut width = 0.0;

        for (script, run) in script_runs(text) {
            let Some(id) = self.select_for_script(style, script) else {
                continue;
            };
            let Some(face) = self.face(id) else {
                continue;
            };
            let Some((segment, segment_metrics)) = shape_segment(&run, &face, style.font_size)
            else {
                continue;
            };
            width += segment
                .glyphs
                .iter()
                .map(|glyph| glyph.advance)
                .sum::<f32>();
            if metrics.is_none() {
                metrics = Some(segment_metrics);
            }
            segments.push(segment);
        }

        if style.letter_spacing != 0.0 {
            let mut glyph_count = 0_usize;
            for segment in &mut segments {
                glyph_count += segment.glyphs.len();
                for glyph in &mut segment.glyphs {
                    glyph.advance += style.letter_spacing;
                }
            }
            width += style.letter_spacing * glyph_count as f32;
        }

        let metrics = metrics?;
        Some(ShapedText {
            segments,
            width,
            ascent: metrics.ascent,
            descent: metrics.descent,
            line_gap: metrics.line_gap,
            font_size: style.font_size,
        })
    }

    fn select_for_script(&self, style: &ComputedStyle, script: Script) -> Option<fontdb::ID> {
        let query = |families: &[Family<'_>]| {
            self.database.query(&Query {
                families,
                weight: Weight(style.font_weight),
                stretch: Stretch::Normal,
                style: match style.font_style {
                    FontStyle::Normal => DbStyle::Normal,
                    FontStyle::Italic => DbStyle::Italic,
                    FontStyle::Oblique => DbStyle::Oblique,
                },
            })
        };

        let mut families = Vec::new();
        for family in &style.font_family {
            let lower = family.to_ascii_lowercase();
            match lower.as_str() {
                "serif" => {
                    families.push(Family::Name("DejaVu Serif"));
                    families.push(Family::Name("Liberation Serif"));
                    families.push(Family::Name("Noto Serif"));
                    families.push(Family::Serif);
                }
                "sans-serif" => {
                    if script == Script::Rtl {
                        families.push(Family::Name("Noto Naskh Arabic"));
                        families.push(Family::Name("Noto Sans Arabic"));
                        families.push(Family::Name("Scheherazade New"));
                    }
                    families.push(Family::Name("DejaVu Sans"));
                    families.push(Family::Name("Liberation Sans"));
                    families.push(Family::Name("Noto Sans"));
                    families.push(Family::SansSerif);
                }
                "monospace" => {
                    families.push(Family::Name("DejaVu Sans Mono"));
                    families.push(Family::Name("Liberation Mono"));
                    families.push(Family::Name("Noto Sans Mono"));
                    families.push(Family::Monospace);
                }
                "cursive" => {
                    families.push(Family::Name("DejaVu Sans"));
                    families.push(Family::Cursive);
                }
                "fantasy" => {
                    families.push(Family::Name("DejaVu Sans"));
                    families.push(Family::Fantasy);
                }
                _ => families.push(Family::Name(family)),
            }
        }

        if script == Script::Rtl {
            families.push(Family::Name("Noto Naskh Arabic"));
            families.push(Family::Name("Noto Sans Arabic"));
            families.push(Family::Name("Scheherazade New"));
            families.push(Family::Name("Amiri"));
        } else {
            families.push(Family::Name("DejaVu Sans"));
            families.push(Family::Name("Liberation Sans"));
            families.push(Family::Name("Noto Sans"));
        }
        families.push(Family::SansSerif);

        query(&families).or_else(|| self.database.faces().next().map(|face| face.id))
    }

    fn face(&self, id: fontdb::ID) -> Option<Arc<FontFace>> {
        if let Some(face) = self.cache.lock().ok()?.get(&id).cloned() {
            return Some(face);
        }
        let family = self
            .database
            .face(id)
            .and_then(|face| face.families.first().map(|family| family.0.clone()))
            .unwrap_or_default();
        let loaded = self.database.with_face_data(id, |data, index| {
            Arc::new(FontFace {
                data: Arc::from(data),
                index,
                family,
            })
        })?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(id, loaded.clone());
        }
        Some(loaded)
    }
}

fn script_runs(text: &str) -> Vec<(Script, String)> {
    let mut runs: Vec<(Script, String)> = Vec::new();
    let mut current = Script::Ltr;
    for ch in text.chars() {
        let script = if is_rtl(ch) {
            Script::Rtl
        } else if is_ltr_strong(ch) {
            Script::Ltr
        } else {
            current
        };
        if runs.is_empty() || script != current {
            current = script;
            runs.push((script, String::new()));
        }
        if let Some((_, run)) = runs.last_mut() {
            run.push(ch);
        }
    }
    runs
}

fn is_rtl(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0590..=0x05FF
            | 0x0600..=0x06FF
            | 0x0700..=0x074F
            | 0x0750..=0x077F
            | 0x0780..=0x07BF
            | 0x07C0..=0x07FF
            | 0x0800..=0x083F
            | 0x0840..=0x085F
            | 0x0860..=0x086F
            | 0x0870..=0x089F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
    )
}

fn is_ltr_strong(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || matches!(
            ch as u32,
            0x00C0..=0x024F | 0x0370..=0x03FF | 0x0400..=0x04FF | 0x2E80..=0x9FFF
        )
}
