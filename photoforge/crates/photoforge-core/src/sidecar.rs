//! Google Photos / Takeout JSON sidecar detection and parsing.
//!
//! Takeout writes a JSON file next to each image, named either
//! `<image>.json` or `<image>.supplemental-metadata.json`. We pull the capture
//! time (`photoTakenTime.timestamp`, Unix seconds) and location (`geoData`).

use crate::date;
use crate::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Metadata lifted from a Takeout sidecar.
#[derive(Debug, Default, Clone)]
pub struct Sidecar {
    /// `photoTakenTime`, formatted as ISO-8601 UTC.
    pub taken_time: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

#[derive(Deserialize)]
struct RawSidecar {
    #[serde(rename = "photoTakenTime")]
    photo_taken_time: Option<RawTimestamp>,
    #[serde(rename = "geoData")]
    geo_data: Option<RawGeo>,
}

#[derive(Deserialize)]
struct RawTimestamp {
    /// Unix seconds, as a string (Google's format).
    timestamp: Option<String>,
}

#[derive(Deserialize)]
struct RawGeo {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

/// Candidate sidecar paths for an image, in preference order.
fn candidates(image: &Path) -> [PathBuf; 2] {
    let base = image.as_os_str().to_string_lossy();
    [
        PathBuf::from(format!("{base}.supplemental-metadata.json")),
        PathBuf::from(format!("{base}.json")),
    ]
}

/// Find and parse the sidecar for `image`, if one exists. Malformed JSON is
/// ignored (returns `None`) rather than failing the file.
pub fn find_and_parse(image: &Path) -> Option<Sidecar> {
    candidates(image)
        .iter()
        .find(|p| p.is_file())
        .and_then(|p| parse_file(p).ok())
}

fn parse_file(path: &Path) -> Result<Sidecar> {
    let raw: RawSidecar = serde_json::from_str(&std::fs::read_to_string(path)?)?;

    let taken_time = raw
        .photo_taken_time
        .and_then(|t| t.timestamp)
        .and_then(|s| s.parse::<i64>().ok())
        .map(date::unix_to_iso);

    // Takeout uses 0.0/0.0 to mean "no location"; treat that as absent.
    let (lat, lon) = match raw.geo_data {
        Some(g) => {
            let la = g.latitude.unwrap_or(0.0);
            let lo = g.longitude.unwrap_or(0.0);
            if la == 0.0 && lo == 0.0 {
                (None, None)
            } else {
                (Some(la), Some(lo))
            }
        }
        None => (None, None),
    };

    Ok(Sidecar {
        taken_time,
        lat,
        lon,
    })
}
