//! # photoforge-core
//!
//! Backend-agnostic core for the photoforge photo organizer. This crate holds
//! all the real work — filesystem scanning, EXIF/sidecar extraction, date
//! resolution, thumbnailing, and the SQLite catalog — and deliberately has
//! **no dependency on Tauri**, so it can be unit-tested and reused headlessly.
//!
//! The headline entry point is [`index_directory`], a parallel, resumable
//! indexer that walks a tree, extracts metadata, and batch-writes it to SQLite.

pub mod classify;
pub mod date;
pub mod db;
pub mod dedupe;
pub mod dhash;
pub mod error;
pub mod exif;
pub mod hash;
pub mod indexer;
pub mod organize;
pub mod scan;
pub mod sidecar;
pub mod thumbnail;

pub use error::{Error, Result};
pub use indexer::{index_directory, index_directory_with_progress, IndexStats, IndexedFile};

/// Version string of the core library, sourced from Cargo at compile time.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
