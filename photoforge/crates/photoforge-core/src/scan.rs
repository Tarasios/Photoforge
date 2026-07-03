//! Recursive filesystem scanning, parallelized with rayon.

use crate::{hash, Photo, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// File extensions we treat as photos.
const PHOTO_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "tif", "tiff", "webp", "heic", "bmp"];

fn is_photo(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| PHOTO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk `root` recursively and return the paths of every photo file found.
pub fn find_photos(root: impl AsRef<Path>) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| is_photo(p))
        .collect()
}

/// Build [`Photo`] records for a directory tree, hashing and reading EXIF for
/// each file in parallel across the rayon thread pool.
pub fn scan(root: impl AsRef<Path>) -> Vec<Result<Photo>> {
    find_photos(root)
        .into_par_iter()
        .map(|path| {
            let size = std::fs::metadata(&path)?.len();
            let content_hash = hash::hash_file(&path)?;
            let captured_at = crate::exif::captured_at(&path).ok().flatten();
            Ok(Photo {
                path,
                size,
                content_hash,
                captured_at,
            })
        })
        .collect()
}
