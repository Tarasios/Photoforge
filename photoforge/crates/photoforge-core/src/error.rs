//! Error types shared across the core pipeline.

use thiserror::Error;

/// Convenience alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All the ways a core operation can fail.
#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("directory walk failed: {0}")]
    Walk(#[from] walkdir::Error),

    #[error("image decode/encode failed: {0}")]
    Image(#[from] image::ImageError),

    #[error("exif parse failed: {0}")]
    Exif(#[from] ::exif::Error),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("json sidecar parse failed: {0}")]
    Json(#[from] serde_json::Error),
}
