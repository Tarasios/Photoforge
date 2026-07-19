//! Perceptual difference hashing (dHash).
//!
//! The idea: shrink the image to a tiny 9x8 grayscale thumbnail, then compare
//! each pixel to its right-hand neighbor. Each comparison yields one bit
//! ("is the image getting brighter left-to-right here?"), giving 8 rows x 8
//! comparisons = a 64-bit fingerprint. Gradients survive recompression,
//! resizing, and small edits far better than raw pixel values do, so two
//! visually identical images land within a few bits of each other (Hamming
//! distance), while unrelated images differ in ~32 bits on average.
//!
//! Why 9x8: we need 8 *comparisons* per row, and comparing adjacent pixels
//! needs one extra column — 9 pixels wide gives exactly 8 left/right pairs.

use crate::Result;
use image::imageops::FilterType;
use image::DynamicImage;
use std::path::Path;

/// Compute the 64-bit dHash of an already-decoded image.
///
/// The caller is responsible for applying EXIF orientation first (see
/// [`apply_orientation`]) so that a photo and its auto-rotated copy hash alike.
pub fn dhash_image(img: &DynamicImage) -> u64 {
    // Lanczos3 is the highest-quality resampler the `image` crate offers; at
    // this scale quality matters more than speed because every pixel of the
    // 9x8 thumbnail feeds a bit of the hash.
    let small = img.resize_exact(9, 8, FilterType::Lanczos3).to_luma8();
    let mut hash = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            let left = small.get_pixel(x, y)[0];
            let right = small.get_pixel(x + 1, y)[0];
            hash <<= 1;
            if left > right {
                hash |= 1;
            }
        }
    }
    hash
}

/// Re-orient a decoded image according to its EXIF `Orientation` tag (1-8).
///
/// Cameras often store the sensor data unrotated and record how the phone was
/// held; without this step, a photo and its baked-rotation copy would hash to
/// completely different values.
pub fn apply_orientation(img: DynamicImage, orientation: Option<i64>) -> DynamicImage {
    match orientation {
        Some(2) => img.fliph(),
        Some(3) => img.rotate180(),
        Some(4) => img.flipv(),
        Some(5) => img.rotate90().fliph(),
        Some(6) => img.rotate90(),
        Some(7) => img.rotate270().fliph(),
        Some(8) => img.rotate270(),
        _ => img, // 1, missing, or out of range: already upright
    }
}

/// Decode `path`, apply `orientation`, and compute its dHash.
pub fn dhash_file(path: impl AsRef<Path>, orientation: Option<i64>) -> Result<u64> {
    let img = image::open(path)?;
    Ok(dhash_image(&apply_orientation(img, orientation)))
}

/// Hamming distance between two dHashes: the number of differing bits.
///
/// `count_ones()` compiles down to a single POPCNT instruction on x86-64.
#[inline]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// A deterministic pseudo-photo with smooth gradients plus blocky detail,
    /// so the hash has real structure (a flat image would hash to 0).
    fn synthetic_photo(seed: u32) -> DynamicImage {
        let mut img = RgbImage::new(256, 256);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let v = ((x * 7 + y * 13 + seed * 31) % 256) as u8;
            let w = (((x / 32) * 50 + (y / 32) * 90 + seed * 17) % 256) as u8;
            *p = Rgb([v, w, v.wrapping_add(w)]);
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn identical_image_distance_zero() {
        let img = synthetic_photo(1);
        assert_eq!(hamming(dhash_image(&img), dhash_image(&img)), 0);
    }

    #[test]
    fn recompressed_jpeg_within_five_bits() {
        let img = synthetic_photo(2);
        // Round-trip through JPEG at quality 60 — lossy, but perceptually close.
        let mut buf = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60)
            .encode_image(&img)
            .unwrap();
        let recompressed = image::load_from_memory(buf.get_ref()).unwrap();
        let d = hamming(dhash_image(&img), dhash_image(&recompressed));
        assert!(d <= 5, "recompressed distance was {d}");
    }

    #[test]
    fn unrelated_images_far_apart() {
        // Different structure, not just a different seed: invert the gradient axes.
        let a = synthetic_photo(3);
        let mut img = RgbImage::new(256, 256);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let v = (255 - ((x * 3 + y * 29) % 256)) as u8;
            *p = Rgb([v, v / 2, 255 - v]);
        }
        let b = DynamicImage::ImageRgb8(img);
        let d = hamming(dhash_image(&a), dhash_image(&b));
        assert!(d > 20, "unrelated distance was only {d}");
    }

    #[test]
    fn orientation_normalizes_rotated_copies() {
        let img = synthetic_photo(4);
        // A camera that stored the pixels rotated 90° CCW plus Orientation=6
        // should hash the same as the upright original.
        let stored = img.rotate270();
        let upright = apply_orientation(stored, Some(6));
        assert_eq!(hamming(dhash_image(&img), dhash_image(&upright)), 0);
    }

    #[test]
    fn hamming_counts_bits() {
        assert_eq!(hamming(0, 0), 0);
        assert_eq!(hamming(0, u64::MAX), 64);
        assert_eq!(hamming(0b1011, 0b0010), 2);
    }
}
