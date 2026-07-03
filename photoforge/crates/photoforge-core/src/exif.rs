//! EXIF metadata extraction via kamadak-exif.

use crate::Result;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Read the capture timestamp (`DateTimeOriginal`, falling back to `DateTime`)
/// from a file's EXIF, if present.
pub fn captured_at(path: impl AsRef<Path>) -> Result<Option<String>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let exif = match exif_reader.read_from_container(&mut reader) {
        Ok(e) => e,
        // A missing/unreadable EXIF block is not an error for our purposes.
        Err(exif::Error::NotFound(_)) | Err(exif::Error::BlankValue(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTime] {
        if let Some(field) = exif.get_field(tag, exif::In::PRIMARY) {
            return Ok(Some(field.display_value().to_string()));
        }
    }
    Ok(None)
}
