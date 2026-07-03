//! Small, dependency-free date helpers: normalize EXIF timestamps, format Unix
//! timestamps as ISO-8601 UTC, and best-effort extract a date from a filename.

/// Convert a civil (Gregorian) day number since 1970-01-01 into `(year, month,
/// day)`. Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

/// Format a Unix timestamp (seconds) as `YYYY-MM-DDTHH:MM:SSZ` (UTC).
pub fn unix_to_iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Normalize an EXIF datetime (`"YYYY:MM:DD HH:MM:SS"`) into ISO-8601 without a
/// zone (`"YYYY-MM-DDTHH:MM:SS"`). Returns `None` for blank/zero timestamps.
pub fn normalize_exif_datetime(raw: &str) -> Option<String> {
    let (date, time) = raw.trim().split_once(' ')?;
    let parts: Vec<&str> = date.split([':', '-', '/']).collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i32 = parts[0].parse().ok()?;
    let mo: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    if y == 0 || mo == 0 || d == 0 || mo > 12 || d > 31 {
        return None;
    }
    Some(format!("{y:04}-{mo:02}-{d:02}T{}", time.trim()))
}

fn num(b: &[u8], start: usize, len: usize) -> Option<u32> {
    if start + len > b.len() {
        return None;
    }
    let mut v = 0u32;
    for &c in &b[start..start + len] {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    Some(v)
}

fn is_sep(b: u8) -> bool {
    matches!(b, b'-' | b'_' | b'.' | b'/')
}

/// Try to read `YYYY[sep]MM[sep]DD` (optionally followed by `HHMMSS`) starting
/// at byte `i`. Handles both compact (`20230115`) and separated (`2023-01-15`)
/// forms. Works on raw bytes so it never slices across a UTF-8 boundary.
fn try_date_at(b: &[u8], i: usize) -> Option<String> {
    let y = num(b, i, 4)?;
    if !(1970..=2099).contains(&y) {
        return None;
    }
    let mut j = i + 4;
    let sep = j < b.len() && is_sep(b[j]);
    if sep {
        j += 1;
    }
    let mo = num(b, j, 2)?;
    if !(1..=12).contains(&mo) {
        return None;
    }
    j += 2;
    if sep {
        if j < b.len() && is_sep(b[j]) {
            j += 1;
        } else {
            return None;
        }
    }
    let d = num(b, j, 2)?;
    if !(1..=31).contains(&d) {
        return None;
    }
    j += 2;

    // Optional time component after a single separator (space, T, _, -).
    let (mut hh, mut mm, mut ss) = (0u32, 0u32, 0u32);
    if j < b.len() && matches!(b[j], b' ' | b'T' | b't' | b'_' | b'-') {
        j += 1;
    }
    if let (Some(h), Some(m), Some(s)) = (num(b, j, 2), num(b, j + 2, 2), num(b, j + 4, 2)) {
        if h < 24 && m < 60 && s < 60 {
            hh = h;
            mm = m;
            ss = s;
        }
    }
    Some(format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}"))
}

/// Best-effort extraction of a capture date from a filename (e.g.
/// `IMG_20230115_120000` or `2023-01-15_vacation`). Returns ISO-8601 (no zone).
pub fn parse_date_from_filename(name: &str) -> Option<String> {
    let b = name.as_bytes();
    (0..b.len()).find_map(|i| try_date_at(b, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_epoch_and_known() {
        assert_eq!(unix_to_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_iso(1_673_784_000), "2023-01-15T12:00:00Z");
    }

    #[test]
    fn exif_normalization() {
        assert_eq!(
            normalize_exif_datetime("2024:01:15 10:30:00").as_deref(),
            Some("2024-01-15T10:30:00")
        );
        assert!(normalize_exif_datetime("0000:00:00 00:00:00").is_none());
    }

    #[test]
    fn filename_dates() {
        assert_eq!(
            parse_date_from_filename("IMG_20230115_120000").as_deref(),
            Some("2023-01-15T12:00:00")
        );
        assert_eq!(
            parse_date_from_filename("2023-01-15_vacation").as_deref(),
            Some("2023-01-15T00:00:00")
        );
        assert!(parse_date_from_filename("holiday-pic").is_none());
    }
}
