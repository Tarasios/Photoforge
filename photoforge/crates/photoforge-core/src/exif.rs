//! EXIF metadata extraction via kamadak-exif.
//!
//! Failing to read EXIF is never fatal here — files without EXIF (PNG/BMP/GIF,
//! or stripped JPEGs) simply yield an empty [`ExifData`].

use crate::date;
use crate::Result;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// The subset of EXIF fields photoforge cares about, already normalized.
#[derive(Debug, Default, Clone)]
pub struct ExifData {
    /// `DateTimeOriginal`, normalized to ISO-8601 (no zone).
    pub datetime_original: Option<String>,
    /// `CreateDate` (EXIF `DateTimeDigitized`), normalized to ISO-8601.
    pub create_date: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    /// Decimal degrees, sign applied from the N/S and E/W references.
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub orientation: Option<i64>,
    /// `PixelXDimension` / `PixelYDimension`, used as a dimension fallback for
    /// formats the `image` crate can't decode (e.g. HEIC).
    pub pixel_width: Option<i64>,
    pub pixel_height: Option<i64>,
}

fn ascii(field: &exif::Field) -> Option<String> {
    match &field.value {
        exif::Value::Ascii(vals) => {
            let s = String::from_utf8_lossy(vals.first()?)
                .trim_matches(|c: char| c == '\0' || c.is_whitespace())
                .to_string();
            (!s.is_empty()).then_some(s)
        }
        _ => None,
    }
}

fn short(field: &exif::Field) -> Option<i64> {
    match &field.value {
        exif::Value::Short(v) => v.first().map(|x| *x as i64),
        exif::Value::Long(v) => v.first().map(|x| *x as i64),
        _ => None,
    }
}

/// Convert a GPS coordinate (3 rationals: deg, min, sec) plus its N/S/E/W
/// reference into signed decimal degrees.
fn gps_coord(coord: &exif::Field, reference: &exif::Field) -> Option<f64> {
    let exif::Value::Rational(parts) = &coord.value else {
        return None;
    };
    if parts.len() < 3 {
        return None;
    }
    let deg = parts[0].to_f64() + parts[1].to_f64() / 60.0 + parts[2].to_f64() / 3600.0;
    let r = ascii(reference).unwrap_or_default();
    let sign = if r.eq_ignore_ascii_case("S") || r.eq_ignore_ascii_case("W") {
        -1.0
    } else {
        1.0
    };
    Some(deg * sign)
}

/// Read the wanted EXIF fields from `path`. Returns an empty [`ExifData`] when
/// the file has no readable EXIF block.
pub fn extract(path: impl AsRef<Path>) -> Result<ExifData> {
    let mut reader = BufReader::new(File::open(path)?);
    let exif = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(e) => e,
        // No/undecodable EXIF is not an error for our purposes.
        Err(_) => return Ok(ExifData::default()),
    };

    use exif::{In, Tag};
    let mut d = ExifData::default();
    let get = |tag| exif.get_field(tag, In::PRIMARY);

    d.datetime_original = get(Tag::DateTimeOriginal)
        .and_then(ascii)
        .and_then(|s| date::normalize_exif_datetime(&s));
    d.create_date = get(Tag::DateTimeDigitized)
        .and_then(ascii)
        .and_then(|s| date::normalize_exif_datetime(&s));
    d.make = get(Tag::Make).and_then(ascii);
    d.model = get(Tag::Model).and_then(ascii);
    d.orientation = get(Tag::Orientation).and_then(short);
    d.pixel_width = get(Tag::PixelXDimension).and_then(short);
    d.pixel_height = get(Tag::PixelYDimension).and_then(short);

    if let (Some(lat), Some(lat_ref)) = (get(Tag::GPSLatitude), get(Tag::GPSLatitudeRef)) {
        d.gps_lat = gps_coord(lat, lat_ref);
    }
    if let (Some(lon), Some(lon_ref)) = (get(Tag::GPSLongitude), get(Tag::GPSLongitudeRef)) {
        d.gps_lon = gps_coord(lon, lon_ref);
    }

    Ok(d)
}
