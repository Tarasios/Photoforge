//! # photoforge-core
//!
//! Backend-agnostic core for the photoforge photo organizer. This crate holds
//! all the real work — filesystem scanning, content hashing, EXIF extraction,
//! thumbnail generation, and the SQLite catalog — and deliberately has **no
//! dependency on Tauri**, so it can be unit-tested and reused headlessly.

pub mod db;
pub mod error;
pub mod exif;
pub mod hash;
pub mod scan;
pub mod thumbnail;

pub use error::{Error, Result};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Version string of the core library, sourced from Cargo at compile time.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A single photo as discovered on disk and enriched by the pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Size in bytes.
    pub size: u64,
    /// BLAKE3 content hash (hex), used for dedup and change detection.
    pub content_hash: String,
    /// Capture timestamp pulled from EXIF, if available (as an EXIF string).
    pub captured_at: Option<String>,
}
