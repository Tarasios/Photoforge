//! Recursive filesystem enumeration of image files (walkdir).

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Image extensions the indexer recognizes (lowercased comparison).
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "heic", "webp", "bmp", "gif"];

/// Whether `path` has one of the recognized image extensions.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk `root` recursively, returning the path of every image file found.
/// Unreadable entries are silently skipped (the indexer surfaces per-file
/// errors separately when it tries to process them).
pub fn enumerate_images(root: impl AsRef<Path>) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| is_image(p))
        .collect()
}
