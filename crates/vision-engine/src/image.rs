//! Decoded image resources.

use std::collections::HashMap;
use std::sync::Arc;

use tiny_skia::{Pixmap, Transform};

use crate::snapshot::Snapshot;

/// A decoded raster image.
#[derive(Debug)]
pub(crate) struct DecodedImage {
    /// Intrinsic width in image pixels.
    pub(crate) width: u32,
    /// Intrinsic height in image pixels.
    pub(crate) height: u32,
    /// Premultiplied RGBA pixels.
    pub(crate) pixmap: Pixmap,
}

/// Images available to a render.
#[derive(Default)]
pub(crate) struct ImageStore {
    images: HashMap<String, Arc<DecodedImage>>,
}

impl ImageStore {
    /// Decodes every resource in a snapshot.
    pub(crate) fn new(snapshot: &Snapshot) -> Self {
        let mut images = HashMap::new();
        for resource in &snapshot.resources {
            let Some(decoded) = decode(&resource.url, &resource.bytes) else {
                continue;
            };
            images.insert(resource.url.clone(), Arc::new(decoded));
        }
        Self { images }
    }

    /// Returns an image by its original `src`.
    pub(crate) fn get(&self, url: &str) -> Option<Arc<DecodedImage>> {
        self.images.get(url).cloned()
    }
}

impl DecodedImage {
    /// Returns a nearest-neighbor scaled copy of the image.
    pub(crate) fn resized(&self, width: u32, height: u32) -> Option<Pixmap> {
        if width == 0 || height == 0 {
            return None;
        }
        if width == self.width && height == self.height {
            return Some(self.pixmap.clone());
        }
        let mut output = Pixmap::new(width, height)?;
        let source = self.pixmap.data();
        let destination = output.data_mut();
        for y in 0..height {
            let source_y = (u64::from(y) * u64::from(self.height) / u64::from(height)) as u32;
            for x in 0..width {
                let source_x = (u64::from(x) * u64::from(self.width) / u64::from(width)) as u32;
                let source_index = ((source_y * self.width + source_x) * 4) as usize;
                let destination_index = ((y * width + x) * 4) as usize;
                destination[destination_index..destination_index + 4]
                    .copy_from_slice(&source[source_index..source_index + 4]);
            }
        }
        Some(output)
    }
}

fn decode(url: &str, bytes: &[u8]) -> Option<DecodedImage> {
    let looks_like_svg = url.to_ascii_lowercase().ends_with(".svg")
        || std::str::from_utf8(bytes).is_ok_and(|text| {
            let text = text.trim_start_matches('\u{feff}').trim_start();
            text.starts_with("<svg") || text.starts_with("<?xml")
        });
    if looks_like_svg {
        if let Some(decoded) = decode_svg(bytes) {
            return Some(decoded);
        }
    }
    decode_raster(bytes)
}

fn decode_raster(bytes: &[u8]) -> Option<DecodedImage> {
    let dynamic = image::load_from_memory(bytes).ok()?;
    let rgba = dynamic.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut pixmap = Pixmap::new(width, height)?;
    let data = pixmap.data_mut();
    for (index, pixel) in rgba.pixels().enumerate() {
        let alpha = u16::from(pixel[3]);
        for channel in 0..3 {
            let value = u16::from(pixel[channel]);
            data[index * 4 + channel] = ((value * alpha + 127) / 255) as u8;
        }
        data[index * 4 + 3] = pixel[3];
    }
    Some(DecodedImage {
        width,
        height,
        pixmap,
    })
}

fn decode_svg(bytes: &[u8]) -> Option<DecodedImage> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let width = size.width().ceil() as u32;
    let height = size.height().ceil() as u32;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }
    let mut pixmap = Pixmap::new(width, height)?;
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
    Some(DecodedImage {
        width,
        height,
        pixmap,
    })
}
