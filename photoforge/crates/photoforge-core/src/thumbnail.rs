//! Thumbnail generation via the `image` crate.

use crate::Result;
use std::path::Path;

/// Decode an image and produce a JPEG thumbnail (bounded to `max_edge` on its
/// longest side, aspect preserved), returned as encoded bytes.
pub fn thumbnail(path: impl AsRef<Path>, max_edge: u32) -> Result<Vec<u8>> {
    let img = image::open(path)?;
    let thumb = img.thumbnail(max_edge, max_edge);

    let mut out = std::io::Cursor::new(Vec::new());
    thumb.write_to(&mut out, image::ImageFormat::Jpeg)?;
    Ok(out.into_inner())
}
