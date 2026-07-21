//! Recursive filesystem enumeration of image files (walkdir), with skip rules.
//!
//! Two layers of skipping:
//!   * **Built-in system folders** ([`BUILTIN_SKIP_DIRS`]) — Windows/system and
//!     tooling directories that never contain user photos. Matched by directory
//!     *name*, case-insensitively, anywhere in the tree.
//!   * **User skip paths** — absolute folder paths stored in the `skip_dirs`
//!     table; the walk prunes any directory equal to or under one of them.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Image extensions the indexer recognizes (lowercased comparison).
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "heic", "webp", "bmp", "gif"];

/// Directory *names* that are always pruned, wherever they appear.
/// Case-insensitive. These are system/tooling folders that never hold user
/// photos but can be enormous (or access-denied minefields).
pub const BUILTIN_SKIP_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "node_modules",
    ".git",
    "windowsapps",
];

/// Whether `path` has one of the recognized image extensions.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Whether a directory entry should be pruned by the built-in name rules.
fn is_builtin_skip(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .map(|n| BUILTIN_SKIP_DIRS.contains(&n.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Case-insensitive "is `path` equal to or inside `root`" for user skips.
/// Windows paths are case-insensitive, so `C:\Foo` must match `c:\foo\bar`.
fn is_under(path: &Path, root: &str) -> bool {
    let p = path.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    let r = root.to_ascii_lowercase().replace('/', "\\");
    let r = r.trim_end_matches('\\');
    p == r || p.starts_with(&format!("{r}\\"))
}

/// Walk `root` recursively, returning the path of every image file found.
/// Prunes built-in system folders and every folder in `user_skips` (absolute
/// paths). Unreadable entries are silently skipped — the indexer surfaces
/// per-file errors separately when it tries to process them.
pub fn enumerate_images(root: impl AsRef<Path>, user_skips: &[String]) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        // filter_entry prunes whole subtrees: returning false for a directory
        // means walkdir never descends into it.
        .filter_entry(|e| {
            if !e.file_type().is_dir() {
                return true;
            }
            if e.depth() > 0 && is_builtin_skip(e.file_name()) {
                return false;
            }
            !user_skips.iter().any(|s| is_under(e.path(), s))
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| is_image(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn skips_builtin_and_user_dirs() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pf_scan_{}_{nanos}", std::process::id()));
        for sub in ["ok", "node_modules", "$RECYCLE.BIN", "private"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
            image::RgbImage::new(2, 2)
                .save(dir.join(sub).join("img.png"))
                .unwrap();
        }

        let all = enumerate_images(&dir, &[]);
        let names: Vec<String> = all
            .iter()
            .map(|p| p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"ok".to_string()));
        assert!(names.contains(&"private".to_string()));
        assert!(!names.contains(&"node_modules".to_string()), "builtin skip");
        assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("$recycle.bin")));

        // User skip prunes 'private' (case-insensitively).
        let skip = dir.join("PRIVATE").to_string_lossy().into_owned();
        let filtered = enumerate_images(&dir, &[skip]);
        assert!(filtered
            .iter()
            .all(|p| !p.to_string_lossy().to_ascii_lowercase().contains("private")));

        let _ = fs::remove_dir_all(&dir);
    }
}
